use core::mem::MaybeUninit;
use core::ptr;

use dali2rust_bsp::psram::PsramBox;
use dali2rust_contracts::msg::ClusterRecordKind;

use crate::frame::{InfoPage, PageMeta, NODE_COUNT};
use crate::presence::{elapsed_ms, in_reach, HALF_CLOCK_MS};

use super::set::InfoSet;

const NODES: usize = NODE_COUNT as usize;

const SETS_PER_NODE: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetId {
    pub mac: [u8; 6],
    pub epoch: u32,
    pub set: u32,
}

impl SetId {
    fn follows(self, held: Self) -> bool {
        let same_run = self.mac == held.mac && self.epoch == held.epoch;
        let ahead = self.set.wrapping_sub(held.set);
        !same_run || (ahead != 0 && ahead < HALF_CLOCK_MS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrival {
    pub node: u8,
    pub own_node: u8,
    pub mac: [u8; 6],
    pub epoch: u32,
    pub now_ms: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageTally {
    pub stale: bool,
    pub abandoned: bool,
    pub taken: bool,
    pub changed: bool,
    pub skipped: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shown {
    id: SetId,
    heard_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Staging {
    id: SetId,
    next_page: u8,
}

pub struct NodeInfo {
    sets: [InfoSet; SETS_PER_NODE],
    shown_at: usize,
    shown: Option<Shown>,
    staging: Option<Staging>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldNode {
    pub mac: [u8; 6],
    pub set: u32,
    pub age_ms: u32,
    pub skipped: u16,
}

pub struct InfoCache {
    nodes: [NodeInfo; NODES],
    changes: u32,
}

impl NodeInfo {
    const fn staging_at(&self) -> usize {
        (self.shown_at + 1) % SETS_PER_NODE
    }

    fn shown_set(&self) -> Option<&InfoSet> {
        self.shown.and(self.sets.get(self.shown_at))
    }

    fn admits_first_page(&self, id: SetId) -> bool {
        let after_staging = self.staging.is_none_or(|staging| id.follows(staging.id));
        let after_shown = self.shown.is_none_or(|shown| id.follows(shown.id));
        after_staging && after_shown
    }

    fn open(&mut self, id: SetId, (node, own): (u8, u8), tally: &mut PageTally) {
        tally.abandoned = self.staging.take().is_some();
        let at = self.staging_at();
        if let Some(set) = self.sets.get_mut(at) {
            set.clear_for(node, own);
        }
        self.staging = Some(Staging { id, next_page: 0 });
    }

    fn take_page(&mut self, at: Arrival, page: &InfoPage<'_>) -> PageTally {
        let meta = page.meta();
        let id = SetId { mac: at.mac, epoch: at.epoch, set: meta.set };
        let mut tally = PageTally::default();
        if meta.page == 0 {
            if !self.admits_first_page(id) {
                return PageTally { stale: true, ..tally };
            }
            self.open(id, (at.node, at.own_node), &mut tally);
        }
        let Some(staging) = self.staging.filter(|staging| staging.id == id && staging.next_page == meta.page) else {
            tally.stale = true;
            tally.abandoned |= self.staging.take_if(|staging| staging.id == id).is_some();
            return tally;
        };
        tally.skipped = self.stage(page);
        self.advance(staging, meta, at.now_ms, &mut tally);
        tally
    }

    fn stage(&mut self, page: &InfoPage<'_>) -> u16 {
        let at = self.staging_at();
        let Some(set) = self.sets.get_mut(at) else {
            return 0;
        };
        let mut skipped = 0_u16;
        for element in page.elements() {
            skipped = skipped.saturating_add(u16::from(set.take(&element).skipped));
        }
        skipped
    }

    fn advance(&mut self, staging: Staging, meta: PageMeta, now_ms: u32, tally: &mut PageTally) {
        if !meta.last {
            self.staging = staging.next_page.checked_add(1).map(|next_page| Staging { next_page, ..staging });
            tally.abandoned |= self.staging.is_none();
            return;
        }
        let incoming = self.staging_at();
        tally.changed = self.shown_set().is_none_or(|shown| self.sets.get(incoming) != Some(shown));
        tally.taken = true;
        self.shown_at = incoming;
        self.shown = Some(Shown { id: staging.id, heard_ms: now_ms });
        self.staging = None;
    }

    fn forget(&mut self) -> bool {
        self.staging = None;
        self.shown.take().is_some()
    }

    fn hold_in_reach(&mut self, now_ms: u32) {
        if let Some(shown) = self.shown.as_mut() {
            shown.heard_ms = in_reach(now_ms, shown.heard_ms);
        }
    }
}

impl InfoCache {
    pub fn take_page(&mut self, at: Arrival, page: &InfoPage<'_>) -> PageTally {
        let Some(node) = self.nodes.get_mut(usize::from(at.node)) else {
            return PageTally { stale: true, ..PageTally::default() };
        };
        let tally = node.take_page(at, page);
        if tally.changed {
            self.changes = self.changes.wrapping_add(1);
        }
        tally
    }

    pub fn forget(&mut self, node: u8) {
        if self.nodes.get_mut(usize::from(node)).is_some_and(NodeInfo::forget) {
            self.changes = self.changes.wrapping_add(1);
        }
    }

    pub fn clear(&mut self) {
        let dropped = self.nodes.iter_mut().fold(false, |dropped, node| node.forget() || dropped);
        if dropped {
            self.changes = self.changes.wrapping_add(1);
        }
    }

    pub const fn changes(&self) -> u32 {
        self.changes
    }

    pub fn hold_in_reach(&mut self, now_ms: u32) {
        self.nodes.iter_mut().for_each(|node| node.hold_in_reach(now_ms));
    }

    pub fn shown(&self, node: u8) -> Option<&InfoSet> {
        self.nodes.get(usize::from(node))?.shown_set()
    }

    pub fn held(&self, node: u8, now_ms: u32) -> Option<HeldNode> {
        let info = self.nodes.get(usize::from(node))?;
        let shown = info.shown?;
        let skipped = info.shown_set().map_or(0, |set| set.skipped);
        Some(HeldNode { mac: shown.id.mac, set: shown.id.set, age_ms: elapsed_ms(now_ms, shown.heard_ms), skipped })
    }

    pub fn counts(&self, node: u8, kind: ClusterRecordKind) -> Option<(u8, u8)> {
        self.shown(node).map(|set| set.count(kind))
    }
}

pub(crate) fn placed_cache() -> PsramBox<InfoCache> {
    // SAFETY: every field of every node is written once below: both sets in place, then the indices and `changes`.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<InfoCache>| {
            let cache = place.as_mut_ptr();
            let nodes = ptr::addr_of_mut!((*cache).nodes).cast::<NodeInfo>();
            for index in 0..NODES {
                init_node(nodes.add(index));
            }
            ptr::addr_of_mut!((*cache).changes).write(0);
        })
    }
}

pub(crate) fn placed_set() -> PsramBox<InfoSet> {
    // SAFETY: `InfoSet::init` writes every field of the place once.
    unsafe { PsramBox::new_with(|place: &mut MaybeUninit<InfoSet>| InfoSet::init(place.as_mut_ptr())) }
}

unsafe fn init_node(node: *mut NodeInfo) {
    // SAFETY: the caller hands a place of `NodeInfo`; each field is written once and nothing is read.
    unsafe {
        let sets = ptr::addr_of_mut!((*node).sets).cast::<InfoSet>();
        for index in 0..SETS_PER_NODE {
            InfoSet::init(sets.add(index));
        }
        ptr::addr_of_mut!((*node).shown_at).write(0);
        ptr::addr_of_mut!((*node).shown).write(None);
        ptr::addr_of_mut!((*node).staging).write(None);
    }
}
