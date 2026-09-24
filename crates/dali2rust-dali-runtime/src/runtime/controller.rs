use core::time::Duration;
use std::sync::{Arc, Mutex};

use dali2rust_domain::dali::commands::{DaliCommand, DaliResponse};
use dali2rust_domain::dali::controller::{
    DaliApplicationController, DaliProductController, Frame24Fault,
};
use dali2rust_domain::dali::frame::{ForwardFrame, FrameError};
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::ses::{
    DaliPriority, DaliSession, RetryPolicy, TransactionPriority,
};
use dali2rust_platform::clock::Clock;
use dali2rust_platform::dali::{
    DaliTransport, DaliWireCounters, Frame24Error, FrameDirection, ObservedRawFrameKind,
    SnifferTap, TransferOutcome, WireLease, YieldGranularity,
};
use std::sync::atomic::Ordering::Relaxed;

mod transaction;
mod retry;
mod sniffer;

#[cfg(test)]
use transaction::BUS_RELEASE_SETTLE_US;
use retry::unit_retryable;

pub const PHY_TICK_US: u32 = 104;

pub const TX_ARM_LEAD_TICKS: u8 = 3;

const MAX_SETTLE_MS: u64 = 20;
const MAX_EXCHANGE_MS: u64 = 50;
pub const BACKGROUND_YIELD_BUDGET_MS: u64 = MAX_SETTLE_MS + MAX_EXCHANGE_MS;

#[derive(Clone, Copy)]
enum WireFrame {
    Forward16(ForwardFrame),
    Forward24([u8; 3]),
}

#[derive(Clone, Copy)]
struct ExchangeObservation {
    outcome: TransferOutcome,
    contended: bool,
}

pub struct DaliController<T: DaliTransport + Send> {
    transport: Arc<Mutex<T>>,
    session: DaliSession,
    clock: Box<dyn Clock>,
    retry_policy: RetryPolicy,
    retry_seed: u64,
    lease: Option<WireLease>,
    at_step_boundary: bool,
    sniffer: Option<Arc<SnifferTap>>,
    sniffer_adapter_id: u8,
    transport_honours_settle: bool,
    wire_class: TransactionPriority,
    tx: TransactionState,
    last_release_ms: u64,
    busy_since_release: bool,
    wire_counters: Arc<DaliWireCounters>,
}

#[derive(Debug, Default)]
struct TransactionState {
    depth: u16,
    first_frame_sent: bool,
    exempt: bool,
    first_frame_at_ms: u64,
    last_frame_at_ms: u64,
}

impl<T: DaliTransport + Send> DaliController<T> {
    pub fn new(transport: Arc<Mutex<T>>, clock: Box<dyn Clock>) -> Self {
        Self::with_retry_policy(transport, clock, RetryPolicy::default())
    }

    pub fn with_retry_policy(
        transport: Arc<Mutex<T>>,
        clock: Box<dyn Clock>,
        retry_policy: RetryPolicy,
    ) -> Self {
        let clock_now = clock.monotonic_ms();
        let retry_seed = clock_now.max(1);
        let transport_honours_settle = transport
            .lock()
            .expect("DaliController: transport mutex poisoned before first use")
            .honours_settle();
        log::info!(
            "DALI controller: transport {} the settling time itself",
            if transport_honours_settle {
                "enforces"
            } else {
                "does NOT enforce"
            }
        );
        Self {
            transport,
            session: DaliSession::new(),
            clock,
            retry_policy,
            retry_seed,
            lease: None,
            at_step_boundary: false,
            sniffer: None,
            sniffer_adapter_id: 0,
            transport_honours_settle,
            wire_class: TransactionPriority::Configuration,
            tx: TransactionState::default(),
            last_release_ms: clock_now,
            busy_since_release: false,
            wire_counters: Arc::new(DaliWireCounters::default()),
        }
    }

    pub fn set_wire_counters(&mut self, counters: Arc<DaliWireCounters>) {
        self.wire_counters = counters;
    }

}

impl<T: DaliTransport + Send> DaliProductController for DaliController<T> {
    type Error = FrameError;

    fn send_command(&mut self, cmd: &DaliCommand) -> Result<DaliResponse, Self::Error> {
        self.send_command_observed(cmd).map(|(response, _)| response)
    }

