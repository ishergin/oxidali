use dali2rust_contracts::msg::BusEventPayload;
use dali2rust_contracts::CORRELATION_NONE;

use crate::presence::OwnMode;

use super::super::counters::bump;
use super::super::forwarding::{forwarding, Fact};
use super::super::outgoing::Queued;
use super::{NodeSender, Reading};

impl NodeSender<'_> {
    pub(super) fn forward(&mut self, payload: &BusEventPayload) {
        if self.mode != OwnMode::Running {
            return;
        }
        let Some(fact) = Fact::of(payload) else {
            return;
        };
        let reading = Reading::of(self.ports);
        match forwarding(&fact, &reading.settings, &*self.ports.parts) {
            Ok(request) => {
                bump(&self.ports.counters.commands_forwarded_total);
                let queued = Queued { correlation_id: CORRELATION_NONE, request, forwarded: true };
                self.admit_request(&reading, queued);
            }
            Err(skip) => bump(self.ports.counters.skipped(skip)),
        }
    }
}
