#![allow(
    clippy::result_unit_err,
    reason = "pool rejection uses unit error; callers map to HTTP status"
)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use dali2rust_bus::{BusFrame, BusSubscriberRx, DEFAULT_CONFIRMATION_SLOTS};
use dali2rust_contracts::msg::ConfirmationEnvelope;

const SLOT_COUNT: usize = DEFAULT_CONFIRMATION_SLOTS;

const UNMATCHED_LOG_EVERY: u32 = 256;

pub(crate) const FORMATTER_FAILED_BODY: &[u8] =
    br#"{"success":false,"error":"formatter_failed","error_code":"formatter_failed"}"#;

const BRIDGE_STACK_SIZE: usize = 8 * 1024;

pub type ReplyFormatter = fn(&ConfirmationEnvelope) -> Option<Vec<u8>>;

struct ConfirmationWaiter {
    state: Mutex<WaiterState>,
    ready: Condvar,
}

#[derive(Default)]
struct WaiterState {
    generation: u32,
    body: Option<Vec<u8>>,
}

impl ConfirmationWaiter {
    fn new() -> Self {
        Self {
            state: Mutex::new(WaiterState::default()),
            ready: Condvar::new(),
        }
    }

    fn begin(&self) -> Option<u32> {
        let mut st = self.state.lock().ok()?;
        st.generation = st.generation.wrapping_add(1);
        st.body = None;
        Some(st.generation)
    }

    fn deliver(&self, generation: u32, body: Vec<u8>) -> bool {
        let Ok(mut st) = self.state.lock() else {
            return false;
        };
        if st.generation != generation {
            return false;
        }
        st.body = Some(body);
        self.ready.notify_all();
        true
    }

    fn take(&self, generation: u32) -> Option<Vec<u8>> {
        let mut st = self.state.lock().ok()?;
        if st.generation != generation {
            return None;
        }
        st.body.take()
    }

    fn wait(&self, generation: u32, timeout: Duration) -> Option<Vec<u8>> {
        let st = self.state.lock().ok()?;
        let (mut st, _) = self
            .ready
            .wait_timeout_while(st, timeout, |s| {
                s.generation == generation && s.body.is_none()
            })
            .ok()?;
        if st.generation != generation {
            return None;
        }
        st.body.take()
    }
}

pub struct ConfirmationHandle {
    waiter: Arc<ConfirmationWaiter>,
    generation: u32,
}

impl ConfirmationHandle {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Vec<u8>, RecvTimeoutError> {
        self.waiter
            .wait(self.generation, timeout)
            .ok_or(RecvTimeoutError::Timeout)
    }

    pub fn try_recv(&self) -> Result<Vec<u8>, TryRecvError> {
        self.waiter
            .take(self.generation)
            .ok_or(TryRecvError::Empty)
    }
}

struct Slot {
    correlation_id: u64,
    reply_formatter: ReplyFormatter,
    generation: u32,
}

pub struct PendingConfirmationSlots {
    slots: Mutex<Vec<Option<Slot>>>,
    waiters: Vec<Arc<ConfirmationWaiter>>,
    unmatched: AtomicU32,
    confirmation_timeouts: AtomicU32,
}

impl Default for PendingConfirmationSlots {
    fn default() -> Self {
        Self::with_capacity(SLOT_COUNT)
    }
}

impl PendingConfirmationSlots {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            slots: Mutex::new((0..capacity).map(|_| None).collect()),
            waiters: (0..capacity)
                .map(|_| Arc::new(ConfirmationWaiter::new()))
                .collect(),
            unmatched: AtomicU32::new(0),
            confirmation_timeouts: AtomicU32::new(0),
        }
    }

    pub fn unmatched_confirmations_load(&self) -> u32 {
        self.unmatched.load(Ordering::Relaxed)
    }

    pub fn confirmation_timeouts_load(&self) -> u32 {
        self.confirmation_timeouts.load(Ordering::Relaxed)
    }

    pub(crate) fn note_confirmation_timeout(&self) {
        self.confirmation_timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn try_register(
        &self,
        correlation_id: u64,
        reply_formatter: ReplyFormatter,
    ) -> Result<ConfirmationHandle, ()> {
        let mut g = self.slots.lock().map_err(|_| ())?;
        if g.iter()
            .flatten()
            .any(|slot| slot.correlation_id == correlation_id)
        {
            return Err(());
        }
        let idx = g.iter().position(Option::is_none).ok_or(())?;
        let waiter = Arc::clone(&self.waiters[idx]);
        let generation = waiter.begin().ok_or(())?;
        g[idx] = Some(Slot {
            correlation_id,
            reply_formatter,
            generation,
        });
        Ok(ConfirmationHandle { waiter, generation })
    }

    pub fn cancel(&self, correlation_id: u64) {
        if let Ok(mut g) = self.slots.lock() {
            for slot in g.iter_mut() {
                if let Some(s) = slot {
                    if s.correlation_id == correlation_id {
                        *slot = None;
                        break;
                    }
                }
            }
        }
    }

    pub fn dispatch_confirmation(&self, frame: &BusFrame) -> bool {
        let BusFrame::Confirmation(ce) = frame else {
            return false;
        };
        let correlation_id = ce.meta.correlation_id;

        let mut g = match self.slots.lock() {
            Ok(x) => x,
            Err(_) => return false,
        };
        for idx in 0..g.len() {
            let Some(s) = g[idx].as_ref() else {
                continue;
            };
            if s.correlation_id != correlation_id {
                continue;
            }

            let s = g[idx].take().expect("slot checked above");
            drop(g);
            let body = (s.reply_formatter)(ce.as_ref()).unwrap_or_else(|| {
                log::error!(
                    "confirmation_bridge: reply formatter returned None for correlation_id {correlation_id}"
                );
                FORMATTER_FAILED_BODY.to_vec()
            });
            return self.waiters[idx].deliver(s.generation, body);
        }
        let total = self.unmatched.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if total == 1 || total % UNMATCHED_LOG_EVERY == 0 {
            log::debug!(
                "confirmation_bridge: unmatched confirmation (correlation_id {correlation_id}, total {total})"
            );
        }
        false
    }
}

