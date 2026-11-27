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

const CROSSED_LINE: usize = 2;
const LINE: usize = 2 * RELATIVE_LINE + 1 + CROSSED_LINE;

pub(crate) const OUTGOING_TARGETS: usize = 16;

const ORDER_HALF: u32 = 1 << 31;

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

    const fn covers_all(self) -> bool {
        matches!(self, Self::All | Self::Node)
    }

    fn overlaps(self, other: Self) -> bool {
        self == other || self.covers_all() || other.covers_all()
    }
}

const fn earlier(order: u32, than: u32) -> bool {
    let gap = than.wrapping_sub(order);
    gap != 0 && gap < ORDER_HALF
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
    orders: [u32; LINE],
}

impl Line {
    const fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn head(&self) -> Option<&Queued> {
        self.entries.first().and_then(Option::as_ref)
    }

    fn head_order(&self) -> Option<u32> {
        self.head().and(self.orders.first().copied())
    }

    fn relatives(&self) -> usize {
        self.entries.iter().flatten().filter(|queued| queued.relative()).count()
    }

    fn crossings(&self) -> usize {
        let waiting = || self.entries.iter().flatten();
        waiting().zip(waiting().skip(1)).filter(|(first, next)| !first.relative() && !next.relative()).count()
    }

    fn tail_order(&self) -> Option<u32> {
        self.orders.get(self.len.checked_sub(1)?).copied()
    }

    fn push(&mut self, queued: Queued, order: u32) -> Admission {
        let (Some(place), Some(stamp)) = (self.entries.get_mut(self.len), self.orders.get_mut(self.len)) else {
            return Admission::Dropped(queued);
        };
        *place = Some(queued);
        *stamp = order;
        self.len += 1;
        Admission::Later { superseded: None }
    }

    fn pop(&mut self) -> Option<Queued> {
        let head = self.entries.first_mut()?.take()?;
        self.entries.rotate_left(1);
        self.orders.rotate_left(1);
        self.len -= 1;
        Some(head)
    }

    fn tail_mut(&mut self) -> Option<(&mut Queued, &mut u32)> {
        let last = self.len.checked_sub(1)?;
        Some((self.entries.get_mut(last)?.as_mut()?, self.orders.get_mut(last)?))
    }

    fn join(&mut self, queued: Queued, order: u32, crossed: bool) -> Admission {
        if queued.relative() {
            if self.relatives() >= RELATIVE_LINE {
                return Admission::Dropped(queued);
            }
            return self.push(queued, order);
        }
        let replaceable = !crossed || self.crossings() >= CROSSED_LINE;
        if let Some((tail, stamp)) = self.tail_mut().filter(|(tail, _)| replaceable && !tail.relative()) {
            let folded = queued.folded_over(Some(&*tail));
            *stamp = order;
            return Admission::Later { superseded: Some(mem::replace(tail, folded)) };
        }
        self.push(queued, order)
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
    next_order: u32,
}

fn waiting_as(pushed: Admission, waiting: Admission) -> Admission {
    match pushed {
        Admission::Later { superseded: None } => waiting,
        refused => refused,
    }
}

impl Outgoing {
    #[cfg(test)]
    pub(crate) fn new() -> PsramBox<Self> {
        placed_outgoing()
    }

    pub(crate) fn admit(&mut self, queued: Queued, now_ms: u32) -> Admission {
        self.prune(now_ms);
        let key = TargetKey::of(&queued.request.action);
        let (copying, behind, crossed) = (self.copying(), self.behind_another(key), self.crossed_since_tail(key));
        let order = self.stamp();
        if let Some(slot) = self.slot_mut(key) {
            if behind || !slot.line.is_empty() || !slot.spaced(queued.spacing_ms(), now_ms) {
                return slot.line.join(queued, order, crossed);
            }
            if !copying {
                return Admission::Now(queued);
            }
            return waiting_as(slot.line.push(queued, order), Admission::AfterCopy);
        }
        self.admit_new(key, queued, (copying, behind), (order, now_ms))
    }

    fn admit_new(&mut self, key: TargetKey, queued: Queued, held: (bool, bool), at: (u32, u32)) -> Admission {
        let ((copying, behind), (order, now_ms)) = (held, at);
        let Some(free) = self.vacant() else {
            return Admission::Full(queued);
        };
        if !copying && !behind {
            return Admission::Now(queued);
        }
        free.claim(key, now_ms.wrapping_sub(TARGET_SPACING_MS));
        let waiting = if behind { Admission::Later { superseded: None } } else { Admission::AfterCopy };
        waiting_as(free.line.push(queued, order), waiting)
    }

    fn stamp(&mut self) -> u32 {
        let order = self.next_order;
        self.next_order = order.wrapping_add(1);
        order
    }

    fn behind_another(&self, key: TargetKey) -> bool {
        self.held().any(|slot| slot.key.is_some_and(|other| other != key && other.overlaps(key)) && !slot.line.is_empty())
    }

    fn crossed_since_tail(&self, key: TargetKey) -> bool {
        let Some(tail) = self.held().find(|slot| slot.key == Some(key)).and_then(|slot| slot.line.tail_order()) else {
            return false;
        };
        let overlapping = self.held().filter(|other| other.key.is_some_and(|other| other != key && other.overlaps(key)));
        overlapping.filter_map(|other| other.line.tail_order()).any(|later| earlier(tail, later))
    }

    fn first_in_order(&self, slot: &Slot) -> bool {
        let (Some(key), Some(order)) = (slot.key, slot.line.head_order()) else {
            return false;
        };
        let overlapping = self.held().filter(|other| other.key.is_some_and(|other| other != key && other.overlaps(key)));
        !overlapping.filter_map(|other| other.line.head_order()).any(|other| earlier(other, order))
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
        let next = self.next_waiting(now_ms)?;
        self.slots.get_mut(next)?.line.pop().map(Due::Waiting)
    }

    pub(crate) fn deadline(&self) -> Option<u32> {
        if self.copying() {
            return earliest(self.held().filter_map(Slot::repeat_due));
        }
        earliest(self.held().filter(|slot| self.first_in_order(slot)).filter_map(Slot::waiting_due))
    }

    fn next_waiting(&self, now_ms: u32) -> Option<usize> {
        let lateness = |slot: &Slot| slot.waiting_due().map(|at_ms| now_ms.wrapping_sub(at_ms)).filter(|late| *late < HALF_CLOCK_MS);
        let free = self.slots.iter().enumerate().filter(|(_, slot)| self.first_in_order(slot));
        let late = free.filter_map(|(index, slot)| lateness(slot).map(|late| (index, late)));
        late.max_by_key(|(_, late)| *late).map(|(index, _)| index)
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
    // SAFETY: `slots` and `next_order` are the fields of `Outgoing`; each, and every field of each slot and of its line, is written once, in place.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<Outgoing>| {
            ptr::addr_of_mut!((*place.as_mut_ptr()).next_order).write(0);
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
        ptr::addr_of_mut!((*slot).line.orders).write([0; LINE]);
    }
}
