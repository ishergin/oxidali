use core::mem::{self, MaybeUninit};

use dali2rust_bsp::psram::PsramBox;
use dali2rust_domain::registry::{ClusterLampReadPort, ClusterPartReadPort};

use crate::frame::{AdapterScope, DatagramBuffer, EncodeError, Header, LampInfoKind, LampStateKind, PageCursor, NODE_COUNT};
use crate::lamps::{fill_info, fill_state, LampInfoSet, LampSet, StateSet};

use super::announce::{hello_delay_ms, next_phase, reached, Turn, PAGE_GAP_MS};

#[cfg(test)]
mod tests;

pub(crate) const ANNOUNCED_ADAPTER: u8 = 0;

const PHASE_HALVES: u32 = 2;

const QUIET_PARTS_OF_REFRESH: u32 = 2;

#[derive(Clone, Copy)]
pub(crate) struct OwnLamps<'a> {
    pub(crate) lamps: &'a dyn ClusterLampReadPort,
    pub(crate) parts: &'a dyn ClusterPartReadPort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Subject {
    State,
    Info,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Stream {
    moved_at: Option<u32>,
    due_ms: Option<u32>,
    forced: bool,
    announced: bool,
    started_ms: Option<u32>,
}

impl Stream {
    fn schedule(&mut self, at_ms: u32, forced: bool) {
        self.due_ms = Some(match self.due_ms {
            Some(due_ms) if !reached(due_ms, at_ms) => due_ms,
            _ => at_ms,
        });
        self.forced |= forced;
    }

    fn open_window(&mut self, turn: Turn) {
        let window_ms = turn.timing.info_coalesce_ms().saturating_add(PAGE_GAP_MS.saturating_mul(u32::from(turn.node)));
        if let Some(moved_ms) = self.moved_at.take() {
            self.schedule(moved_ms.wrapping_add(window_ms), false);
        }
        if !self.announced && self.due_ms.is_none() {
            self.schedule(turn.now_ms.wrapping_add(window_ms), false);
        }
    }

    fn refresh(&mut self, now_ms: u32, quiet_ms: u32) {
        if self.started_ms.is_some_and(|started_ms| now_ms.wrapping_sub(started_ms) < quiet_ms) {
            return;
        }
        self.schedule(now_ms, true);
    }

