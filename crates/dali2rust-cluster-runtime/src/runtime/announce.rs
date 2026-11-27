use core::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

use dali2rust_bsp::psram::PsramBox;
use dali2rust_domain::registry::{ClusterPartReadPort, ClusterRecordReadPort, CLUSTER_RECORD_KINDS};

use crate::frame::{DatagramBuffer, EncodeError, Header, PageCursor, NODE_COUNT};
use crate::info::{placed_set, InfoSet};
use crate::presence::{PresenceTiming, HALF_CLOCK_MS};

pub(crate) const PAGE_GAP_MS: u32 = 10;

#[derive(Clone, Copy)]
pub(crate) struct OwnInfo<'a> {
    pub(crate) records: &'a dyn ClusterRecordReadPort,
    pub(crate) parts: &'a dyn ClusterPartReadPort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Turn {
    pub(crate) running: bool,
    pub(crate) node: u8,
    pub(crate) now_ms: u32,
    pub(crate) hello: bool,
    pub(crate) timing: PresenceTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Paging {
    cursor: PageCursor,
    next_ms: u32,
}

pub(crate) struct Announcer {
    snapshot: PsramBox<InfoSet>,
    announced: Option<u64>,
    dirty: bool,
    due_ms: Option<u32>,
    refresh_ms: Option<u32>,
    paging: Option<Paging>,
    next_set: u32,
}

impl Announcer {
    pub(crate) fn new() -> Self {
        Self {
            snapshot: placed_set(),
            announced: None,
            dirty: false,
            due_ms: None,
            refresh_ms: None,
            paging: None,
            next_set: 0,
        }
    }

    pub(crate) fn note_settings_changed(&mut self) {
        self.dirty = true;
    }

    pub(crate) const fn announced(&self) -> Option<u64> {
        self.announced
    }

    pub(crate) fn plan(&mut self, turn: Turn, own: OwnInfo<'_>) {
        if !turn.running {
            self.stand_down();
            return;
        }
        self.notice_change(turn, own);
        if turn.hello {
            self.schedule(turn.now_ms.wrapping_add(hello_delay_ms(turn)));
        }
        let refresh_ms = *self.refresh_ms.get_or_insert_with(|| refresh_phase(turn));
        if reached(turn.now_ms, refresh_ms) {
            self.schedule(turn.now_ms);
            self.refresh_ms = Some(refresh_phase(turn));
        }
        self.start_due(turn, own);
    }

    fn notice_change(&mut self, turn: Turn, own: OwnInfo<'_>) {
        if !self.dirty && self.announced.is_some() {
            return;
        }
        self.dirty = false;
        if self.announced != Some(fingerprint(turn.node, own)) {
            self.schedule(turn.now_ms.wrapping_add(turn.timing.info_coalesce_ms()));
        }
    }

    fn stand_down(&mut self) {
        self.announced = None;
        self.due_ms = None;
        self.refresh_ms = None;
        self.paging = None;
    }

    fn schedule(&mut self, at_ms: u32) {
        self.due_ms = Some(match self.due_ms {
            Some(due_ms) if !reached(due_ms, at_ms) => due_ms,
            _ => at_ms,
        });
    }

    fn start_due(&mut self, turn: Turn, own: OwnInfo<'_>) {
        let due = self.due_ms.is_some_and(|due_ms| reached(turn.now_ms, due_ms));
        if self.paging.is_some() || !due {
            return;
        }
        self.due_ms = None;
        self.announced = Some(fingerprint(turn.node, own));
        self.snapshot.fill(turn.node, own.records, own.parts);
        self.paging = Some(Paging { cursor: PageCursor::new(self.next_set), next_ms: turn.now_ms });
        self.next_set = self.next_set.wrapping_add(1);
    }

    pub(crate) const fn paging(&self) -> bool {
        self.paging.is_some()
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
        let written = paging.cursor.write_next(buffer, header, self.snapshot.elements());
        let closes_set = paging.cursor.finished();
        paging.next_ms = now_ms.wrapping_add(PAGE_GAP_MS);
        if closes_set || written.is_err() {
            self.paging = None;
        }
        written?.ok_or(EncodeError::TooManyPages)
    }

    pub(crate) fn deadline(&self) -> Option<u32> {
        let paging = self.paging.map(|paging| paging.next_ms);
        [self.due_ms, paging, self.refresh_ms].into_iter().flatten().reduce(|earliest, at_ms| {
            if reached(at_ms, earliest) {
                at_ms
            } else {
                earliest
            }
        })
    }
}

pub(crate) const fn reached(now_ms: u32, at_ms: u32) -> bool {
    now_ms.wrapping_sub(at_ms) < HALF_CLOCK_MS
}

fn refresh_phase(turn: Turn) -> u32 {
    let period_ms = turn.timing.info_refresh_ms().max(1);
    let offset_ms = period_ms / u32::from(NODE_COUNT) * u32::from(turn.node);
    let start_ms = turn.now_ms.wrapping_sub(turn.now_ms % period_ms).wrapping_add(offset_ms);
    if reached(turn.now_ms, start_ms) {
        start_ms.wrapping_add(period_ms)
    } else {
        start_ms
    }
}

const fn hello_delay_ms(turn: Turn) -> u32 {
    turn.timing.info_coalesce_ms().saturating_mul(1 + turn.node as u32)
}

fn fingerprint(node: u8, own: OwnInfo<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    node.hash(&mut hasher);
    for kind in CLUSTER_RECORD_KINDS {
        for number in own.records.cluster_record_numbers(kind).iter() {
            let record = own.records.cluster_record(kind, number);
            (number, record.map(|view| (view.mark.generation, view.mark.nonce, view.revision))).hash(&mut hasher);
        }
        for (adapter, local) in own.parts.cluster_part_keys(kind).iter() {
            let part = own.parts.cluster_part(kind, adapter, local);
            let held = part.map(|view| {
                let record = (view.record.number, view.record.mark.generation, view.record.mark.nonce);
                (record, view.send, view.accept, view.revision)
            });
            (adapter, local, held).hash(&mut hasher);
        }
    }
    hasher.finish()
}
