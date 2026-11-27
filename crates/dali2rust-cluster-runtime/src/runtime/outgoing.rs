use core::mem::{self, MaybeUninit};
use core::ptr;

use dali2rust_bsp::psram::PsramBox;

use dali2rust_contracts::msg::{ClusterCommandCommand, LevelTransition, LightSetpoint};

use crate::frame::{Action, Command, Header, LightVerb, RecallScope, Target};
use crate::presence::HALF_CLOCK_MS;

#[cfg(test)]
mod tests;

pub(crate) const COPY_GAP_MS: u32 = 20;

pub(crate) const TARGET_SPACING_MS: u32 = 100;

pub(crate) const RELATIVE_SPACING_MS: u32 = 50;

pub(crate) const RELATIVE_LINE: usize = 4;

const LINE: usize = 2 * RELATIVE_LINE + 1;

pub(crate) const OUTGOING_TARGETS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetKey {
    Channel(u8),
    Node,
    All,
    Lamp([u8; 3]),
}

impl TargetKey {
    pub(crate) const fn of(action: &Action) -> Self {
        match action {
            Action::Light { target: Target::Channel(channel), .. }
            | Action::RecallScene { scope: RecallScope::Channel(channel), .. } => Self::Channel(channel.number),
            Action::RecallScene { scope: RecallScope::Node, .. } => Self::Node,
            Action::Light { target: Target::All, .. } => Self::All,
            Action::Light { target: Target::Lamp(lamp), .. } => Self::Lamp([lamp.node, lamp.adapter, lamp.lamp]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Queued {
    pub(crate) correlation_id: u64,
    pub(crate) request: ClusterCommandCommand,
    pub(crate) forwarded: bool,
}

impl Queued {
    fn folded_over(mut self, earlier: Option<&Self>) -> Self {
        let Some(earlier) = earlier.filter(|earlier| self.folds_over(earlier)) else {
            return self;
        };
        let setpoints = (setpoint_mut(&mut self.request.action), setpoint_of(&earlier.request.action));
        if let (Some(later), Some(base)) = setpoints {
            let mut merged = base.clone();
            merged.merge_from(later);
            *later = merged;
        }
        self
    }

    fn folds_over(&self, earlier: &Self) -> bool {
        let same_source = self.request.source == earlier.request.source;
        self.forwarded && earlier.forwarded && same_source && self.request.hold_hcl == earlier.request.hold_hcl
    }

    pub(crate) const fn relative(&self) -> bool {
        is_relative(&self.request.action)
    }

    const fn spacing_ms(&self) -> u32 {
        if self.relative() {
            RELATIVE_SPACING_MS
        } else {
            TARGET_SPACING_MS
        }
    }
}

pub(crate) const fn is_relative(action: &Action) -> bool {
    let Action::Light { verb, .. } = action else {
        return false;
    };
    match verb {
        LightVerb::StopFade => true,
        LightVerb::Setpoint(_) => false,
        LightVerb::Transition(transition) => relative_transition(*transition),
    }
}

const fn relative_transition(transition: LevelTransition) -> bool {
    match transition {
        LevelTransition::Up
        | LevelTransition::Down
        | LevelTransition::StepUp
        | LevelTransition::StepDown
        | LevelTransition::OnAndStepUp
        | LevelTransition::StepDownAndOff => true,
        LevelTransition::RecallMaxLevel | LevelTransition::RecallMinLevel | LevelTransition::GoToLastActiveLevel => {
            false
        }
    }
}

#[derive(Debug)]
struct Line {
    len: usize,
    entries: [Option<Queued>; LINE],
}

impl Line {
    const fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn head(&self) -> Option<&Queued> {
        self.entries.first().and_then(Option::as_ref)
    }

    fn relatives(&self) -> usize {
        self.entries.iter().flatten().filter(|queued| queued.relative()).count()
    }

    fn push(&mut self, queued: Queued) {
        if let Some(place) = self.entries.get_mut(self.len) {
            *place = Some(queued);
            self.len += 1;
        }
    }

    fn pop(&mut self) -> Option<Queued> {
        let head = self.entries.first_mut()?.take()?;
        self.entries.rotate_left(1);
        self.len -= 1;
        Some(head)
    }

    fn tail_mut(&mut self) -> Option<&mut Queued> {
        let last = self.len.checked_sub(1)?;
        self.entries.get_mut(last)?.as_mut()
    }

    fn join(&mut self, queued: Queued) -> Admission {
        if queued.relative() {
            if self.relatives() >= RELATIVE_LINE {
                return Admission::Dropped(queued);
            }
            self.push(queued);
            return Admission::Later { superseded: None };
        }
        if let Some(tail) = self.tail_mut().filter(|tail| !tail.relative()) {
            let folded = queued.folded_over(Some(&*tail));
            return Admission::Later { superseded: Some(mem::replace(tail, folded)) };
        }
        self.push(queued);
        Admission::Later { superseded: None }
    }

    fn drain(&mut self) -> impl Iterator<Item = Queued> + '_ {
        self.len = 0;
        self.entries.iter_mut().filter_map(Option::take)
    }
}

const fn setpoint_of(action: &Action) -> Option<&LightSetpoint> {
    match action {
        Action::Light { verb: LightVerb::Setpoint(setpoint), .. } => Some(setpoint),
        Action::Light { .. } | Action::RecallScene { .. } => None,
    }
}

fn setpoint_mut(action: &mut Action) -> Option<&mut LightSetpoint> {
    match action {
        Action::Light { verb: LightVerb::Setpoint(setpoint), .. } => Some(setpoint),
        Action::Light { .. } | Action::RecallScene { .. } => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repeat {
    pub(crate) header: Header,
    pub(crate) command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Admission {
    Now(Queued),
    AfterCopy,
    Later { superseded: Option<Queued> },
    Dropped(Queued),
    Full(Queued),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Due {
    Repeat(Repeat),
    Waiting(Queued),
}

#[derive(Debug)]
struct Slot {
    key: Option<TargetKey>,
    sent_ms: u32,
    repeat: Option<(u32, Repeat)>,
    line: Line,
}

impl Slot {
    fn spaced(&self, spacing_ms: u32, now_ms: u32) -> bool {
        reached(now_ms, self.sent_ms.wrapping_add(spacing_ms))
    }

    fn idle(&self, now_ms: u32) -> bool {
        self.repeat.is_none() && self.line.is_empty() && self.spaced(TARGET_SPACING_MS, now_ms)
    }

    fn repeat_due(&self) -> Option<u32> {
        self.repeat.as_ref().map(|(due_ms, _)| *due_ms)
    }

    fn waiting_due(&self) -> Option<u32> {
        self.line.head().map(|head| self.sent_ms.wrapping_add(head.spacing_ms()))
    }

    fn claim(&mut self, key: TargetKey, sent_ms: u32) {
        self.key = Some(key);
        self.sent_ms = sent_ms;
        self.repeat = None;
    }
}

const fn reached(now_ms: u32, at_ms: u32) -> bool {
    now_ms.wrapping_sub(at_ms) < HALF_CLOCK_MS
}

fn earliest(deadlines: impl Iterator<Item = u32>) -> Option<u32> {
    deadlines.reduce(|earliest, at_ms| if at_ms.wrapping_sub(earliest) < HALF_CLOCK_MS { earliest } else { at_ms })
}

#[derive(Debug)]
pub(crate) struct Outgoing {
    slots: [Slot; OUTGOING_TARGETS],
}

impl Outgoing {
    #[cfg(test)]
    pub(crate) fn new() -> PsramBox<Self> {
        placed_outgoing()
    }

    pub(crate) fn admit(&mut self, queued: Queued, now_ms: u32) -> Admission {
        self.prune(now_ms);
        let copying = self.copying();
        let key = TargetKey::of(&queued.request.action);
        if let Some(slot) = self.slot_mut(key) {
            if !slot.line.is_empty() || !slot.spaced(queued.spacing_ms(), now_ms) {
                return slot.line.join(queued);
            }
            if !copying {
                return Admission::Now(queued);
            }
            slot.line.push(queued);
            return Admission::AfterCopy;
        }
        self.admit_new(key, queued, copying, now_ms)
    }

    fn admit_new(&mut self, key: TargetKey, queued: Queued, copying: bool, now_ms: u32) -> Admission {
        let Some(free) = self.vacant() else {
            return Admission::Full(queued);
        };
        if !copying {
            return Admission::Now(queued);
        }
        free.claim(key, now_ms.wrapping_sub(TARGET_SPACING_MS));
        free.line.push(queued);
        Admission::AfterCopy
    }

    pub(crate) fn sent(&mut self, key: TargetKey, repeat: Repeat, now_ms: u32) {
        let due_ms = now_ms.wrapping_add(COPY_GAP_MS);
        let slot = match self.slots.iter().position(|slot| slot.key == Some(key)) {
            Some(held) => self.slots.get_mut(held),
            None => self.vacant(),
        };
        if let Some(slot) = slot {
            slot.claim(key, now_ms);
            slot.repeat = Some((due_ms, repeat));
        }
    }

    pub(crate) fn take_due(&mut self, now_ms: u32) -> Option<Due> {
        if self.copying() {
            let slot = self.most_late(now_ms, Slot::repeat_due)?;
            return slot.repeat.take().map(|(_, repeat)| Due::Repeat(repeat));
        }
        self.most_late(now_ms, Slot::waiting_due)?.line.pop().map(Due::Waiting)
    }

    pub(crate) fn deadline(&self) -> Option<u32> {
        let slots = self.held();
        let due: fn(&Slot) -> Option<u32> = if self.copying() { Slot::repeat_due } else { Slot::waiting_due };
        earliest(slots.filter_map(due))
    }

    fn copying(&self) -> bool {
        self.held().any(|slot| slot.repeat.is_some())
    }

    fn held(&self) -> impl Iterator<Item = &Slot> {
        self.slots.iter().filter(|slot| slot.key.is_some())
    }

    fn most_late(&mut self, now_ms: u32, due: fn(&Slot) -> Option<u32>) -> Option<&mut Slot> {
        let lateness = move |slot: &Slot| due(slot).map(|at_ms| now_ms.wrapping_sub(at_ms)).filter(|late| *late < HALF_CLOCK_MS);
        let late = self.slots.iter_mut().filter(|slot| slot.key.is_some() && lateness(slot).is_some());
        late.max_by_key(|slot| lateness(slot))
    }

    pub(crate) fn prune(&mut self, now_ms: u32) {
        for slot in &mut self.slots {
            if slot.key.is_some() && slot.idle(now_ms) {
                slot.key = None;
            }
        }
    }

    pub(crate) fn abandon(&mut self) -> impl Iterator<Item = Queued> + '_ {
        self.slots.iter_mut().flat_map(|slot| {
            slot.key = None;
            slot.repeat = None;
            slot.line.drain()
        })
    }

    fn slot_mut(&mut self, key: TargetKey) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|slot| slot.key == Some(key))
    }

    fn vacant(&mut self) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|slot| slot.key.is_none())
    }
}

pub(crate) fn placed_outgoing() -> PsramBox<Outgoing> {
    // SAFETY: `slots` is the only field of `Outgoing`; every field of each slot and of its line is written once, in place.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<Outgoing>| {
            let slots = ptr::addr_of_mut!((*place.as_mut_ptr()).slots).cast::<Slot>();
            for index in 0..OUTGOING_TARGETS {
                write_vacant(slots.add(index));
            }
        })
    }
}

unsafe fn write_vacant(slot: *mut Slot) {
    // SAFETY: the caller hands a valid, uninitialised slot; each field is written once and nothing is read.
    unsafe {
        ptr::addr_of_mut!((*slot).key).write(None);
        ptr::addr_of_mut!((*slot).sent_ms).write(0);
        ptr::addr_of_mut!((*slot).repeat).write(None);
        ptr::addr_of_mut!((*slot).line.len).write(0);
        let entries = ptr::addr_of_mut!((*slot).line.entries).cast::<Option<Queued>>();
        for index in 0..LINE {
            entries.add(index).write(None);
        }
    }
}