    fn take_due(&mut self, now_ms: u32) -> Option<bool> {
        if !self.due_ms.is_some_and(|due_ms| reached(now_ms, due_ms)) {
            return None;
        }
        self.due_ms = None;
        Some(mem::take(&mut self.forced))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Paging {
    subject: Subject,
    cursor: PageCursor,
    next_ms: u32,
}

pub(crate) struct LampAnnouncer {
    state: Stream,
    info: Stream,
    refresh_ms: Option<u32>,
    paging: Option<Paging>,
    next_set: u32,
    state_next: PsramBox<StateSet>,
    state_sent: PsramBox<StateSet>,
    info_next: PsramBox<LampInfoSet>,
    info_sent: PsramBox<LampInfoSet>,
}

impl LampAnnouncer {
    pub(crate) fn new() -> Self {
        Self {
            state: Stream::default(),
            info: Stream::default(),
            refresh_ms: None,
            paging: None,
            next_set: 0,
            state_next: placed(),
            state_sent: placed(),
            info_next: placed(),
            info_sent: placed(),
        }
    }

    pub(crate) fn note_moved(&mut self, subject: Subject, now_ms: u32) {
        let stream = self.stream(subject);
        stream.moved_at.get_or_insert(now_ms);
    }

    const fn stream(&mut self, subject: Subject) -> &mut Stream {
        match subject {
            Subject::State => &mut self.state,
            Subject::Info => &mut self.info,
        }
    }

    pub(crate) fn plan(&mut self, turn: Turn, own: OwnLamps<'_>) {
        if !turn.running {
            self.stand_down();
            return;
        }
        self.state.open_window(turn);
        self.info.open_window(turn);
        if turn.hello {
            let at_ms = turn.now_ms.wrapping_add(hello_delay_ms(turn));
            self.state.schedule(at_ms, true);
            self.info.schedule(at_ms, true);
        }
        let refresh_ms = *self.refresh_ms.get_or_insert_with(|| refresh_phase(turn));
        if reached(turn.now_ms, refresh_ms) {
            let quiet_ms = turn.timing.info_refresh_ms() / QUIET_PARTS_OF_REFRESH;
            self.state.refresh(turn.now_ms, quiet_ms);
            self.info.refresh(turn.now_ms, quiet_ms);
            self.refresh_ms = Some(refresh_phase(turn));
        }
        self.start_due(turn, own);
    }

    fn stand_down(&mut self) {
        self.state = Stream::default();
        self.info = Stream::default();
        self.refresh_ms = None;
        self.paging = None;
    }

    fn start_due(&mut self, turn: Turn, own: OwnLamps<'_>) {
        if self.paging.is_some() {
            return;
        }
        for subject in [Subject::State, Subject::Info] {
            let Some(forced) = self.stream(subject).take_due(turn.now_ms) else {
                continue;
            };
            if self.start(subject, forced, turn.now_ms, own) {
                return;
            }
        }
    }

    fn start(&mut self, subject: Subject, forced: bool, now_ms: u32, own: OwnLamps<'_>) -> bool {
        let announced = self.stream(subject).announced;
        let moved = match subject {
            Subject::State => {
                fill_state(&mut self.state_next, (own.lamps, own.parts), ANNOUNCED_ADAPTER, now_ms);
                let moved = !announced || !self.state_next.same_but_age(&self.state_sent);
                mem::swap(&mut self.state_next, &mut self.state_sent);
                moved
            }
            Subject::Info => {
                fill_info(&mut self.info_next, own.lamps, ANNOUNCED_ADAPTER);
                let moved = !announced || *self.info_next != *self.info_sent;
                mem::swap(&mut self.info_next, &mut self.info_sent);
                moved
            }
        };
        if !moved && !forced {
            return false;
        }
        let stream = self.stream(subject);
        stream.announced = true;
        stream.started_ms = Some(now_ms);
        self.paging = Some(Paging { subject, cursor: PageCursor::new(self.next_set), next_ms: now_ms });
        self.next_set = self.next_set.wrapping_add(1);
        true
    }

    pub(crate) fn page_due(&self, now_ms: u32) -> bool {
        self.paging.is_some_and(|paging| reached(now_ms, paging.next_ms))
    }

    pub(crate) fn write_page(
        &mut self,
        buffer: &mut DatagramBuffer,
        header: &Header,
        now_ms: u32,
    ) -> Result<usize, EncodeError> {
        let Some(paging) = self.paging.as_mut() else {
            return Err(EncodeError::TooManyPages);
        };
        let scope = AdapterScope { adapter: ANNOUNCED_ADAPTER };
        let written = match paging.subject {
            Subject::State => paging.cursor.write_next_of::<LampStateKind>(buffer, header, scope, self.state_sent.elements()),
            Subject::Info => paging.cursor.write_next_of::<LampInfoKind>(buffer, header, scope, self.info_sent.elements()),
        };
        paging.next_ms = now_ms.wrapping_add(PAGE_GAP_MS);
        if paging.cursor.finished() || written.is_err() {
            self.paging = None;
        }
        written?.ok_or(EncodeError::TooManyPages)
    }

    pub(crate) fn deadline(&self) -> Option<u32> {
        let paging = self.paging.map(|paging| paging.next_ms);
        let due = [self.state.due_ms, self.info.due_ms].into_iter().flatten().filter(|_| paging.is_none());
        due.chain([paging, self.refresh_ms].into_iter().flatten()).reduce(|earliest, at_ms| {
            if reached(at_ms, earliest) {
                earliest
            } else {
                at_ms
            }
        })
    }
}

fn refresh_phase(turn: Turn) -> u32 {
    let period_ms = turn.timing.info_refresh_ms().max(1);
    let slot_ms = period_ms / u32::from(NODE_COUNT);
    next_phase(turn.now_ms, period_ms, slot_ms * u32::from(turn.node) + slot_ms / PHASE_HALVES)
}

fn placed<S: LampSet>() -> PsramBox<S> {
    // SAFETY: `LampSet::init` writes every field of the place once.
    unsafe { PsramBox::new_with(|place: &mut MaybeUninit<S>| S::init(place.as_mut_ptr())) }
}
