use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bus::{BusChannel, BusFrame, BusPublisher, PublishResult};
use dali2rust_contracts::bus::{build_confirmation_envelope, command_envelope, event_envelope};
use dali2rust_contracts::msg::{
    BusCommandPayload, BusEventPayload, CommandEnvelope, ConfirmationEnvelope,
    DaliCommandPayload, DaliEventPayload, DeliveryStatus, EventEnvelope, Origin,
};

use crate::sync::{wait_until, SHORT_POLL};

pub fn command_frame() -> BusFrame {
    BusFrame::command(command_envelope(
        0,
        1,
        0,
        None,
        DaliCommandPayload {
            wire_address: 2,
            command: 5,
            repeat_count: 1,
            raw_mode: false,
            raw_expects_backward: false,
        },
    ))
}

pub fn confirmation_frame() -> BusFrame {
    BusFrame::confirmation(build_confirmation_envelope(1, DeliveryStatus::Ok, 0, 0))
}

pub fn event_frame() -> BusFrame {
    BusFrame::event(event_envelope(
        0,
        1,
        0,
        Some(Origin::Internal),
        DaliEventPayload {
            wire_address: 2,
            command: 5,
            repeat_count: 1,
        },
    ))
}

pub fn projected_event_frame(correlation_id: u64) -> BusFrame {
    const SETPOINT: dali2rust_contracts::msg::LightSetpoint =
        dali2rust_contracts::msg::LightSetpoint {
            power: dali2rust_contracts::msg::PowerState::On,
            level: 128,
            color: None,
        };
    BusFrame::event(event_envelope(
        0,
        correlation_id,
        0,
        Some(Origin::Internal),
        dali2rust_contracts::msg::RuntimeStateChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: Some(1),
            short_address: Some(1),
            state_setpoint: SETPOINT,
            state_observation: dali2rust_contracts::msg::RuntimeObservation::default(),
            commit_source: dali2rust_contracts::msg::RuntimeSource::Api,
            commit_dimensions: SETPOINT.dimensions(),
        },
    ))
}

pub fn publish_queued(publisher: &BusPublisher, channel: BusChannel, frame: BusFrame) {
    assert_eq!(publisher.try_publish(channel, frame), PublishResult::Queued);
}

const REFUSAL_BATCH: usize = 4;

const REFUSAL_DEADLINE: Duration = Duration::from_secs(2);

pub struct PublishTally {
    pub attempted: u32,
    pub refused: u32,
}

pub fn publish_until_refused(
    publisher: &BusPublisher,
    channel: BusChannel,
    make_frame: impl Fn() -> BusFrame,
    want: u32,
) -> PublishTally {
    let mut tally = PublishTally {
        attempted: 0,
        refused: 0,
    };
    wait_until(
        || {
            for _ in 0..REFUSAL_BATCH {
                tally.attempted += 1;
                let result = publisher.try_publish(channel, make_frame());
                tally.refused += u32::from(result == PublishResult::DroppedIngressFull);
            }
            tally.refused >= want
        },
        REFUSAL_DEADLINE,
    );
    tally
}

pub struct EventObserver {
    rx: Receiver<BusFrame>,
}

impl EventObserver {
    pub fn new(rx: Receiver<BusFrame>) -> Self {
        Self { rx }
    }

    pub fn recv_matching(
        &self,
        timeout: Duration,
        predicate: impl Fn(&BusEventPayload) -> bool,
    ) -> Arc<EventEnvelope> {
        recv_event_matching(&self.rx, timeout, predicate)
    }

    pub fn into_inner(self) -> Receiver<BusFrame> {
        self.rx
    }
}

pub fn recv_confirmation_for(
    rx: &Receiver<BusFrame>,
    correlation_id: u64,
    timeout: Duration,
) -> Arc<ConfirmationEnvelope> {
    recv_matching_frame(rx, timeout, |frame| match frame {
        BusFrame::Confirmation(conf) if conf.meta.correlation_id == correlation_id => {
            Some(Arc::clone(conf))
        }
        _ => None,
    })
}

pub fn recv_event_matching(
    rx: &Receiver<BusFrame>,
    timeout: Duration,
    predicate: impl Fn(&BusEventPayload) -> bool,
) -> Arc<EventEnvelope> {
    recv_matching_frame(rx, timeout, |frame| match frame {
        BusFrame::Event(event) if predicate(&event.payload) => Some(Arc::clone(event)),
        _ => None,
    })
}

pub fn recv_command_matching(
    rx: &Receiver<BusFrame>,
    timeout: Duration,
    predicate: impl Fn(&BusCommandPayload) -> bool,
) -> CommandEnvelope {
    recv_matching_frame(rx, timeout, |frame| match frame {
        BusFrame::Command(command) if predicate(&command.payload) => Some(command.as_ref().clone()),
        _ => None,
    })
}

