use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, PublishResult};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::BusCommandPayload;
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};

use super::super::announce::reached;
use super::super::copies::{copy_commands, Announced, CopyReads, COPY_COMMANDS_PER_PASS};
use super::super::round::{takes_block, RoundReads, RoundTurn};
use super::super::shared::ClusterPresenceReadPort;
use super::super::standing::HomeReads;
use super::NodeSender;

impl NodeSender<'_> {
    pub(super) fn keep_block(&mut self, own: u8, running: bool, now_ms: u32) {
        let timing = self.ports.presence.timing();
        let records = &*self.ports.records;
        let reads = RoundReads { presence: &self.ports.presence, info: &self.ports.info, records, timing };
        let turn = RoundTurn { running: takes_block(running, self.ports.presence.own_status().block), own, now_ms };
        for take_back in self.round.advance(turn, reads) {
            self.publish_registry(take_back.into());
        }
        self.ports.presence.note_block_settled(self.round.settled());
        if running && reached(now_ms, self.next_copy_ms) {
            for take_back in self.round.late(turn, reads) {
                self.publish_registry(take_back.into());
            }
            self.sync_copies(own, now_ms);
        }
    }

    fn sync_copies(&mut self, own: u8, now_ms: u32) {
        let home = HomeReads { presence: &*self.ports.presence, info: &*self.ports.info };
        let timing = self.ports.presence.timing();
        let own_block = self.ports.presence.own_status().block;
        let announced = Announced::of(self.announcer.sets_sent(now_ms), own_block, now_ms, timing);
        let reads = CopyReads { home, parts: &*self.ports.parts, copies: &*self.ports.copies, announced };
        let commands = copy_commands(own, reads);
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
