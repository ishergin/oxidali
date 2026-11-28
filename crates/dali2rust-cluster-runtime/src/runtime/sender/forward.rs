use core::sync::atomic::Ordering::Relaxed;

use dali2rust_contracts::msg::{BusEventPayload, ClusterCommandCommand};
use dali2rust_contracts::CORRELATION_NONE;
use dali2rust_platform::liveness::monotonic_ms;

use crate::frame::{encode, Frame};
use crate::presence::OwnMode;

use super::super::counters::bump;
use super::super::forwarding::{forwarding, Fact, ForeignStanding};
use super::super::hold::Held;
use super::super::outgoing::{is_relative, Queued};
use super::{NodeSender, Reading};

const NOTE_COPIES: usize = 2;

impl NodeSender<'_> {
    pub(super) fn forward(&mut self, payload: &BusEventPayload) {
        if self.mode != OwnMode::Running {
            return;
        }
        let Some(fact) = Fact::of(payload) else {
            return;
        };
        let now_ms = monotonic_ms();
        let wire = self.ports.presence.foreign_standing(now_ms);
        let reading = Reading::of(self.ports);
        match forwarding(&fact, &reading.settings, &*self.ports.parts, wire) {
            Ok(request) if fact.foreign() => self.hold_foreign(&reading, request, wire.shared, now_ms),
            Ok(request) => self.send_forwarded(&reading, request),
            Err(skip) => {
                bump(self.ports.counters.skipped(skip));
                self.note_wire(&reading, &fact, wire);
            }
        }
    }

    fn note_wire(&mut self, reading: &Reading, fact: &Fact, wire: ForeignStanding) {
        if fact.foreign() || !wire.shared || wire.forwards {
            return;
        }
        let header = self.next_header(reading);
        let written = encode(&header, &Frame::WireNote(fact.note(reading.wire())), &mut self.buffer).ok();
        for _ in 0..NOTE_COPIES {
            self.transmit(written);
        }
    }

    fn send_forwarded(&mut self, reading: &Reading, request: ClusterCommandCommand) {
        bump(&self.ports.counters.commands_forwarded_total);
        let queued = Queued { correlation_id: CORRELATION_NONE, request, forwarded: true };
        self.admit_request(reading, queued);
    }

    fn hold_foreign(&mut self, reading: &Reading, request: ClusterCommandCommand, shared: bool, now_ms: u32) {
        let held = self.hold.line().hold(request, shared, now_ms);
        let counters = &self.ports.counters;
        match held {
            Held::Cancelled => bump(&counters.foreign_cancelled_total),
            Held::Queued { held, pushed_out } => {
                if held {
                    bump(&counters.foreign_held_total);
                }
                if let Some(early) = pushed_out {
                    bump(&counters.foreign_hold_full_total);
                    self.send_foreign(reading, early);
                }
            }
        }
        self.release_foreign(reading, now_ms);
    }

    pub(super) fn release_foreign(&mut self, reading: &Reading, now_ms: u32) {
        if self.mode != OwnMode::Running {
            let relative = self.hold.line().clear().filter(|request| is_relative(&request.action)).count();
            let relative = u32::try_from(relative).unwrap_or(u32::MAX);
            self.ports.counters.relative_verbs_dropped_total.fetch_add(relative, Relaxed);
            return;
        }
        loop {
            let due = self.hold.line().take_due(now_ms);
            let Some(request) = due else {
                return;
            };
            self.send_foreign(reading, request);
        }
    }

    fn send_foreign(&mut self, reading: &Reading, request: ClusterCommandCommand) {
        bump(&self.ports.counters.foreign_forwarded_total);
        self.send_forwarded(reading, request);
    }
}
