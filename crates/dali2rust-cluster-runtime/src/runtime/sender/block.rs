use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, PublishResult};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::BusCommandPayload;
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};

use super::super::announce::reached;
use super::super::copies::{copy_commands, CopyReads, COPY_COMMANDS_PER_PASS};
use super::super::round::{RoundReads, RoundTurn};
use super::super::standing::HomeReads;
use super::NodeSender;

impl NodeSender<'_> {
    pub(super) fn keep_block(&mut self, own: u8, running: bool, now_ms: u32) {
        let timing = self.ports.presence.timing();
        let records = &*self.ports.records;
        let reads = RoundReads { presence: &self.ports.presence, info: &self.ports.info, records, timing };
        for take_back in self.round.advance(RoundTurn { running, own, now_ms }, reads) {
            self.publish_registry(take_back.into());
        }
        self.ports.presence.note_block_settled(self.round.settled());
        if running && reached(now_ms, self.next_copy_ms) {
            self.sync_copies(own, now_ms);
        }
    }

    fn sync_copies(&mut self, own: u8, now_ms: u32) {
        let home = HomeReads { presence: &*self.ports.presence, info: &*self.ports.info };
        let reads = CopyReads { home, parts: &*self.ports.parts, copies: &*self.ports.copies };
        let commands = copy_commands(own, reads);
        let timing = self.ports.presence.timing();
        let full = commands.len() >= COPY_COMMANDS_PER_PASS;
        let next_ms = if full { timing.info_coalesce_ms() } else { timing.pulse_period_ms() };
        self.next_copy_ms = now_ms.wrapping_add(next_ms);
        for command in commands {
            self.publish_registry(command.into());
        }
    }

    pub(super) fn block_deadline(&self) -> Option<u32> {
        let timing = self.ports.presence.timing();
        let copies = (self.mode == crate::presence::OwnMode::Running).then_some(self.next_copy_ms);
        [self.round.deadline(timing), copies].into_iter().flatten().reduce(|earliest, at_ms| {
            if reached(at_ms, earliest) {
                earliest
            } else {
                at_ms
            }
        })
    }

    fn publish_registry(&self, payload: BusCommandPayload) -> bool {
        let envelope = command_envelope(SOURCE_ID_UNSPECIFIED, CORRELATION_NONE, self.ports.bus_id.0, None, payload);
        let frame = BusFrame::command(envelope);
        publish_or_drop(&self.ports.publisher, BusChannel::Commands, frame, "cluster-block") == PublishResult::Queued
    }
}
