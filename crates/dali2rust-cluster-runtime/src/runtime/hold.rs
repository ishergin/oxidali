use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use dali2rust_contracts::msg::ClusterCommandCommand;

use crate::frame::{Action, LightVerb};

use super::announce::reached;
use super::outgoing::{is_relative, RELATIVE_SPACING_MS};

#[cfg(test)]
mod tests;

const NEIGHBOUR_LATENCY_MS: u32 = 20;

pub(crate) const FOREIGN_HOLD_MS: u32 = RELATIVE_SPACING_MS + NEIGHBOUR_LATENCY_MS;

pub(crate) const HOLD_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Waiting {
    request: ClusterCommandCommand,
    due_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Echo {
    action: Action,
    until_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Held {
    Cancelled,
    Queued { held: bool, pushed_out: Option<ClusterCommandCommand> },
}

#[derive(Debug, Default)]
pub(crate) struct HoldLine {
    waiting: VecDeque<Waiting>,
    echoes: VecDeque<Echo>,
}

impl HoldLine {
    pub(crate) fn new() -> Self {
        Self { waiting: VecDeque::with_capacity(HOLD_DEPTH), echoes: VecDeque::with_capacity(HOLD_DEPTH) }
    }

    pub(crate) fn hold(&mut self, request: ClusterCommandCommand, shared: bool, now_ms: u32) -> Held {
        self.expire(now_ms);
        if self.take_echo(&request.action) {
            return Held::Cancelled;
        }
        let pushed_out = (self.waiting.len() >= HOLD_DEPTH).then(|| self.waiting.pop_front()).flatten();
        let due_ms = if shared { now_ms.wrapping_add(FOREIGN_HOLD_MS) } else { now_ms };
        self.waiting.push_back(Waiting { request, due_ms });
        Held::Queued { held: shared, pushed_out: pushed_out.map(|out| out.request) }
    }

    pub(crate) fn cancel(&mut self, neighbour: &Action, now_ms: u32) -> (u32, bool) {
        self.expire(now_ms);
        let cancelled = self.cancel_waiting(neighbour);
        let relative = is_relative(neighbour);
        if relative && cancelled > 0 {
            return (cancelled, false);
        }
        let evicted = self.echoes.len() >= HOLD_DEPTH;
        if evicted {
            self.echoes.pop_front();
        }
        self.echoes.push_back(Echo { action: neighbour.clone(), until_ms: now_ms.wrapping_add(FOREIGN_HOLD_MS) });
        (cancelled, evicted)
    }

    pub(crate) fn take_due(&mut self, now_ms: u32) -> Option<ClusterCommandCommand> {
        let due = self.waiting.front().is_some_and(|front| reached(now_ms, front.due_ms));
        due.then(|| self.waiting.pop_front()).flatten().map(|waiting| waiting.request)
    }

    pub(crate) fn deadline(&self) -> Option<u32> {
        self.waiting.front().map(|front| front.due_ms)
    }

    pub(crate) fn clear(&mut self) -> impl Iterator<Item = ClusterCommandCommand> + '_ {
        self.echoes.clear();
        self.waiting.drain(..).map(|waiting| waiting.request)
    }

    fn cancel_waiting(&mut self, neighbour: &Action) -> u32 {
        if is_relative(neighbour) {
            let first = self.waiting.iter().position(|waiting| same_command(&waiting.request.action, neighbour));
            return first.and_then(|index| self.waiting.remove(index)).map_or(0, |_| 1);
        }
        let before = self.waiting.len();
        self.waiting.retain(|waiting| !same_command(&waiting.request.action, neighbour));
        u32::try_from(before - self.waiting.len()).unwrap_or(u32::MAX)
    }

    fn take_echo(&mut self, observed: &Action) -> bool {
        let Some(index) = self.echoes.iter().position(|echo| same_command(observed, &echo.action)) else {
            return false;
        };
        if is_relative(observed) {
            self.echoes.remove(index);
        }
        true
    }

    fn expire(&mut self, now_ms: u32) {
        self.echoes.retain(|echo| !reached(now_ms, echo.until_ms));
    }
}

fn same_command(observed: &Action, neighbour: &Action) -> bool {
    match (observed, neighbour) {
        (Action::Light { target, verb }, Action::Light { target: other_target, verb: other_verb }) => {
            target == other_target && same_verb(verb, other_verb)
        }
        (Action::RecallScene { .. }, Action::RecallScene { .. }) => observed == neighbour,
        (Action::Light { .. }, Action::RecallScene { .. }) | (Action::RecallScene { .. }, Action::Light { .. }) => false,
    }
}

fn same_verb(observed: &LightVerb, neighbour: &LightVerb) -> bool {
    match (observed, neighbour) {
        (LightVerb::Setpoint(_), LightVerb::Setpoint(_)) => true,
        _ => observed == neighbour,
    }
}

pub(crate) struct ForeignHold {
    line: Mutex<HoldLine>,
}

impl ForeignHold {
    pub(crate) fn new() -> Self {
        Self { line: Mutex::new(HoldLine::new()) }
    }

    pub(crate) fn line(&self) -> MutexGuard<'_, HoldLine> {
        self.line.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