pub fn try_recv_command_matching(
    rx: &Receiver<BusFrame>,
    timeout: Duration,
    predicate: impl Fn(&BusCommandPayload) -> bool,
) -> Option<CommandEnvelope> {
    try_recv_matching_frame(rx, timeout, |frame| match frame {
        BusFrame::Command(command) if predicate(&command.payload) => Some(command.as_ref().clone()),
        _ => None,
    })
}

pub fn try_recv_event_matching_envelope(
    rx: &Receiver<BusFrame>,
    timeout: Duration,
    predicate: impl Fn(&EventEnvelope) -> bool,
) -> Option<Arc<EventEnvelope>> {
    try_recv_matching_frame(rx, timeout, |frame| match frame {
        BusFrame::Event(event) if predicate(event) => Some(Arc::clone(event)),
        _ => None,
    })
}

fn recv_matching_frame<T>(
    rx: &Receiver<BusFrame>,
    timeout: Duration,
    select: impl FnMut(&BusFrame) -> Option<T>,
) -> T {
    try_recv_matching_frame(rx, timeout, select)
        .unwrap_or_else(|| panic!("matching bus frame not observed within {timeout:?}"))
}

fn try_recv_matching_frame<T>(
    rx: &Receiver<BusFrame>,
    timeout: Duration,
    mut select: impl FnMut(&BusFrame) -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let slice = remaining.min(SHORT_POLL);
        match rx.recv_timeout(slice) {
            Ok(frame) => {
                if let Some(selected) = select(&frame) {
                    return Some(selected);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(err) => panic!("receive failed before deadline: {err}"),
        }
    }
}

const BURST_STANDOFF: Duration = Duration::from_millis(15);

const MAX_BURST_STANDOFF: Duration = Duration::from_millis(240);

const SPARSE_BURST: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloodOutcome {
    pub bursts: u32,
    pub drained: u32,
}

impl FloodOutcome {
    pub fn burst_pressure_held(self) -> bool {
        self.bursts > 1 && self.drained > 0
    }

    pub fn since(self, earlier: Self) -> Self {
        Self {
            bursts: self.bursts.saturating_sub(earlier.bursts),
            drained: self.drained.saturating_sub(earlier.drained),
        }
    }
}

#[derive(Debug, Default)]
pub struct FloodProgress {
    bursts: std::sync::atomic::AtomicU32,
    drained: std::sync::atomic::AtomicU32,
}

impl FloodProgress {
    pub fn snapshot(&self) -> FloodOutcome {
        FloodOutcome {
            bursts: self.bursts.load(std::sync::atomic::Ordering::Relaxed),
            drained: self.drained.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

pub fn flood_in_bursts(
    publisher: &BusPublisher,
    channel: BusChannel,
    make_frame: impl Fn() -> BusFrame,
    stop: &std::sync::atomic::AtomicBool,
    max_duration: Duration,
) -> FloodOutcome {
    flood_in_bursts_observed(
        publisher,
        channel,
        make_frame,
        stop,
        max_duration,
        &FloodProgress::default(),
    )
}

pub fn flood_in_bursts_observed(
    publisher: &BusPublisher,
    channel: BusChannel,
    make_frame: impl Fn() -> BusFrame,
    stop: &std::sync::atomic::AtomicBool,
    max_duration: Duration,
    progress: &FloodProgress,
) -> FloodOutcome {
    let until = Instant::now() + max_duration;
    let mut outcome = FloodOutcome {
        bursts: 0,
        drained: 0,
    };
    let mut standoff = BURST_STANDOFF;
    while Instant::now() < until && !stop.load(std::sync::atomic::Ordering::Relaxed) {
        let mut accepted = 0u32;
        while publisher.try_publish(channel, make_frame()) == PublishResult::Queued {
            accepted = accepted.saturating_add(1);
        }
        if outcome.bursts > 0 {
            outcome.drained = outcome.drained.saturating_add(accepted);
            progress
                .drained
                .store(outcome.drained, std::sync::atomic::Ordering::Relaxed);
        }
        outcome.bursts = outcome.bursts.saturating_add(1);
        progress
            .bursts
            .store(outcome.bursts, std::sync::atomic::Ordering::Relaxed);
        standoff = if accepted <= SPARSE_BURST && outcome.bursts > 1 {
            (standoff * 2).min(MAX_BURST_STANDOFF)
        } else {
            BURST_STANDOFF
        };
        // sleep-ok: burst stand-off, one bus drain interval plus margin
        std::thread::sleep(standoff);
    }
    outcome
}
