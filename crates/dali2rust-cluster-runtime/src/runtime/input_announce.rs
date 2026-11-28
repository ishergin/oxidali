use dali2rust_bsp::psram::PsramBox;
use dali2rust_domain::registry::ClusterInputReadPort;

use crate::frame::{DatagramBuffer, EncodeError, Header, InputInfoKind, PageCursor, NODE_COUNT};
use crate::inputs::{fingerprint, placed_own_inputs, OwnIdentity, OwnInputs};

use super::announce::{hello_delay_ms, next_phase, reached, Turn, PAGE_GAP_MS};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Paging {
    cursor: PageCursor,
    next_ms: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct OwnInputReads<'a> {
    pub(crate) identity: &'a OwnIdentity,
    pub(crate) port: &'a dyn ClusterInputReadPort,
}

pub(crate) struct InputAnnouncer {
    snapshot: PsramBox<OwnInputs>,
    announced: Option<u64>,
    dirty: bool,
    due_ms: Option<u32>,
    refresh_ms: Option<u32>,
    paging: Option<Paging>,
    next_set: u32,
}

impl InputAnnouncer {
    pub(crate) fn new() -> Self {
        Self {
            snapshot: placed_own_inputs(),
            announced: None,
            dirty: false,
            due_ms: None,
            refresh_ms: None,
            paging: None,
            next_set: 0,
        }
    }

    pub(crate) fn note_changed(&mut self) {
        self.dirty = true;
    }

    pub(crate) const fn announced(&self) -> Option<u64> {
        self.announced
    }

    pub(crate) fn plan(&mut self, turn: Turn, own: OwnInputReads<'_>) {
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

    fn notice_change(&mut self, turn: Turn, own: OwnInputReads<'_>) {
        if !self.dirty && self.announced.is_some() {
            return;
        }
        self.dirty = false;
        if self.announced != Some(fingerprint(own.identity, own.port)) {
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

    fn start_due(&mut self, turn: Turn, own: OwnInputReads<'_>) {
        let due = self.due_ms.is_some_and(|due_ms| reached(turn.now_ms, due_ms));
        if self.paging.is_some() || !due {
            return;
        }
        self.due_ms = None;
        self.announced = Some(fingerprint(own.identity, own.port));
        self.snapshot.fill(own.identity.clone(), own.port);
        self.paging = Some(Paging { cursor: PageCursor::new(self.next_set), next_ms: turn.now_ms });
        self.next_set = self.next_set.wrapping_add(1);
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
        let written = paging.cursor.write_next_of::<InputInfoKind>(buffer, header, (), self.snapshot.elements(now_ms));
        let closes_set = paging.cursor.finished();
        paging.next_ms = now_ms.wrapping_add(PAGE_GAP_MS);
        if closes_set || written.is_err() {
            self.paging = None;
        }
        written?.ok_or(EncodeError::TooManyPages)
    }

    pub(crate) fn deadline(&self) -> Option<u32> {
        let paging = self.paging.map(|paging| paging.next_ms);
        let due_ms = self.due_ms.filter(|_| paging.is_none());
        [due_ms, paging, self.refresh_ms].into_iter().flatten().reduce(|earliest, at_ms| {
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
    let offset_ms = period_ms / 2 + period_ms / 2 / u32::from(NODE_COUNT) * u32::from(turn.node);
    next_phase(turn.now_ms, period_ms, offset_ms % period_ms)
}
