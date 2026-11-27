use core::cmp::Reverse;

use dali2rust_contracts::msg::{FixedText32, FixedText64};

use crate::frame::NODE_COUNT;

use super::board::{BoardTable, HeardBoard};
use super::timing::{elapsed_ms, later, PresenceTiming};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnFacts {
    pub cluster_name: FixedText64,
    pub cluster_tag: u32,
    pub installation: FixedText32,
    pub mac: [u8; 6],
    pub node_no: u8,
    pub wire: u8,
    pub running_since_ms: Option<u32>,
}

impl OwnFacts {
    const fn holds(&self, node: u8) -> bool {
        self.running_since_ms.is_some() && self.node_no == node
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IgnoredReason {
    OtherCluster,
    Waiting,
    NumberHeld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Standing {
    Holder,
    Ignored(IgnoredReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WireOwner {
    Nobody,
    ThisNode,
    Board { node: u8, mac: [u8; 6] },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OwnFaults {
    pub split_pair: bool,
    pub wire_chain: bool,
}

const NODES: usize = NODE_COUNT as usize;

#[derive(Clone, Copy)]
pub struct Roster<'a> {
    pub table: &'a BoardTable,
    pub own: &'a OwnFacts,
    pub now_ms: u32,
    pub timing: PresenceTiming,
    winners: [Option<&'a FixedText32>; NODES],
}

impl<'a> Roster<'a> {
    pub fn new(table: &'a BoardTable, own: &'a OwnFacts, now_ms: u32, timing: PresenceTiming) -> Self {
        let mut roster = Self { table, own, now_ms, timing, winners: [None; NODES] };
        for node in 0..NODE_COUNT {
            roster.winners[usize::from(node)] = roster.winner(node);
        }
        roster
    }

    pub fn live(&self, board: &HeardBoard) -> bool {
        board.is_live(self.now_ms, self.timing)
    }

    fn member(&self, board: &HeardBoard) -> bool {
        board.pulse.cluster_name == self.own.cluster_name
    }

    fn live_holding_members(&self, node: u8) -> impl Iterator<Item = &'a HeardBoard> + '_ {
        self.table.iter().filter(move |board| {
            board.node_no == node && board.holds() && self.member(board) && self.live(board)
        })
    }

    fn winner(&self, node: u8) -> Option<&'a FixedText32> {
        let boards = self.live_holding_members(node).map(|board| (board.mac, &board.pulse.installation));
        let own = self.own.holds(node).then_some((self.own.mac, &self.own.installation));
        boards.chain(own).min_by_key(|(mac, _)| *mac).map(|(_, installation)| installation)
    }

    pub fn standing(&self, board: &HeardBoard) -> Standing {
        if !self.member(board) {
            return Standing::Ignored(IgnoredReason::OtherCluster);
        }
        if !board.holds() {
            return Standing::Ignored(IgnoredReason::Waiting);
        }
        match self.winners.get(usize::from(board.node_no)).copied().flatten() {
            Some(installation) if *installation != board.pulse.installation => {
                Standing::Ignored(IgnoredReason::NumberHeld)
            }
            _ => Standing::Holder,
        }
    }

    fn holders(&self, node: u8) -> impl Iterator<Item = &'a HeardBoard> + '_ {
        self.table
            .iter()
            .filter(move |board| board.node_no == node && self.standing(board) == Standing::Holder)
    }

    fn live_holders(&self, node: u8) -> impl Iterator<Item = &'a HeardBoard> + '_ {
        self.holders(node).filter(move |board| self.live(board))
    }

    pub fn displayed(&self, node: u8) -> Option<&'a HeardBoard> {
        self.holders(node).max_by_key(|board| self.rank(board))
    }

    fn rank(&self, board: &HeardBoard) -> (bool, Reverse<u32>) {
        let live = self.live(board);
        let since = match (live, board.holding_since_ms) {
            (true, Some(run_started)) => run_started,
            _ => board.last_heard_ms,
        };
        (live, Reverse(elapsed_ms(self.now_ms, since)))
    }

    pub fn ignored(&self) -> impl Iterator<Item = (&'a HeardBoard, IgnoredReason)> + '_ {
        self.table.iter().filter(|board| self.live(board)).filter_map(|board| match self.standing(board) {
            Standing::Ignored(reason) => Some((board, reason)),
            Standing::Holder => None,
        })
    }

    pub fn split(&self, node: u8) -> bool {
        let both = |a: &HeardBoard, b: &HeardBoard| a.mac != b.mac && transmitting_together(self.now_ms, a, b);
        let boards = self.live_holders(node).any(|a| self.live_holders(node).any(|b| both(a, b)));
        boards || (self.own.holds(node) && self.split_with_this_node())
    }

    fn split_with_this_node(&self) -> bool {
        let Some(run_started) = self.own.running_since_ms else {
            return false;
        };
        self.live_holders(self.own.node_no).any(|board| {
            board.pulse.installation == self.own.installation
                && later(self.now_ms, board.last_heard_ms, run_started)
        })
    }

    pub fn rival(&self) -> Option<&'a HeardBoard> {
        self.live_holding_members(self.own.node_no)
            .filter(|board| board.pulse.installation != self.own.installation)
            .min_by_key(|board| board.mac)
    }

    fn wire_of(&self, node: u8) -> Option<u8> {
        if node == self.own.node_no {
            return Some(self.own.wire);
        }
        self.displayed(node).filter(|board| self.live(board)).map(|board| board.pulse.wire)
    }

    fn names_a_chain(&self, node: u8, wire: u8) -> bool {
        wire != node && self.wire_of(wire).is_some_and(|named| named != wire)
    }

    fn named_by_others(&self, node: u8) -> bool {
        self.table.iter().any(|board| {
            board.node_no != node
                && board.pulse.wire == node
                && self.live(board)
                && self.standing(board) == Standing::Holder
        })
    }

    fn chained(&self, node: u8, wire: u8) -> bool {
        self.names_a_chain(node, wire) || (wire != node && self.named_by_others(node))
    }

    pub fn faults(&self) -> OwnFaults {
        let own = self.own;
        OwnFaults {
            split_pair: own.holds(own.node_no) && self.split_with_this_node(),
            wire_chain: self.chained(own.node_no, own.wire),
        }
    }

    pub fn wire_owner(&self) -> WireOwner {
        let own = self.own;
        let boards = self.table.iter().filter(|board| self.on_this_wire(board));
        let candidates = boards.map(|board| (board.node_no, board.mac, false));
        let this = own.holds(own.node_no) && !self.faults().wire_chain;
        let all = candidates.chain(this.then_some((own.node_no, own.mac, true)));
        match all.min_by_key(|(node, mac, _)| (*node, *mac)) {
            None => WireOwner::Nobody,
            Some((_, _, true)) => WireOwner::ThisNode,
            Some((node, mac, false)) => WireOwner::Board { node, mac },
        }
    }

    fn on_this_wire(&self, board: &HeardBoard) -> bool {
        board.pulse.wire == self.own.wire
            && self.live(board)
            && self.standing(board) == Standing::Holder
            && !self.chained(board.node_no, board.pulse.wire)
    }

    pub fn keeps(&self, board: &HeardBoard) -> bool {
        if self.live(board) {
            return true;
        }
        let holder = self.standing(board) == Standing::Holder;
        holder && !self.own.holds(board.node_no) && self.live_holders(board.node_no).next().is_none()
    }

    pub fn heard(&self, node: u8) -> bool {
        self.own.holds(node) || self.live_holders(node).next().is_some()
    }
}

fn transmitting_together(now_ms: u32, a: &HeardBoard, b: &HeardBoard) -> bool {
    let (Some(a_started), Some(b_started)) = (a.holding_since_ms, b.holding_since_ms) else {
        return false;
    };
    later(now_ms, a.last_heard_ms, b_started) && later(now_ms, b.last_heard_ms, a_started)
}
