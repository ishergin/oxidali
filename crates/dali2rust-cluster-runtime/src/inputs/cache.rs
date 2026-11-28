use core::mem::MaybeUninit;
use core::ptr;

use dali2rust_bsp::psram::PsramBox;

use crate::frame::{InputEvent, InputInfoPage, PageMeta, NODE_COUNT};
use crate::info::SetId;
use crate::presence::{in_reach, HALF_CLOCK_MS};

use super::set::{InputSet, Received};

const NODES: usize = NODE_COUNT as usize;

const SETS_PER_NODE: usize = 2;

pub(crate) const REPLAYED_EVENTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputArrival {
    pub node: u8,
    pub mac: [u8; 6],
    pub epoch: u32,
    pub now_ms: u32,
}


#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputTally {
    pub stale: bool,
    pub abandoned: bool,
    pub taken: bool,
    pub changed: bool,
    pub skipped: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputHeard {
    Applied,
    Older,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Applied {
    mac: [u8; 6],
    epoch: u32,
    seq: u32,
}

const fn after(seq: u32, than: u32) -> bool {
    let ahead = seq.wrapping_sub(than);
    ahead != 0 && ahead < HALF_CLOCK_MS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Recent {
    event: InputEvent,
    heard_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) id: SetId,
    pub(crate) heard_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Staging {
    id: SetId,
    next_page: u8,
}

pub struct NodeInputs {
    sets: [InputSet; SETS_PER_NODE],
    shown_at: usize,
    pub(crate) shown: Option<Shown>,
    staging: Option<Staging>,
    applied: Option<Applied>,
    recent: [Option<Recent>; REPLAYED_EVENTS],
    recent_next: usize,
}

impl NodeInputs {
    const fn staging_at(&self) -> usize {
        (self.shown_at + 1) % SETS_PER_NODE
    }

    pub(crate) fn shown_set(&self) -> Option<&InputSet> {
        self.shown.and(self.sets.get(self.shown_at))
    }

    fn admits_first_page(&self, id: SetId) -> bool {
        let after_staging = self.staging.is_none_or(|staging| id.follows(staging.id));
        let after_shown = self.shown.is_none_or(|shown| id.follows(shown.id));
        after_staging && after_shown
    }

    fn open(&mut self, id: SetId, tally: &mut InputTally) {
        tally.abandoned = self.staging.take().is_some();
        let at = self.staging_at();
        if let Some(set) = self.sets.get_mut(at) {
            set.clear();
        }
        self.staging = Some(Staging { id, next_page: 0 });
    }

    fn take_page(&mut self, at: InputArrival, page: &InputInfoPage<'_>) -> InputTally {
        let meta = page.meta();
        let id = SetId { mac: at.mac, epoch: at.epoch, set: meta.set };
        let mut tally = InputTally::default();
        if meta.page == 0 {
            if !self.admits_first_page(id) {
                return InputTally { stale: true, ..tally };
            }
            self.open(id, &mut tally);
        }
        let Some(staging) = self.staging.filter(|staging| staging.id == id && staging.next_page == meta.page) else {
            tally.abandoned |= self.staging.take_if(|staging| staging.id == id).is_some();
            return InputTally { stale: true, ..tally };
        };
        tally.skipped = self.stage(page, Received { now_ms: at.now_ms });
        self.advance(staging, meta, at.now_ms, &mut tally);
        tally
    }

    fn stage(&mut self, page: &InputInfoPage<'_>, received: Received) -> u16 {
        let at = self.staging_at();
        let Some(set) = self.sets.get_mut(at) else {
            return 0;
        };
        let mut skipped = 0_u16;
        for element in page.elements() {
            skipped = skipped.saturating_add(u16::from(!set.take(&element, received)));
        }
        set.skipped = set.skipped.saturating_add(skipped);
        skipped
    }

    fn advance(&mut self, staging: Staging, meta: PageMeta, now_ms: u32, tally: &mut InputTally) {
        if !meta.last {
            self.staging = staging.next_page.checked_add(1).map(|next_page| Staging { next_page, ..staging });
            tally.abandoned |= self.staging.is_none();
            return;
        }
        let incoming = self.staging_at();
        let same_run = self.shown.is_some_and(|shown| shown.id.mac == staging.id.mac && shown.id.epoch == staging.id.epoch);
        if let ([first, second], true) = (&mut self.sets, same_run) {
            let (staged, shown) = if incoming == 0 { (first, &*second) } else { (second, &*first) };
            staged.keep_newer_from(shown, now_ms);
        }
        self.replay_recent(staging.id, incoming, now_ms);
        tally.changed = self.shown_set().is_none_or(|shown| self.sets.get(incoming).is_some_and(|set| !set.same_but_values(shown)));
        tally.taken = true;
        self.shown_at = incoming;
        self.shown = Some(Shown { id: staging.id, heard_ms: now_ms });
        self.staging = None;
    }

    fn replay_recent(&mut self, id: SetId, incoming: usize, now_ms: u32) {
        let of_the_set = self.applied.is_some_and(|applied| applied.mac == id.mac && applied.epoch == id.epoch);
        let Some(set) = self.sets.get_mut(incoming).filter(|_| of_the_set) else {
            return;
        };
        let oldest_first = self.recent.iter().cycle().skip(self.recent_next).take(REPLAYED_EVENTS);
        for recent in oldest_first.flatten() {
            set.replay_event(&recent.event, recent.heard_ms, now_ms);
        }
    }

    fn admit_event(&mut self, at: InputArrival, seq: u32) -> InputHeard {
        let same_run = |applied: &Applied| applied.mac == at.mac && applied.epoch == at.epoch;
        if self.applied.is_some_and(|applied| same_run(&applied) && !after(seq, applied.seq)) {
            return InputHeard::Older;
        }
        if !self.applied.is_some_and(|applied| same_run(&applied)) {
            self.recent = [None; REPLAYED_EVENTS];
        }
        self.applied = Some(Applied { mac: at.mac, epoch: at.epoch, seq });
        InputHeard::Applied
    }

    fn remember(&mut self, event: &InputEvent, heard_ms: u32) {
        if let Some(place) = self.recent.get_mut(self.recent_next) {
            *place = Some(Recent { event: *event, heard_ms });
        }
        self.recent_next = (self.recent_next + 1) % REPLAYED_EVENTS;
    }

    fn hear_event(&mut self, at: InputArrival, event: &InputEvent) -> bool {
        let staging = self.staging_at();
        let shown_at = self.shown_at;
        let shown_run = self.shown.filter(|shown| shown.id.mac == at.mac).is_some();
        let staging_run = self.staging.filter(|staging| staging.id.mac == at.mac).is_some();
        let mut heard = false;
        for (index, set) in self.sets.iter_mut().enumerate() {
            if (index == shown_at && shown_run) || (index == staging && staging_run) {
                heard |= set.hear_event(event, at.now_ms);
            }
        }
        heard
    }

    fn forget(&mut self) -> bool {
        self.staging = None;
        self.recent = [None; REPLAYED_EVENTS];
        self.shown.take().is_some()
    }

    fn note_run(&mut self, at: InputArrival) -> bool {
        let earlier = |id: &SetId| id.mac == at.mac && id.epoch != at.epoch;
        self.staging.take_if(|staging| earlier(&staging.id));
        self.shown.take_if(|shown| earlier(&shown.id)).is_some()
    }

    fn hold_in_reach(&mut self, now_ms: u32) {
        if let Some(shown) = self.shown.as_mut() {
            shown.heard_ms = in_reach(now_ms, shown.heard_ms);
        }
        self.sets.iter_mut().for_each(|set| set.hold_in_reach(now_ms));
        for recent in self.recent.iter_mut().flatten() {
            recent.heard_ms = in_reach(now_ms, recent.heard_ms);
        }
    }
}

pub struct InputCache {
    nodes: [NodeInputs; NODES],
    changes: u32,
}

impl InputCache {
    pub fn take_page(&mut self, at: InputArrival, page: &InputInfoPage<'_>) -> InputTally {
        let Some(node) = self.nodes.get_mut(usize::from(at.node)) else {
            return InputTally { stale: true, ..InputTally::default() };
        };
        let tally = node.take_page(at, page);
        if tally.changed {
            self.changes = self.changes.wrapping_add(1);
        }
        tally
    }

    pub fn hear_event(&mut self, at: InputArrival, seq: u32, event: &InputEvent) -> InputHeard {
        let Some(node) = self.nodes.get_mut(usize::from(at.node)) else {
            return InputHeard::Older;
        };
        let heard = node.admit_event(at, seq);
        if heard == InputHeard::Applied {
            node.hear_event(at, event);
            node.remember(event, at.now_ms);
        }
        heard
    }

    pub fn forget(&mut self, node: u8) {
        if self.nodes.get_mut(usize::from(node)).is_some_and(NodeInputs::forget) {
            self.changes = self.changes.wrapping_add(1);
        }
    }

    pub fn note_run(&mut self, at: InputArrival) {
        if self.nodes.get_mut(usize::from(at.node)).is_some_and(|node| node.note_run(at)) {
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

    pub fn node(&self, node: u8) -> Option<&NodeInputs> {
        self.nodes.get(usize::from(node)).filter(|inputs| inputs.shown.is_some())
    }
}

pub(crate) fn placed_input_cache() -> PsramBox<InputCache> {
    // SAFETY: every field of every node is written once below: both sets in place, then the marks and `changes`.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<InputCache>| {
            let cache = place.as_mut_ptr();
            let nodes = ptr::addr_of_mut!((*cache).nodes).cast::<NodeInputs>();
            for index in 0..NODES {
                init_node(nodes.add(index));
            }
            ptr::addr_of_mut!((*cache).changes).write(0);
        })
    }
}

unsafe fn init_node(node: *mut NodeInputs) {
    // SAFETY: the caller hands a place of `NodeInputs`; each field is written once and nothing is read.
    unsafe {
        let sets = ptr::addr_of_mut!((*node).sets).cast::<InputSet>();
        for index in 0..SETS_PER_NODE {
            InputSet::init(sets.add(index));
        }
        ptr::addr_of_mut!((*node).shown_at).write(0);
        ptr::addr_of_mut!((*node).shown).write(None);
        ptr::addr_of_mut!((*node).staging).write(None);
        ptr::addr_of_mut!((*node).applied).write(None);
        ptr::addr_of_mut!((*node).recent).write([None; REPLAYED_EVENTS]);
        ptr::addr_of_mut!((*node).recent_next).write(0);
    }
}
