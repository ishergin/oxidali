use core::mem::{self, MaybeUninit};
use core::ptr;

use dali2rust_bsp::psram::PsramBox;

use dali2rust_contracts::msg::{ClusterCommandCommand, LightSetpoint};

use crate::frame::{Action, Command, Header, LightVerb, RecallScope, Target};
use crate::presence::HALF_CLOCK_MS;

#[cfg(test)]
mod tests;

pub(crate) const COPY_GAP_MS: u32 = 20;

pub(crate) const TARGET_SPACING_MS: u32 = 100;

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
        let Some(earlier) = earlier else {
            return self;
        };
        let setpoints = (setpoint_mut(&mut self.request.action), setpoint_of(&earlier.request.action));
        if let (Some(later), Some(base)) = setpoints {
            let mut merged = base.clone();
            merged.merge_from(later);
            *later = merged;
            self.forwarded &= earlier.forwarded;
        }
        self
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
    Full(Queued),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Due {
    Repeat(Repeat),
    Waiting(Queued),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Slot {
    key: TargetKey,
    sent_ms: u32,
    repeat: Option<(u32, Repeat)>,
    waiting: Option<Queued>,
}

impl Slot {
    fn spaced(&self, now_ms: u32) -> bool {
        reached(now_ms, self.sent_ms.wrapping_add(TARGET_SPACING_MS))
    }

    fn idle(&self, now_ms: u32) -> bool {
        self.repeat.is_none() && self.waiting.is_none() && self.spaced(now_ms)
    }

    fn repeat_due(&self) -> Option<u32> {
        self.repeat.as_ref().map(|(due_ms, _)| *due_ms)
    }

    fn waiting_due(&self) -> Option<u32> {
        self.waiting.as_ref().map(|_| self.sent_ms.wrapping_add(TARGET_SPACING_MS))
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
    slots: [Option<Slot>; OUTGOING_TARGETS],
}

impl Outgoing {
    #[cfg(test)]
    pub(crate) const fn new() -> Self {
        Self { slots: [const { None }; OUTGOING_TARGETS] }
    }

    pub(crate) fn admit(&mut self, queued: Queued, now_ms: u32) -> Admission {
        self.prune(now_ms);
        let copying = self.copying();
        let key = TargetKey::of(&queued.request.action);
        if let Some(slot) = self.slot_mut(key) {
            if slot.waiting.is_some() || !slot.spaced(now_ms) {
                let queued = queued.folded_over(slot.waiting.as_ref());
                return Admission::Later { superseded: slot.waiting.replace(queued) };
            }
            if !copying {
                return Admission::Now(queued);
            }
            slot.waiting = Some(queued);
            return Admission::AfterCopy;
        }
        self.admit_new(key, queued, copying, now_ms)
    }

    fn admit_new(&mut self, key: TargetKey, queued: Queued, copying: bool, now_ms: u32) -> Admission {
        let Some(free) = self.slots.iter_mut().find(|place| place.is_none()) else {
            return Admission::Full(queued);
        };
        if !copying {
            return Admission::Now(queued);
        }
        let sent_ms = now_ms.wrapping_sub(TARGET_SPACING_MS);
        *free = Some(Slot { key, sent_ms, repeat: None, waiting: Some(queued) });
        Admission::AfterCopy
    }

    pub(crate) fn sent(&mut self, key: TargetKey, repeat: Repeat, now_ms: u32) {
        let slot = Slot { key, sent_ms: now_ms, repeat: Some((now_ms.wrapping_add(COPY_GAP_MS), repeat)), waiting: None };
        if let Some(held) = self.slot_mut(key) {
            let waiting = held.waiting.take();
            *held = Slot { waiting, ..slot };
            return;
        }
        if let Some(free) = self.slots.iter_mut().find(|place| place.is_none()) {
            *free = Some(slot);
        }
    }

    pub(crate) fn take_due(&mut self, now_ms: u32) -> Option<Due> {
        if self.copying() {
            let slot = self.most_late(now_ms, Slot::repeat_due)?;
            return slot.repeat.take().map(|(_, repeat)| Due::Repeat(repeat));
        }
        self.most_late(now_ms, Slot::waiting_due)?.waiting.take().map(Due::Waiting)
    }

    pub(crate) fn deadline(&self) -> Option<u32> {
        let slots = self.slots.iter().flatten();
        let due: fn(&Slot) -> Option<u32> = if self.copying() { Slot::repeat_due } else { Slot::waiting_due };
        earliest(slots.filter_map(due))
    }

    fn copying(&self) -> bool {
        self.slots.iter().flatten().any(|slot| slot.repeat.is_some())
    }

    fn most_late(&mut self, now_ms: u32, due: fn(&Slot) -> Option<u32>) -> Option<&mut Slot> {
        let lateness = move |slot: &Slot| due(slot).map(|at_ms| now_ms.wrapping_sub(at_ms)).filter(|late| *late < HALF_CLOCK_MS);
        let late = self.slots.iter_mut().flatten().filter(|slot| lateness(slot).is_some());
        late.max_by_key(|slot| lateness(slot))
    }

    pub(crate) fn prune(&mut self, now_ms: u32) {
        for place in &mut self.slots {
            if place.as_ref().is_some_and(|slot| slot.idle(now_ms)) {
                *place = None;
            }
        }
    }

    pub(crate) fn abandon(&mut self) -> impl Iterator<Item = Queued> + '_ {
        self.slots.iter_mut().filter_map(|place| mem::take(place).and_then(|slot| slot.waiting))
    }

    fn slot_mut(&mut self, key: TargetKey) -> Option<&mut Slot> {
        self.slots.iter_mut().flatten().find(|slot| slot.key == key)
    }
}

pub(crate) fn placed_outgoing() -> PsramBox<Outgoing> {
    // SAFETY: `slots` is the only field of `Outgoing`; each of its places is written once with `None`.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<Outgoing>| {
            let slots = ptr::addr_of_mut!((*place.as_mut_ptr()).slots).cast::<Option<Slot>>();
            for index in 0..OUTGOING_TARGETS {
                slots.add(index).write(None);
            }
        })
    }
}