    fn send_command_observed(
        &mut self,
        cmd: &DaliCommand,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        self.send_command_impl(cmd)
    }

    fn session(&self) -> &DaliSession {
        &self.session
    }
}

impl<T: DaliTransport + Send> DaliApplicationController for DaliController<T> {
    fn send_raw(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<DaliResponse, Self::Error> {
        self.send_raw_observed(frame, expects_backward)
            .map(|(response, _)| response)
    }

    fn send_raw_observed(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        self.transmit_frame_observed(frame, expects_backward)
    }

    fn send_raw_once(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        self.exchange_once(frame, expects_backward)
    }

    fn send_frame24(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
    ) -> Result<DaliResponse, Frame24Fault> {
        let attempts = self.retry_policy.effective_max_attempts();
        self.perform_exchange24(frame, expects_backward, attempts)
    }

    fn send_frame24_once(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
    ) -> Result<DaliResponse, Frame24Fault> {
        self.perform_exchange24(frame, expects_backward, 1)
    }

    fn supports_frame24(&self) -> bool {
        self.transport
            .lock()
            .expect("DaliController: transport mutex poisoned")
            .supports_frame24()
    }

    fn send_raw_pair(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        self.run_transaction(false, |c| c.send_unit(None, frame, expects_backward, true))
    }

    fn send_raw_enabled_query(
        &mut self,
        enable: ForwardFrame,
        frame: ForwardFrame,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        self.run_transaction(false, |c| c.send_unit(Some(enable), frame, true, false))
    }

    fn with_wire_lease<R>(&mut self, lease: WireLease, run: impl FnOnce(&mut Self) -> R) -> R {
        let previous = self.lease.replace(lease);
        let previous_boundary = std::mem::take(&mut self.at_step_boundary);
        let result = run(self);
        self.lease = previous;
        self.at_step_boundary = previous_boundary;
        result
    }

    fn with_wire_class<R>(
        &mut self,
        class: Option<TransactionPriority>,
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous = class.map(|class| std::mem::replace(&mut self.wire_class, class));
        let result = run(self);
        if let Some(previous) = previous {
            self.wire_class = previous;
        }
        if self.tx.depth > 0 {
            self.wire_counters.transaction_leaks.fetch_add(1, Relaxed);
            debug_assert_eq!(self.tx.depth, 0, "transaction leaked past its command");
            self.tx = TransactionState::default();
        }
        result
    }

    fn step_boundary(&mut self) {
        self.at_step_boundary = true;
        if self.tx.depth > 0 && self.tx.first_frame_sent && self.yield_pending() {
            self.close_transaction_for_yield();
        }
    }

    fn transaction<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        self.run_transaction(false, run)
    }

    fn transaction_exempt<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        self.run_transaction(true, run)
    }
}

impl<T: DaliTransport + Send> DaliController<T> {
    fn send_command_impl(&mut self, cmd: &DaliCommand) -> Result<(DaliResponse, bool), FrameError> {
        let frame = cmd.to_forward_frame();
        let expects_backward = cmd.is_query();
        let repeats = cmd.requires_repeat();
        let enable = match cmd {
            DaliCommand::Extended { command, .. } => Some(
                SpecialCommand::EnableDeviceType(command.enable_device_type().code())
                    .to_forward_frame(),
            ),
            _ => None,
        };

        let (response, contended) =
            self.run_transaction(false, |c| c.send_unit(enable, frame, expects_backward, repeats))?;
        Ok((response_if_expected(response, expects_backward), contended))
    }

    fn send_unit(
        &mut self,
        enable: Option<ForwardFrame>,
        frame: ForwardFrame,
        expects_backward: bool,
        repeats: bool,
    ) -> Result<(DaliResponse, bool), FrameError> {
        let attempts = self.retry_policy.effective_max_attempts();
        for attempt in 0..attempts {
            match self.try_unit_once(enable, frame, expects_backward, repeats) {
                Ok(result) => return Ok(result),
                Err(error) if !unit_retryable(error) => return Err(error),
                Err(error) => {
                    self.retry_or_fail(attempt, attempts, error)?;
                    self.note_unit_reopened();
                }
            }
        }
        Err(FrameError::TransportError)
    }

