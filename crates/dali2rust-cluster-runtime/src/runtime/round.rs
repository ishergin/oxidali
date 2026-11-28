use dali2rust_contracts::msg::{ClusterRecordKind, ClusterRecordTakeBackCommand, ClusterTakenRecord};
use dali2rust_domain::registry::{cluster_block, ClusterRecordReadPort, CLUSTER_RECORD_KINDS};

use crate::frame::NODE_COUNT;
use crate::info::{ClusterInfo, HeldRecord, InfoSet, BLOCK_SLOTS};
use crate::presence::{elapsed_ms, PresenceTiming};

use super::shared::{ClusterPresence, ClusterPresenceReadPort};

#[cfg(test)]
mod tests;

const MAX_TAKE_BACK_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Listening { since_ms: u32 },
    Applying { sent_ms: u32, attempts: u8 },
    Settled,
}

pub(crate) struct BlockRound {
    phase: Phase,
}

#[derive(Clone, Copy)]
pub(crate) struct RoundReads<'a> {
    pub(crate) presence: &'a ClusterPresence,
    pub(crate) info: &'a ClusterInfo,
    pub(crate) records: &'a dyn ClusterRecordReadPort,
    pub(crate) timing: PresenceTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoundTurn {
    pub(crate) running: bool,
    pub(crate) own: u8,
    pub(crate) now_ms: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Offer {
    pub(crate) best: Option<HeldRecord>,
    pub(crate) floor: u32,
}

impl Offer {
    fn take(&mut self, copy: Option<&HeldRecord>, referenced: impl Iterator<Item = u32>) {
        let rank = |held: &HeldRecord| (held.mark.generation, held.revision);
        if let Some(copy) = copy {
            self.floor = self.floor.max(copy.mark.generation);
            if self.best.as_ref().is_none_or(|best| rank(best) < rank(copy)) {
                self.best = Some(copy.clone());
            }
        }
        self.floor = referenced.fold(self.floor, u32::max);
    }
}

impl BlockRound {
    pub(crate) const fn new() -> Self {
        Self { phase: Phase::Idle }
    }

    pub(crate) const fn settled(&self) -> bool {
        matches!(self.phase, Phase::Settled)
    }

    pub(crate) fn advance(&mut self, turn: RoundTurn, reads: RoundReads<'_>) -> Vec<ClusterRecordTakeBackCommand> {
        let since_ms = match self.phase {
            Phase::Listening { since_ms } => since_ms,
            _ => turn.now_ms,
        };
        let heard = turn.running && every_member_heard(turn, since_ms, reads);
        self.step(turn, (heard, reads.timing), || take_back_intents(turn.own, turn.now_ms, reads))
    }

    pub(crate) fn step(
        &mut self,
        turn: RoundTurn,
        (heard, timing): (bool, PresenceTiming),
        intents: impl FnOnce() -> Vec<ClusterRecordTakeBackCommand>,
    ) -> Vec<ClusterRecordTakeBackCommand> {
        let (due, attempts) = match (turn.running, self.phase) {
            (false, _) => (false, self.reset()),
            (true, Phase::Idle) => (heard, self.listen_from(turn.now_ms)),
            (true, Phase::Listening { since_ms }) => (heard || elapsed_ms(turn.now_ms, since_ms) >= timing.block_round_ms(), 1),
            (true, Phase::Applying { sent_ms, attempts }) => (self.retry_due(turn.now_ms, (sent_ms, attempts), timing), attempts + 1),
            (true, Phase::Settled) => (false, 0),
        };
        if !due {
            return Vec::new();
        }
        let intents = intents();
        self.phase = if intents.is_empty() { Phase::Settled } else { Phase::Applying { sent_ms: turn.now_ms, attempts } };
        intents
    }

    pub(crate) fn deadline(&self, timing: PresenceTiming) -> Option<u32> {
        match self.phase {
            Phase::Listening { since_ms } => Some(since_ms.wrapping_add(timing.block_round_ms())),
            Phase::Applying { sent_ms, .. } => Some(sent_ms.wrapping_add(timing.info_coalesce_ms())),
            Phase::Idle | Phase::Settled => None,
        }
    }

    fn reset(&mut self) -> u8 {
        self.phase = Phase::Idle;
        0
    }

    fn listen_from(&mut self, now_ms: u32) -> u8 {
        self.phase = Phase::Listening { since_ms: now_ms };
        1
    }

    fn retry_due(&mut self, now_ms: u32, (sent_ms, attempts): (u32, u8), timing: PresenceTiming) -> bool {
        if elapsed_ms(now_ms, sent_ms) < timing.info_coalesce_ms() {
            return false;
        }
        if attempts >= MAX_TAKE_BACK_ATTEMPTS {
            self.phase = Phase::Settled;
            return false;
        }
        true
    }
}

fn every_member_heard(turn: RoundTurn, since_ms: u32, reads: RoundReads<'_>) -> bool {
    let waited_ms = elapsed_ms(turn.now_ms, since_ms);
    (0..NODE_COUNT).filter(|node| *node != turn.own).all(|node| {
        let Some(view) = reads.presence.heard_node(node).filter(|view| view.live) else {
            return true;
        };
        let heard = reads.info.with_shown(node, turn.now_ms, |_, held| held.mac == view.mac && held.age_ms < waited_ms);
        heard.unwrap_or(false)
    })
}

pub(crate) fn offers(own: u8, now_ms: u32, info: &ClusterInfo, kind: ClusterRecordKind) -> [Offer; BLOCK_SLOTS] {
    let mut offers: [Offer; BLOCK_SLOTS] = Default::default();
    for node in (0..NODE_COUNT).filter(|node| *node != own) {
        info.with_shown(node, now_ms, |set, _| offer_of(set, own, kind, &mut offers));
    }
    offers
}

fn offer_of(set: &InfoSet, own: u8, kind: ClusterRecordKind, offers: &mut [Offer; BLOCK_SLOTS]) {
    if set.own() != own {
        return;
    }
    let numbers = *cluster_block(own).start()..;
    for ((number, offer), copy) in numbers.zip(offers.iter_mut()).zip(set.takeback(kind)) {
        let referenced = set.parts(kind).iter().flatten().filter(|part| part.record.number == number);
        offer.take(copy.as_ref(), referenced.map(|part| part.record.mark.generation));
    }
}

fn take_back_intents(own: u8, now_ms: u32, reads: RoundReads<'_>) -> Vec<ClusterRecordTakeBackCommand> {
    let first = *cluster_block(own).start();
    let mut intents = Vec::new();
    for kind in CLUSTER_RECORD_KINDS {
        for (number, offer) in (first..).zip(offers(own, now_ms, reads.info, kind)) {
            intents.extend(intent(reads.records, kind, number, offer));
        }
    }
    intents
}

pub(crate) fn intent(
    records: &dyn ClusterRecordReadPort,
    kind: ClusterRecordKind,
    number: u8,
    offer: Offer,
) -> Option<ClusterRecordTakeBackCommand> {
    let memory = records.cluster_generation(kind, number);
    let wanted = match (records.cluster_record(kind, number), &offer.best) {
        (Some(live), Some(best)) => best.mark == live.mark && best.revision > live.revision,
        (Some(_), None) => false,
        (None, best) => {
            let adopts = best.as_ref().is_some_and(|best| best.mark.generation > memory && best.mark.generation >= offer.floor);
            adopts || offer.floor > memory
        }
    };
    let record = offer.best.map(|best| ClusterTakenRecord { mark: best.mark, revision: best.revision, name: best.name });
    wanted.then_some(ClusterRecordTakeBackCommand { kind, number, generation_floor: offer.floor, record })
}
