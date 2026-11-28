use core::ptr;

use crate::frame::PageMeta;
use crate::presence::{in_reach, HALF_CLOCK_MS};

use super::set::LampSet;

const BUFFERS: usize = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LampTally {
    pub unheld: bool,
    pub stale: bool,
    pub abandoned: bool,
    pub taken: bool,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) set: u32,
    pub(crate) heard_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Staging {
    set: u32,
    next_page: u8,
}

pub struct Staged<S> {
    buffers: [S; BUFFERS],
    shown_at: usize,
    shown: Option<Shown>,
    staging: Option<Staging>,
}

const fn later(set: u32, than: u32) -> bool {
    let ahead = set.wrapping_sub(than);
    ahead != 0 && ahead < HALF_CLOCK_MS
}

impl<S: LampSet> Staged<S> {
    /// # Safety
    /// `place` is a place of `Self` that nothing reads before this call writes every field once.
    pub(crate) unsafe fn init(place: *mut Self) {
        // SAFETY: both buffers are initialised in place by `LampSet::init`, then the three indices are written.
        unsafe {
            let buffers = ptr::addr_of_mut!((*place).buffers).cast::<S>();
            for index in 0..BUFFERS {
                S::init(buffers.add(index));
            }
            ptr::addr_of_mut!((*place).shown_at).write(0);
            ptr::addr_of_mut!((*place).shown).write(None);
            ptr::addr_of_mut!((*place).staging).write(None);
        }
    }

    const fn staging_at(&self) -> usize {
        (self.shown_at + 1) % BUFFERS
    }

    pub(crate) fn shown(&self) -> Option<&S> {
        self.shown.and(self.buffers.get(self.shown_at))
    }

    pub(crate) const fn shown_mark(&self) -> Option<Shown> {
        self.shown
    }

    fn admits(&self, set: u32) -> bool {
        let after_staging = self.staging.is_none_or(|staging| later(set, staging.set));
        self.shown.is_none_or(|shown| later(set, shown.set)) && after_staging
    }

    fn open(&mut self, set: u32) -> Option<bool> {
        if !self.admits(set) {
            return None;
        }
        let abandoned = self.staging.take().is_some();
        let at = self.staging_at();
        self.buffers.get_mut(at).into_iter().for_each(S::clear);
        self.staging = Some(Staging { set, next_page: 0 });
        Some(abandoned)
    }

    pub(crate) fn take_page(&mut self, meta: PageMeta, now_ms: u32, fill: impl FnOnce(&mut S)) -> LampTally {
        let mut tally = LampTally::default();
        if meta.page == 0 {
            let Some(abandoned) = self.open(meta.set) else {
                return LampTally { stale: true, ..tally };
            };
            tally.abandoned = abandoned;
        }
        let Some(staging) = self.staging.filter(|staging| staging.set == meta.set && staging.next_page == meta.page)
        else {
            tally.abandoned |= self.staging.take_if(|staging| staging.set == meta.set).is_some();
            return LampTally { stale: true, ..tally };
        };
        let at = self.staging_at();
        if let Some(buffer) = self.buffers.get_mut(at) {
            fill(buffer);
        }
        self.advance(staging, meta.last, now_ms, &mut tally);
        tally
    }

    fn advance(&mut self, staging: Staging, last: bool, now_ms: u32, tally: &mut LampTally) {
        if !last {
            let next = staging.next_page.checked_add(1);
            self.staging = next.map(|next_page| Staging { next_page, ..staging });
            tally.abandoned |= next.is_none();
            return;
        }
        let incoming = self.staging_at();
        tally.changed = self.shown().is_none_or(|shown| self.buffers.get(incoming) != Some(shown));
        tally.taken = true;
        self.shown_at = incoming;
        self.shown = Some(Shown { set: staging.set, heard_ms: now_ms });
        self.staging = None;
    }

    pub(crate) fn forget(&mut self) -> bool {
        self.staging = None;
        self.shown.take().is_some()
    }

    pub(crate) fn hold_in_reach(&mut self, now_ms: u32) {
        if let Some(shown) = self.shown.as_mut() {
            shown.heard_ms = in_reach(now_ms, shown.heard_ms);
        }
    }
}