pub fn spawn_confirmation_bridge(
    rx: BusSubscriberRx,
    slots: Arc<PendingConfirmationSlots>,
) -> std::thread::JoinHandle<()> {
    dali2rust_bsp::esp_thread::spawn_named_stack_in(
        c"confirm-bridge",
        BRIDGE_STACK_SIZE,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        move || {
            while let Ok(frame) = dali2rust_bus::Receiver::recv(&rx) {
                slots.dispatch_confirmation(&frame);
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
    use dali2rust_bus::BusFrame;
    use dali2rust_contracts::bus::build_confirmation_envelope;
    use dali2rust_contracts::msg::DeliveryStatus;

    fn passthrough(ce: &dali2rust_contracts::msg::ConfirmationEnvelope) -> Option<Vec<u8>> {
        serde_json::to_vec(ce).ok()
    }

    fn no_body(_ce: &dali2rust_contracts::msg::ConfirmationEnvelope) -> Option<Vec<u8>> {
        None
    }

    fn confirmation_frame(correlation_id: u64) -> BusFrame {
        BusFrame::confirmation(build_confirmation_envelope(
            correlation_id,
            DeliveryStatus::Ok,
            0,
            SOURCE_ID_UNSPECIFIED,
        ))
    }

    #[test]
    fn duplicate_correlation_registration_is_rejected() {
        let slots = PendingConfirmationSlots::with_capacity(2);

        assert!(slots.try_register(42, passthrough).is_ok());
        assert!(slots.try_register(42, passthrough).is_err());
    }

    #[test]
    fn formatter_failure_returns_formatter_failed_body() {
        let slots = PendingConfirmationSlots::with_capacity(1);
        let rx = slots.try_register(42, no_body).expect("register");

        assert!(slots.dispatch_confirmation(&confirmation_frame(42)));
        let body = rx.try_recv().expect("formatter failed body");
        let text = std::str::from_utf8(&body).expect("utf-8 body");
        assert!(text.contains("\"success\":false"));
        assert!(text.contains("formatter_failed"));

        assert!(slots.try_register(43, passthrough).is_ok());
    }

    #[test]
    fn abandoned_handle_releases_slot() {
        let slots = PendingConfirmationSlots::with_capacity(1);
        let rx = slots.try_register(42, passthrough).expect("register");
        drop(rx);

        assert!(slots.dispatch_confirmation(&confirmation_frame(42)));
        assert!(slots.try_register(43, passthrough).is_ok());
        assert_eq!(slots.unmatched_confirmations_load(), 0);
    }

    #[test]
    fn reused_slot_does_not_leak_body_to_the_previous_handle() {
        let slots = PendingConfirmationSlots::with_capacity(1);
        let stale = slots.try_register(42, passthrough).expect("register");

        slots.cancel(42);
        let fresh = slots.try_register(43, passthrough).expect("reuse slot");

        assert!(slots.dispatch_confirmation(&confirmation_frame(43)));
        assert!(stale.try_recv().is_err());
        assert!(fresh.try_recv().is_ok());
    }

    #[test]
    fn late_confirmation_for_cancelled_request_is_unmatched() {
        let slots = PendingConfirmationSlots::with_capacity(1);
        let abandoned = slots.try_register(42, passthrough).expect("register");
        slots.cancel(42);

        assert!(!slots.dispatch_confirmation(&confirmation_frame(42)));
        assert!(abandoned.try_recv().is_err());
        assert_eq!(slots.unmatched_confirmations_load(), 1);
    }

    #[test]
    fn recv_timeout_returns_the_delivered_body() {
        let slots = Arc::new(PendingConfirmationSlots::with_capacity(2));
        let handle = slots.try_register(7, passthrough).expect("register");

        let writer = Arc::clone(&slots);
        let t = std::thread::spawn(move || {
            writer.dispatch_confirmation(&confirmation_frame(7))
        });
        let body = handle
            .recv_timeout(Duration::from_secs(2))
            .expect("confirmation body");
        assert!(t.join().expect("writer thread"));
        assert!(!body.is_empty());
    }

    #[test]
    fn recv_timeout_times_out_without_a_confirmation() {
        let slots = PendingConfirmationSlots::with_capacity(1);
        let handle = slots.try_register(9, passthrough).expect("register");

        assert!(handle.recv_timeout(Duration::from_millis(20)).is_err());
    }

    #[test]
    fn unmatched_confirmation_bumps_the_counter() {
        let slots = PendingConfirmationSlots::with_capacity(2);
        let _rx = slots.try_register(42, passthrough).expect("register");

        assert!(!slots.dispatch_confirmation(&confirmation_frame(7)));
        assert!(!slots.dispatch_confirmation(&confirmation_frame(8)));
        assert_eq!(slots.unmatched_confirmations_load(), 2);

        assert!(slots.dispatch_confirmation(&confirmation_frame(42)));
        assert_eq!(slots.unmatched_confirmations_load(), 2);
    }
}
