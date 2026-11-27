use dali2rust_contracts::msg::{fixed_text_32, fixed_text_64, FixedText32, FixedText64};

use crate::frame::{Header, Hello, NodeClaim, Pulse};
use crate::sequence::{Arrival, SequenceWindow};

use super::timing::{elapsed_ms, in_reach, PresenceTiming};

pub const MAX_HEARD_BOARDS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PulseFacts {
    pub cluster_name: FixedText64,
    pub installation: FixedText32,
    pub claim: NodeClaim,
    pub wire: u8,
    pub wire_protection: bool,
}

impl PulseFacts {
    fn of(pulse: &Pulse<'_>) -> Self {
        Self {
            cluster_name: fixed_text_64(pulse.cluster_name),
            installation: fixed_text_32(pulse.installation),
            claim: pulse.claim,
            wire: pulse.wire,
            wire_protection: pulse.wire_protection,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Arriving<'a> {
    pub header: &'a Header,
    pub source: [u8; 4],
    pub now_ms: u32,
    pub timing: PresenceTiming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeardBoard {
    pub mac: [u8; 6],
    pub source: [u8; 4],
    pub node_no: u8,
    pub pulse: PulseFacts,
    pub ha_face: Option<bool>,
    pub holding_since_ms: Option<u32>,
    pub last_heard_ms: u32,
    pub sequence_gaps: u32,
    window: SequenceWindow,
}

impl HeardBoard {
    fn first(at: Arriving<'_>, pulse: &Pulse<'_>) -> Self {
        let pulse = PulseFacts::of(pulse);
        let holding = matches!(pulse.claim, NodeClaim::Holding);
        Self {
            mac: at.header.sender_mac,
            source: at.source,
            node_no: at.header.node_no,
            pulse,
            ha_face: None,
            holding_since_ms: holding.then_some(at.now_ms),
            last_heard_ms: at.now_ms,
            sequence_gaps: 0,
            window: SequenceWindow::starting_at(at.header.boot_epoch, at.header.seq),
        }
    }

    pub const fn holds(&self) -> bool {
        matches!(self.pulse.claim, NodeClaim::Holding)
    }

    pub const fn age_ms(&self, now_ms: u32) -> u32 {
        elapsed_ms(now_ms, self.last_heard_ms)
    }

    pub const fn is_live(&self, now_ms: u32, timing: PresenceTiming) -> bool {
        self.age_ms(now_ms) <= timing.liveness_ms()
    }

    fn hold_in_reach(&mut self, now_ms: u32) {
        self.last_heard_ms = in_reach(now_ms, self.last_heard_ms);
        self.holding_since_ms = self.holding_since_ms.map(|since_ms| in_reach(now_ms, since_ms));
    }

    fn admit(&mut self, header: &Header) -> Taken {
        let taken = Taken::of(self.window.admit(header.boot_epoch, header.seq));
        match taken {
            Taken::Fresh { skipped } => self.sequence_gaps = self.sequence_gaps.wrapping_add(skipped),
            Taken::Restarted => self.holding_since_ms = None,
            _ => {}
        }
        taken
    }

    fn take_pulse(&mut self, at: Arriving<'_>, pulse: &Pulse<'_>) {
        let run_goes_on = self.holding_since_ms.is_some()
            && self.node_no == at.header.node_no
            && self.is_live(at.now_ms, at.timing);
        self.pulse = PulseFacts::of(pulse);
        self.holding_since_ms = match (self.holds(), run_goes_on) {
            (false, _) => None,
            (true, true) => self.holding_since_ms,
            (true, false) => Some(at.now_ms),
        };
        self.source = at.source;
        self.node_no = at.header.node_no;
        self.last_heard_ms = at.now_ms;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    First { evicted: bool },
    Fresh { skipped: u32 },
    Restarted,
    Late,
    Repeated,
    Unknown,
}

impl Taken {
    const fn of(arrival: Arrival) -> Self {
        match arrival {
            Arrival::Fresh { skipped } => Self::Fresh { skipped },
            Arrival::Restarted => Self::Restarted,
            Arrival::Late => Self::Late,
            Arrival::Duplicate | Arrival::TooOld => Self::Repeated,
        }
    }

    pub const fn applies(self) -> bool {
        matches!(self, Self::Fresh { .. } | Self::Restarted)
    }

    pub const fn admits_command(self) -> bool {
        matches!(self, Self::Fresh { .. } | Self::Restarted | Self::Late)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardTable {
    pub(crate) slots: [Option<HeardBoard>; MAX_HEARD_BOARDS],
}

impl Default for BoardTable {
    fn default() -> Self {
        Self { slots: [const { None }; MAX_HEARD_BOARDS] }
    }
}

impl BoardTable {
    pub fn iter(&self) -> impl Iterator<Item = &HeardBoard> {
        self.slots.iter().flatten()
    }

    pub fn take_pulse(&mut self, at: Arriving<'_>, pulse: &Pulse<'_>) -> Taken {
        let Some(board) = self.find_mut(at.header.sender_mac) else {
            return self.insert(HeardBoard::first(at, pulse), at.now_ms);
        };
        let taken = board.admit(at.header);
        if taken.applies() {
            board.take_pulse(at, pulse);
        }
        taken
    }

    pub fn take_hello(&mut self, header: &Header, hello: &Hello) -> Taken {
        let Some(board) = self.find_mut(header.sender_mac) else {
            return Taken::Unknown;
        };
        let taken = board.admit(header);
        if taken.applies() {
            board.ha_face = Some(hello.ha_face);
        }
        taken
    }

    pub fn take_other(&mut self, header: &Header) -> Taken {
        self.find_mut(header.sender_mac).map_or(Taken::Unknown, |board| board.admit(header))
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&HeardBoard) -> bool) -> usize {
        let mut removed = 0;
        for slot in &mut self.slots {
            if slot.as_ref().is_some_and(|board| !keep(board)) {
                *slot = None;
                removed += 1;
            }
        }
        removed
    }

    pub fn clear(&mut self) {
        self.retain(|_| false);
    }

    pub fn hold_in_reach(&mut self, now_ms: u32) {
        for board in self.slots.iter_mut().flatten() {
            board.hold_in_reach(now_ms);
        }
    }

    pub fn marks(&self, mut mark: impl FnMut(&HeardBoard) -> bool) -> [bool; MAX_HEARD_BOARDS] {
        let mut marks = [false; MAX_HEARD_BOARDS];
        for (marked, slot) in marks.iter_mut().zip(&self.slots) {
            *marked = slot.as_ref().is_some_and(&mut mark);
        }
        marks
    }

    pub fn drop_marked(&mut self, marks: [bool; MAX_HEARD_BOARDS]) -> usize {
        let mut dropped = 0;
        for (slot, marked) in self.slots.iter_mut().zip(marks) {
            if marked && slot.take().is_some() {
                dropped += 1;
            }
        }
        dropped
    }

    fn find_mut(&mut self, mac: [u8; 6]) -> Option<&mut HeardBoard> {
        self.slots.iter_mut().flatten().find(|board| board.mac == mac)
    }

    fn insert(&mut self, board: HeardBoard, now_ms: u32) -> Taken {
        let (index, evicted) = self.free_or_stalest(now_ms);
        self.slots[index] = Some(board);
        Taken::First { evicted }
    }

    fn free_or_stalest(&self, now_ms: u32) -> (usize, bool) {
        if let Some(free) = self.slots.iter().position(Option::is_none) {
            return (free, false);
        }
        let age = |slot: &Option<HeardBoard>| slot.as_ref().map_or(0, |board| board.age_ms(now_ms));
        let stalest = (0..MAX_HEARD_BOARDS).max_by_key(|&index| age(&self.slots[index]));
        (stalest.unwrap_or(0), true)
    }
}