    fn try_unit_once(
        &mut self,
        enable: Option<ForwardFrame>,
        frame: ForwardFrame,
        expects_backward: bool,
        repeats: bool,
    ) -> Result<(DaliResponse, bool), FrameError> {
        let mut contended = false;
        if let Some(enable) = enable {
            contended |= self.exchange_once(enable, false)?.1;
        }
        let (mut backward, observed) = self.exchange_once(frame, expects_backward)?;
        contended |= observed;
        if repeats {
            let (repeat_backward, observed) = self.exchange_once(frame, expects_backward)?;
            backward = repeat_backward;
            contended |= observed;
            self.judge_send_twice_interval()?;
        }
        Ok((backward, contended))
    }

    // IEC 62386-101 Table 17, Table 20
    fn judge_send_twice_interval(&mut self) -> Result<(), FrameError> {
        let Some(ticks) = self.last_tx_settle_ticks() else {
            return Ok(());
        };
        let interval_us = u32::from(ticks) * PHY_TICK_US;
        if interval_us >= RetryPolicy::SEND_TWICE_MAX_INTERVAL_US {
            self.wire_counters.send_twice_split.fetch_add(1, Relaxed);
            return Err(FrameError::BusBusy);
        }
        if interval_us >= RetryPolicy::SEND_TWICE_TX_MAX_US {
            self.wire_counters
                .send_twice_over_transmitter_max
                .fetch_add(1, Relaxed);
        }
        Ok(())
    }

