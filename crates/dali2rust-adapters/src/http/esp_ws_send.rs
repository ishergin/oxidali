use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dali2rust_ws_runtime::{
    HandoffOutcome, SendSlot, SlotPool, WsSink, WsSinkError, SENDER_BATCH_FRAMES,
};
use esp_idf_svc::http::server::ws::EspHttpWsDetachedSender;
use esp_idf_svc::sys::{
    httpd_handle_t, httpd_queue_work, httpd_ws_frame_t, httpd_ws_send_frame_async, httpd_ws_type_t,
    httpd_ws_type_t_HTTPD_WS_TYPE_CLOSE, httpd_ws_type_t_HTTPD_WS_TYPE_TEXT, ESP_OK,
};

const SOCKET_SEND_TIMEOUT_S: u64 = 5;

const SENDS_PER_FRAME: u64 = 2;

const SEND_DEADLINE: Duration =
    Duration::from_secs(SOCKET_SEND_TIMEOUT_S * SENDS_PER_FRAME * SENDER_BATCH_FRAMES as u64);

pub type WsSlotPool = SlotPool<SendWork>;

pub fn slot_pool(slots: usize) -> WsSlotPool {
    SlotPool::new(slots, SEND_DEADLINE)
}

pub struct SendWork {
    frames: Vec<Arc<str>>,
    kind: httpd_ws_type_t,
    server: httpd_handle_t,
    fd: i32,
    session: EspHttpWsDetachedSender,
}

// SAFETY: `server` is usable from any task via `httpd_queue_work`; the detached sender is `Send`.
unsafe impl Send for SendWork {}

pub struct DirectWsSink {
    slot: &'static SendSlot<SendWork>,
    server: httpd_handle_t,
    fd: i32,
    closed: AtomicBool,
    session: EspHttpWsDetachedSender,
}

// SAFETY: `server` is usable from any task via `httpd_queue_work`; the sender is `Send`.
unsafe impl Send for DirectWsSink {}
unsafe impl Sync for DirectWsSink {}

impl DirectWsSink {
    pub fn claim(
        pool: &'static WsSlotPool,
        server: httpd_handle_t,
        fd: i32,
        session: EspHttpWsDetachedSender,
    ) -> Option<Self> {
        Some(Self {
            slot: pool.claim()?,
            server,
            fd,
            closed: AtomicBool::new(false),
            session,
        })
    }


    fn send(&self, frames: &[Arc<str>], kind: httpd_ws_type_t) -> Result<(), WsSinkError> {
        if self.closed.load(Ordering::Acquire) || self.session.is_closed() {
            self.closed.store(true, Ordering::Release);
            return Err(WsSinkError::Closed);
        }
        let slot = self.slot;
        let work = || SendWork {
            frames: frames.to_vec(),
            kind,
            server: self.server,
            fd: self.fd,
            session: self.session.clone(),
        };
        self.judge(slot.hand_over(work, || self.queue(slot)))
    }

    fn queue(&self, slot: &'static SendSlot<SendWork>) -> bool {
        let arg = (slot as *const SendSlot<SendWork>).cast_mut().cast();
        // SAFETY: the slot lives in the pool leaked at boot, so it outlives the work item.
        let queued = unsafe { httpd_queue_work(self.server, Some(send_on_httpd_task), arg) };
        queued == ESP_OK
    }

    fn judge(&self, outcome: HandoffOutcome) -> Result<(), WsSinkError> {
        if outcome == HandoffOutcome::Done {
            return Ok(());
        }
        self.closed.store(true, Ordering::Release);
        if outcome == HandoffOutcome::Lost {
            log::warn!("ws: fd {} waited out its queued send; dropping it", self.fd);
        }
        Err(WsSinkError::Closed)
    }
}

impl Drop for DirectWsSink {
    fn drop(&mut self) {
        self.slot.release();
    }
}

impl WsSink for DirectWsSink {
    fn send_batch(&self, frames: &[Arc<str>]) -> Result<(), WsSinkError> {
        self.send(frames, httpd_ws_type_t_HTTPD_WS_TYPE_TEXT)
    }

    fn close(&self) {
        let _ = self.send(&[], httpd_ws_type_t_HTTPD_WS_TYPE_CLOSE);
    }
}

unsafe extern "C" fn send_on_httpd_task(arg: *mut core::ffi::c_void) {
    // SAFETY: `arg` is the slot `queue` handed to esp-idf; it lives in the pool leaked at boot.
    let slot = unsafe { &*arg.cast::<SendSlot<SendWork>>() };
    slot.execute(send_work);
}

fn send_work(work: SendWork) -> bool {
    if work.session.is_closed() {
        return false;
    }
    if work.frames.is_empty() {
        return send_one(&work, "");
    }
    work.frames.iter().all(|frame| send_one(&work, frame))
}

fn send_one(work: &SendWork, frame: &str) -> bool {
    let mut raw = httpd_ws_frame_t {
        final_: true,
        fragmented: false,
        type_: work.kind,
        payload: frame.as_ptr().cast_mut(),
        len: frame.len(),
    };
    // SAFETY: the handle and socket the work names; `frame` lives for the call.
    unsafe { httpd_ws_send_frame_async(work.server, work.fd, &mut raw) == ESP_OK }
}
