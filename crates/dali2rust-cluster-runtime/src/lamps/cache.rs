use core::mem::MaybeUninit;
use core::ptr;

use dali2rust_bsp::psram::PsramBox;

use crate::frame::{LampInfoPage, LampStatePage, PageMeta, NODE_COUNT};

use super::set::{LampInfoSet, StateSet};
use super::staged::{LampTally, Staged};
use super::LAMP_ADAPTERS;

const NODES: usize = NODE_COUNT as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampArrival {
    pub node: u8,
    pub mac: [u8; 6],
    pub epoch: u32,
    pub now_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) mac: [u8; 6],
    pub(crate) epoch: u32,
}

pub struct NodeLamps {
    pub(crate) run: Option<Run>,
    pub(crate) state: Staged<StateSet>,
    pub(crate) info: Staged<LampInfoSet>,
}

pub struct LampCache {
    nodes: [NodeLamps; NODES],
    changes: u32,
}

impl NodeLamps {
    fn forget(&mut self) -> bool {
        self.run = None;
        let state = self.state.forget();
        self.info.forget() || state
    }

    fn enter(&mut self, at: LampArrival) -> bool {
        let run = Run { mac: at.mac, epoch: at.epoch };
        if self.run == Some(run) {
            return false;
        }
        let dropped = self.forget();
        self.run = Some(run);
        dropped
    }
}

impl LampCache {
    fn arrive(&mut self, at: LampArrival, adapter: u8) -> Option<&mut NodeLamps> {
        if adapter >= LAMP_ADAPTERS {
            return None;
        }
        let node = self.nodes.get_mut(usize::from(at.node))?;
        if node.enter(at) {
            self.changes = self.changes.wrapping_add(1);
        }
        Some(node)
    }

    pub fn take_state(&mut self, at: LampArrival, page: &LampStatePage<'_>) -> LampTally {
        let Some(node) = self.arrive(at, page.scope().adapter) else {
            return LampTally { unheld: true, ..LampTally::default() };
        };
        let tally = node.state.take_page(page.meta(), at.now_ms, |set| page.elements().for_each(|e| set.take(&e)));
        self.count(tally)
    }

    pub fn take_info(&mut self, at: LampArrival, page: &LampInfoPage<'_>) -> LampTally {
        let Some(node) = self.arrive(at, page.scope().adapter) else {
            return LampTally { unheld: true, ..LampTally::default() };
        };
        let meta: PageMeta = page.meta();
        let tally = node.info.take_page(meta, at.now_ms, |set| page.elements().for_each(|e| set.take(&e)));
        self.count(tally)
    }

    fn count(&mut self, tally: LampTally) -> LampTally {
        if tally.changed {
            self.changes = self.changes.wrapping_add(1);
        }
        tally
    }

    pub fn forget(&mut self, node: u8) {
        if self.nodes.get_mut(usize::from(node)).is_some_and(NodeLamps::forget) {
            self.changes = self.changes.wrapping_add(1);
        }
    }

    pub fn note_run(&mut self, at: LampArrival) {
        let Some(node) = self.nodes.get_mut(usize::from(at.node)) else {
            return;
        };
        if node.run.is_some_and(|run| run.mac == at.mac && run.epoch != at.epoch) && node.forget() {
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
        for node in &mut self.nodes {
            node.state.hold_in_reach(now_ms);
            node.info.hold_in_reach(now_ms);
        }
    }

    pub fn node(&self, node: u8) -> Option<&NodeLamps> {
        self.nodes.get(usize::from(node)).filter(|lamps| lamps.run.is_some())
    }
}

pub(crate) fn placed_lamp_cache() -> PsramBox<LampCache> {
    // SAFETY: every field of every node is written once below: the run, both staged pairs in place, then `changes`.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<LampCache>| {
            let cache = place.as_mut_ptr();
            let nodes = ptr::addr_of_mut!((*cache).nodes).cast::<NodeLamps>();
            for index in 0..NODES {
                let node = nodes.add(index);
                ptr::addr_of_mut!((*node).run).write(None);
                Staged::init(ptr::addr_of_mut!((*node).state));
                Staged::init(ptr::addr_of_mut!((*node).info));
            }
            ptr::addr_of_mut!((*cache).changes).write(0);
        })
    }
}