    fn exchange_once(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<(DaliResponse, bool), FrameError> {
        let obs = self.exchange_with_attempts(WireFrame::Forward16(frame), expects_backward, 1)?;
        Ok((response_from_outcome(obs.outcome), obs.contended))
    }

    fn session_wait(&mut self, settle_us: u32) -> Result<(), FrameError> {
        if self.transport_honours_settle {
            return Ok(());
        }
        let wait = self
            .session
            .min_wait_before_next(settle_us, self.clock.as_ref());
        if !wait.is_zero() {
            // sleep-ok: IEC 62386-102 inter-frame settle — blocking wait is the session contract
            std::thread::sleep(wait);
        }
        Ok(())
    }

    fn transmit_frame_observed(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<(DaliResponse, bool), FrameError> {
        let attempts = self.retry_policy.effective_max_attempts();
        let outcome =
            self.exchange_with_attempts(WireFrame::Forward16(frame), expects_backward, attempts)?;
        Ok((response_from_outcome(outcome.outcome), outcome.contended))
    }

    fn perform_exchange24(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
        attempts: u8,
    ) -> Result<DaliResponse, Frame24Fault> {
        if !self.supports_frame24() {
            return Err(Frame24Fault::Unsupported);
        }
        let observation = self
            .exchange_with_attempts(WireFrame::Forward24(frame), expects_backward, attempts)
            .map_err(fault24_from_error)?;
        response24_from_outcome(observation.outcome)
    }

    fn perform_exchange(
        &mut self,
        frame: WireFrame,
        expects_backward: bool,
        settle_us: u32,
    ) -> Result<ExchangeObservation, FrameError> {
        let mut t = self
            .transport
            .lock()
            .expect("DaliController: transport mutex poisoned (previous panic in transport code)");
        let foreign_before = t.foreign_activity();
        let outcome = match frame {
            WireFrame::Forward16(f) => t
                .exchange_frame_with_settle(f.raw(), expects_backward, settle_us)
                .map_err(|_| FrameError::TransportError)?,
            WireFrame::Forward24(bytes) => {
                match t.exchange_frame24_with_settle(bytes, expects_backward, settle_us) {
                    Ok(outcome) => outcome,
                    Err(Frame24Error::Transport(_)) => TransferOutcome::NoAnswer,
                    Err(Frame24Error::Unsupported) => return Err(FrameError::TransportError),
                }
            }
        };
        let foreign_after = t.foreign_activity();
        Ok(ExchangeObservation {
            outcome,
            contended: foreign_after != foreign_before,
        })
    }

    // IEC 62386-101 §9.1.3, §9.1.4
    fn note_frame_transmitted(&mut self, outcome: TransferOutcome, priority: DaliPriority) {
        self.note_outcome_counter(outcome);
        if matches!(
            outcome,
            TransferOutcome::BusBusy | TransferOutcome::Collision
        ) {
            return;
        }
        self.session.record_transmission(self.clock.as_ref());
        self.busy_since_release = true;
        self.wire_counters.note_frame_sent(priority as u8);
        self.note_settle_window(priority);
        let now = self.clock.monotonic_ms();
        self.tx.last_frame_at_ms = now;
        if self.tx.depth > 0 && !self.tx.first_frame_sent {
            self.tx.first_frame_sent = true;
            self.tx.first_frame_at_ms = now;
            self.wire_counters
                .note_transaction_started(self.wire_class as u8);
        }
    }

    fn note_outcome_counter(&self, outcome: TransferOutcome) {
        let counter = match outcome {
            TransferOutcome::BusBusy => &self.wire_counters.bus_acquire_timeout,
            TransferOutcome::Collision => &self.wire_counters.collisions,
            TransferOutcome::ForeignInWindow => &self.wire_counters.foreign_in_window,
            TransferOutcome::CorruptedInWindow => &self.wire_counters.corrupted_in_window,
            TransferOutcome::Answer(_) | TransferOutcome::NoAnswer => return,
        };
        counter.fetch_add(1, Relaxed);
    }

    fn resolve_exchange_outcome(
        &mut self,
        exchange: ExchangeObservation,
        expects_backward: bool,
        attempt: u8,
        attempts: u8,
        frame: WireFrame,
    ) -> Result<Option<TransferOutcome>, FrameError> {
        if self.should_retry_contended_query_no_answer(exchange, expects_backward) {
            return self.retry_query_no_answer(attempt, attempts);
        }
        match exchange.outcome {
            TransferOutcome::Answer(_) | TransferOutcome::NoAnswer => Ok(Some(exchange.outcome)),
            TransferOutcome::Collision => self.retry_or_fail(attempt, attempts, FrameError::Collision),
            TransferOutcome::BusBusy => self.retry_or_fail(attempt, attempts, FrameError::BusBusy),
            TransferOutcome::ForeignInWindow | TransferOutcome::CorruptedInWindow
                if matches!(frame, WireFrame::Forward24(_)) =>
            {
                Ok(Some(exchange.outcome))
            }
            // IEC 62386-101 §8.2.5
            TransferOutcome::CorruptedInWindow if expects_backward => Ok(Some(exchange.outcome)),
            TransferOutcome::ForeignInWindow if expects_backward => {
                self.retry_or_fail(attempt, attempts, FrameError::Collision)
            }
            TransferOutcome::ForeignInWindow | TransferOutcome::CorruptedInWindow => {
                Ok(Some(TransferOutcome::NoAnswer))
            }
        }
    }

}

fn response_from_outcome(outcome: TransferOutcome) -> DaliResponse {
    match outcome {
        TransferOutcome::Answer(backward) => DaliResponse::Answer(backward),
        TransferOutcome::NoAnswer => DaliResponse::NoAnswer,
        TransferOutcome::CorruptedInWindow => DaliResponse::Violation,
        TransferOutcome::Collision | TransferOutcome::BusBusy | TransferOutcome::ForeignInWindow => {
            DaliResponse::NoAnswer
        }
    }
}

fn fault24_from_error(error: FrameError) -> Frame24Fault {
    match error {
        FrameError::Preempted => Frame24Fault::Preempted,
        FrameError::Collision
        | FrameError::BusBusy
        | FrameError::TransportError
        | FrameError::BackwardTimeout
        | FrameError::BackwardNack => Frame24Fault::Contended,
    }
}

fn response24_from_outcome(outcome: TransferOutcome) -> Result<DaliResponse, Frame24Fault> {
    match outcome {
        TransferOutcome::Collision | TransferOutcome::BusBusy | TransferOutcome::ForeignInWindow => {
            Err(Frame24Fault::Contended)
        }
        terminal => Ok(response_from_outcome(terminal)),
    }
}

fn response_if_expected(response: DaliResponse, expects_backward: bool) -> DaliResponse {
    if expects_backward {
        response
    } else {
        DaliResponse::NoAnswer
    }
}

#[cfg(test)]
mod tests;
