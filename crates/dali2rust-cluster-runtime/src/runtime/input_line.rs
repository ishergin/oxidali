use core::ptr;

use dali2rust_contracts::msg::InputEventKind;

use crate::frame::{Header, InputAddress, InputEvent};
use crate::presence::HALF_CLOCK_MS;

#[cfg(test)]
mod tests;

pub(crate) const INPUT_LINE: usize = 16;

pub(crate) const MEASURED_INSTANCES: usize = 16;

pub(crate) const ILLUMINANCE_PERIOD_MS: u32 = 1_000;

pub(crate) const POSITION_WINDOW_MS: u32 = 250;

const fn reached(now_ms: u32, at_ms: u32) -> bool {
    now_ms.wrapping_sub(at_ms) < HALF_CLOCK_MS
}

pub(crate) const fn period_ms(kind: InputEventKind) -> Option<u32> {
    match kind {
        InputEventKind::Illuminance => Some(ILLUMINANCE_PERIOD_MS),
        InputEventKind::Position => Some(POSITION_WINDOW_MS),
        InputEventKind::Generic | InputEventKind::Button | InputEventKind::Occupancy => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputRepeat {
    pub(crate) header: Header,
    pub(crate) event: InputEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputAdmission {
    Now(InputEvent),
    Later,
    Dropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Measure {
    address: InputAddress,
    period_ms: u32,
    sent_ms: u32,
    pending: Option<InputEvent>,
}

impl Measure {
    const fn due_ms(&self) -> u32 {
        self.sent_ms.wrapping_add(self.period_ms)
    }

    const fn open(&self, now_ms: u32) -> bool {
        !reached(now_ms, self.due_ms())
    }
}

#[derive(Debug)]
pub(crate) struct InputLine {
    queue: [Option<InputEvent>; INPUT_LINE],
    len: usize,
    repeat: Option<(u32, InputRepeat)>,
    measures: [Option<Measure>; MEASURED_INSTANCES],
}

impl InputLine {
    pub(crate) unsafe fn init(place: *mut Self) {
        // SAFETY: the caller hands a place of `InputLine`; every field is written once here and nothing is read.
        unsafe {
            ptr::addr_of_mut!((*place).queue).write([None; INPUT_LINE]);
            ptr::addr_of_mut!((*place).len).write(0);
            ptr::addr_of_mut!((*place).repeat).write(None);
            ptr::addr_of_mut!((*place).measures).write([None; MEASURED_INSTANCES]);
        }
    }

    pub(crate) const fn copying(&self) -> bool {
        self.repeat.is_some()
    }

    pub(crate) fn admit(&mut self, event: InputEvent, held: bool) -> InputAdmission {
        if !held && self.len == 0 {
            return InputAdmission::Now(event);
        }
        let Some(place) = self.queue.get_mut(self.len) else {
            return InputAdmission::Dropped;
        };
        *place = Some(event);
        self.len += 1;
        InputAdmission::Later
    }

    pub(crate) fn sent(&mut self, repeat: InputRepeat, now_ms: u32, gap_ms: u32) {
        self.repeat = Some((now_ms.wrapping_add(gap_ms), repeat));
    }

    pub(crate) fn repeat_due(&self) -> Option<u32> {
        self.repeat.map(|(due_ms, _)| due_ms)
    }

    pub(crate) fn take_repeat(&mut self, now_ms: u32) -> Option<InputRepeat> {
        self.repeat.take_if(|(due_ms, _)| reached(now_ms, *due_ms)).map(|(_, repeat)| repeat)
    }

    pub(crate) fn pop(&mut self) -> Option<InputEvent> {
        let head = self.queue.first_mut()?.take()?;
        self.queue.rotate_left(1);
        self.len -= 1;
        Some(head)
    }

    pub(crate) fn measure(&mut self, event: InputEvent, period_ms: u32, now_ms: u32) -> Option<InputEvent> {
        if let Some(measure) = self.measure_mut(event.address) {
            if measure.open(now_ms) {
                measure.pending = Some(event);
                return None;
            }
            *measure = Measure { period_ms, sent_ms: now_ms, pending: None, ..*measure };
            return Some(event);
        }
        let opened = Some(Measure { address: event.address, period_ms, sent_ms: now_ms, pending: None });
        let idle = |slot: &Option<Measure>| slot.is_some_and(|measure| measure.pending.is_none());
        let at = self.measures.iter().position(Option::is_none).or_else(|| self.measures.iter().position(idle));
        if let Some(slot) = at.and_then(|at| self.measures.get_mut(at)) {
            *slot = opened;
        }
        Some(event)
    }

    fn measure_mut(&mut self, address: InputAddress) -> Option<&mut Measure> {
        self.measures.iter_mut().flatten().find(|measure| measure.address == address)
    }

    pub(crate) fn take_trailing(&mut self, now_ms: u32) -> Option<InputEvent> {
        let due = self.measures.iter_mut().flatten().find(|measure| measure.pending.is_some() && !measure.open(now_ms))?;
        due.sent_ms = now_ms;
        due.pending.take()
    }

    pub(crate) fn trailing_due(&self) -> impl Iterator<Item = u32> + '_ {
        self.measures.iter().flatten().filter(|measure| measure.pending.is_some()).map(Measure::due_ms)
    }

    pub(crate) fn prune(&mut self, now_ms: u32) {
        for slot in &mut self.measures {
            slot.take_if(|measure| measure.pending.is_none() && !measure.open(now_ms));
        }
    }

    pub(crate) fn abandon(&mut self) -> u32 {
        let queued = u32::try_from(self.len).unwrap_or(u32::MAX);
        let pending = self.measures.iter().flatten().filter(|measure| measure.pending.is_some()).count();
        self.queue = [None; INPUT_LINE];
        self.len = 0;
        self.repeat = None;
        self.measures = [None; MEASURED_INSTANCES];
        queued.saturating_add(u32::try_from(pending).unwrap_or(u32::MAX))
    }
}
