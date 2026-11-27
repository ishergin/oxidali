use dali2rust_contracts::msg::{ClusterCommandCommand, ErrorCode};
use dali2rust_platform::liveness::monotonic_ms;

use crate::frame::{encode, Action, Command, Frame, Header};
use crate::presence::OwnMode;

use super::super::counters::bump;
use super::super::execute::{is_supported, Gate, Stamp};
use super::super::outgoing::{Admission, Due, Queued, Repeat, TargetKey};
use super::{confirm, NodeSender, Reading};

pub const CONTROLLER_PASSIVE: &str = "controller_passive";
pub const CLUSTER_OFF: &str = "cluster_off";
pub const CLUSTER_NO_ADDRESS: &str = "cluster_no_address";
pub const CLUSTER_NOT_RUNNING: &str = "cluster_not_running";
pub const CLUSTER_BUSY: &str = "cluster_busy";
pub const CLUSTER_COMMAND_UNSUPPORTED: &str = "cluster_command_unsupported";
pub const CLUSTER_SEND_FAILED: &str = "cluster_send_failed";
const SUPERSEDED: &str = "superseded";

impl NodeSender<'_> {
    pub(super) fn on_command(&mut self, correlation_id: u64, request: &ClusterCommandCommand) {
        let reading = Reading::of(self.ports);
        if let Some(cause) = self.refusal(&reading, request) {
            return self.refuse(correlation_id, ErrorCode::Conflict, cause);
        }
        self.admit_request(&reading, Queued { correlation_id, request: request.clone(), forwarded: false });
    }

    pub(super) fn admit_request(&mut self, reading: &Reading, queued: Queued) {
        let now_ms = monotonic_ms();
        match self.outgoing.admit(queued, now_ms) {
            Admission::Now(queued) => self.send_command(reading, queued, now_ms),
            Admission::AfterCopy => {}
            Admission::Later { superseded } => self.delayed(superseded),
            Admission::Dropped(queued) => self.dropped(&queued),
            Admission::Full(queued) => self.refuse(queued.correlation_id, ErrorCode::Conflict, CLUSTER_BUSY),
        }
    }

    fn refusal(&self, reading: &Reading, request: &ClusterCommandCommand) -> Option<&'static str> {
        let unsupported = || (!is_supported(&request.action)).then_some(CLUSTER_COMMAND_UNSUPPORTED);
        self.mode_refusal(reading).or_else(unsupported)
    }

    fn mode_refusal(&self, reading: &Reading) -> Option<&'static str> {
        if !reading.active {
            return Some(CONTROLLER_PASSIVE);
        }
        match self.mode {
            OwnMode::Off => Some(CLUSTER_OFF),
            OwnMode::NoAddress => Some(CLUSTER_NO_ADDRESS),
            OwnMode::Running => None,
            OwnMode::BindFailed | OwnMode::Standby | OwnMode::Listening | OwnMode::Waiting => Some(CLUSTER_NOT_RUNNING),
        }
    }

    fn delayed(&self, superseded: Option<Queued>) {
        bump(&self.ports.counters.commands_delayed_total);
        if let Some(earlier) = superseded {
            bump(&self.ports.counters.commands_superseded_total);
            self.refuse(earlier.correlation_id, ErrorCode::Superseded, SUPERSEDED);
        }
    }

    fn dropped(&self, queued: &Queued) {
        bump(&self.ports.counters.relative_verbs_dropped_total);
        self.refuse(queued.correlation_id, ErrorCode::Conflict, CLUSTER_BUSY);
    }

    fn refuse(&self, correlation_id: u64, code: ErrorCode, cause: &str) {
        confirm(&self.ports.publisher, correlation_id, Some((code, cause)));
    }

    pub(super) fn carry_commands(&mut self, reading: &Reading, now_ms: u32) {
        if let Some(cause) = self.mode_refusal(reading) {
            let ports = self.ports;
            for abandoned in self.outgoing.abandon() {
                confirm(&ports.publisher, abandoned.correlation_id, Some((ErrorCode::Conflict, cause)));
            }
            return;
        }
        while let Some(due) = self.outgoing.take_due(now_ms) {
            match due {
                Due::Repeat(repeat) => self.repeat(&repeat),
                Due::Waiting(queued) => self.send_command(reading, queued, now_ms),
            }
        }
        self.outgoing.prune(now_ms);
    }

    fn send_command(&mut self, reading: &Reading, queued: Queued, now_ms: u32) {
        let Queued { correlation_id, request, forwarded } = queued;
        let executed_on_wire = self.executed_on_wire(reading, &request.action, forwarded);
        let command = Command { action: request.action, source: request.source, hold_hcl: request.hold_hcl, executed_on_wire };
        let header = self.next_header(reading);
        let key = TargetKey::of(&command.action);
        if !self.transmit_command(&header, &command) {
            return self.refuse(correlation_id, ErrorCode::ExecutionFailed, CLUSTER_SEND_FAILED);
        }
        if !forwarded {
            self.execution.execute(&command.action, Stamp::of(&command), Gate::Own);
        }
        bump(&self.ports.counters.commands_sent_total);
        self.outgoing.sent(key, Repeat { header, command }, now_ms);
        confirm(&self.ports.publisher, correlation_id, None);
    }

    fn executed_on_wire(&self, reading: &Reading, action: &Action, forwarded: bool) -> Option<u8> {
        let executed_here = forwarded || self.execution.plan(action, Gate::Own).accepted > 0;
        executed_here.then_some(reading.wire())
    }

    fn repeat(&mut self, repeat: &Repeat) {
        self.transmit_command(&repeat.header, &repeat.command);
    }

    fn transmit_command(&mut self, header: &Header, command: &Command) -> bool {
        let written = encode(header, &Frame::Command(command.clone()), &mut self.buffer).ok();
        self.transmit(written)
    }
}
