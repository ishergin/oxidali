use dali2rust_platform::small_sort::insertion_sort_by;
use dali2rust_rules_model::{Rule, RuleSet, SolarEvent, Trigger, TriggerKind};

use super::activate::{activate, Mode};
use super::counters::bump;
use super::eval::solar_minute;
use super::exec::{run_steps, ExecEnv, Job};
use super::outcome::ActivationOutcome;
use super::state::{Continuation, EventCtx, EverySlot, Volatile};

const WEEK_MINUTES: u32 = 7 * 24 * 60;
const WALL_CATCHUP_CAP_MIN: u32 = 60;
pub(crate) const WALL_POLL_MS: u64 = 1_000;

pub(crate) fn name_order(rules: &RuleSet) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rules.rules.len()).collect();
    insertion_sort_by(&mut order, |a, b| rules.rules[*a].name > rules.rules[*b].name);
    order
}

pub(crate) fn service_tick(env: &mut ExecEnv<'_>, out: &mut Vec<ActivationOutcome>) {
    fire_continuations(env, out);
    fire_timers(env, out);
    fire_every(env, out);
    fire_wall_schedules(env, out);
}

fn fire_continuations(env: &mut ExecEnv<'_>, out: &mut Vec<ActivationOutcome>) {
    let now = env.world.now_ms;
    let mut due: Vec<Continuation> = Vec::new();
    env.vol.continuations.retain(|c| {
        if c.due_ms <= now {
            due.push(c.clone());
            false
        } else {
            true
        }
    });
    due.sort_unstable_by_key(|c| (c.due_ms, c.seq));
    for cont in due {
        bump(&mut env.counters.continuations_fired);
        out.push(resume(env, cont));
    }
}

fn resume(env: &mut ExecEnv<'_>, cont: Continuation) -> ActivationOutcome {
    let job = Job {
        rule: cont.rule.clone(),
        trigger_kind: cont.trigger_kind,
        depth: cont.depth,
        dry: false,
        hold_hcl: rule_hold_hcl(env.rules, &cont.rule),
        ctx: cont.ctx,
        budget_left: cont.budget_left,
        steps: cont.steps.into(),
    };
    let run = run_steps(env, job);
    super::activate::finish_wet(env, &run, cont.depth, false);
    ActivationOutcome {
        rule: cont.rule,
        dry: false,
        effects: run.effects,
        partial: run.partial,
        trigger_kind: cont.trigger_kind,
    }
}

fn rule_hold_hcl(rules: &RuleSet, name: &str) -> bool {
    rules.rule(name).is_none_or(|r| r.hold_hcl)
}

fn fire_timers(env: &mut ExecEnv<'_>, out: &mut Vec<ActivationOutcome>) {
    let now = env.world.now_ms;
    let mut due: Vec<super::state::NamedTimer> = Vec::new();
    env.vol.timers.retain(|t| {
        if t.due_ms <= now {
            due.push(t.clone());
            false
        } else {
            true
        }
    });
    insertion_sort_by(&mut due, |a, b| (a.due_ms, &a.name) > (b.due_ms, &b.name));
    for timer in due {
        fire_one_timer(env, &timer.name, out);
    }
}

fn fire_one_timer(env: &mut ExecEnv<'_>, name: &str, out: &mut Vec<ActivationOutcome>) {
    for idx in name_order(env.rules) {
        if !is_enabled(env, idx) {
            continue;
        }
        let fired = env.rules.rules[idx]
            .triggers
            .iter()
            .any(|t| matches!(t, Trigger::TimerFires { timer } if timer == name));
        if fired {
            activate(env, idx, TriggerKind::TimerFires, EventCtx::default(), 0, Mode::Auto, out);
        }
    }
}

fn is_enabled(env: &ExecEnv<'_>, idx: usize) -> bool {
    env.enabled.get(idx).copied().unwrap_or(false)
}

fn fire_every(env: &mut ExecEnv<'_>, out: &mut Vec<ActivationOutcome>) {
    let now = env.world.now_ms;
    let due: Vec<(String, usize)> = env
        .vol
        .every
        .iter_mut()
        .filter(|slot| slot.due_ms <= now)
        .map(|slot| {
            slot.due_ms = now.saturating_add(u64::from(slot.period_ms));
            (slot.rule.clone(), slot.trigger_idx)
        })
        .collect();
    for (rule_name, _) in due {
        let Some(idx) = env.rules.rules.iter().position(|r| r.name == rule_name) else {
            continue;
        };
        if is_enabled(env, idx) {
            activate(env, idx, TriggerKind::Every, EventCtx::default(), 0, Mode::Auto, out);
        }
    }
}

pub(crate) fn rebuild_every(vol: &mut Volatile, rules: &RuleSet, now_ms: u64) {
    let mut slots: Vec<EverySlot> = Vec::new();
    for rule in &rules.rules {
        for (trigger_idx, trigger) in rule.triggers.iter().enumerate() {
            let Trigger::Every { period_ms } = trigger else {
                continue;
            };
            let kept = vol.every.iter().find(|s| {
                s.rule == rule.name && s.trigger_idx == trigger_idx && s.period_ms == period_ms.0
            });
            slots.push(EverySlot {
                rule: rule.name.clone(),
                trigger_idx,
                period_ms: period_ms.0,
                due_ms: kept.map_or(now_ms.saturating_add(u64::from(period_ms.0)), |s| s.due_ms),
            });
        }
    }
    insertion_sort_by(&mut slots, |a, b| (&a.rule, a.trigger_idx) > (&b.rule, b.trigger_idx));
    vol.every = slots;
}

fn week_min(wall: &super::world::WallTime) -> u32 {
    u32::from(wall.weekday) * 24 * 60 + u32::from(wall.minutes_of_day)
}

fn cyclic_dist(from: u32, to: u32) -> u32 {
    (to + WEEK_MINUTES - from) % WEEK_MINUTES
}

fn fire_wall_schedules(env: &mut ExecEnv<'_>, out: &mut Vec<ActivationOutcome>) {
    let Some(wall) = env.world.wall else {
        if has_wall_triggers(env) {
            bump(&mut env.counters.ticks_time_unsynced);
        }
        env.vol.prev_week_min = None;
        return;
    };
    let now_min = week_min(&wall);
    let prev = env.vol.prev_week_min.replace(now_min);
    let Some(prev) = prev else {
        return;
    };
    if !has_wall_triggers(env) {
        return;
    }
    let window = cyclic_dist(prev, now_min).min(WALL_CATCHUP_CAP_MIN);
    if window == 0 {
        return;
    }
    let base = (now_min + WEEK_MINUTES - window) % WEEK_MINUTES;
    fire_due_wall_rules(env, base, window, out);
}

fn has_wall_triggers(env: &ExecEnv<'_>) -> bool {
    env.rules.rules.iter().enumerate().any(|(idx, rule)| {
        is_enabled(env, idx)
            && rule
                .triggers
                .iter()
                .any(|t| matches!(t, Trigger::AtTime { .. } | Trigger::AtSolar { .. }))
    })
}

fn fire_due_wall_rules(env: &mut ExecEnv<'_>, base: u32, window: u32, out: &mut Vec<ActivationOutcome>) {
    let mut solar_unsynced = false;
    for idx in name_order(env.rules) {
        if !is_enabled(env, idx) {
            continue;
        }
        let due = due_wall_kind(env, &env.rules.rules[idx], base, window, &mut solar_unsynced);
        if let Some(kind) = due {
            activate(env, idx, kind, EventCtx::default(), 0, Mode::Auto, out);
        }
    }
    if solar_unsynced {
        bump(&mut env.counters.ticks_time_unsynced);
    }
}

fn due_wall_kind(
    env: &ExecEnv<'_>,
    rule: &Rule,
    base: u32,
    window: u32,
    solar_unsynced: &mut bool,
) -> Option<TriggerKind> {
    for trigger in &rule.triggers {
        match trigger {
            Trigger::AtTime { time, days } => {
                let minute = u32::from(time.hour) * 60 + u32::from(time.minute);
                let due = dali2rust_rules_model::Weekday::ALL
                    .into_iter()
                    .filter(|d| days.contains(*d))
                    .map(|d| u32::from(d.index()) * 24 * 60 + minute)
                    .any(|target| in_window(base, window, target));
                if due {
                    return Some(TriggerKind::AtTime);
                }
            }
            Trigger::AtSolar { event, offset_ms } => {
                match solar_target_minute(env, *event, *offset_ms) {
                    Some(minute) => {
                        let due = (0..7u32)
                            .map(|d| d * 24 * 60 + minute)
                            .any(|target| in_window(base, window, target));
                        if due {
                            return Some(TriggerKind::AtSolar);
                        }
                    }
                    None => *solar_unsynced = true,
                }
            }
            _ => {}
        }
    }
    None
}

fn solar_target_minute(env: &ExecEnv<'_>, event: SolarEvent, offset_ms: i32) -> Option<u32> {
    env.world
        .sun
        .as_ref()
        .map(|sun| u32::from(solar_minute(sun, event, offset_ms)))
}

fn in_window(base: u32, window: u32, target: u32) -> bool {
    let dist = cyclic_dist(base, target);
    dist >= 1 && dist <= window
}

pub(crate) fn next_deadline(
    vol: &Volatile,
    rules: Option<&RuleSet>,
    enabled: &[bool],
    now_ms: u64,
) -> Option<u64> {
    let mut next: Option<u64> = None;
    let mut consider = |candidate: u64| {
        next = Some(next.map_or(candidate, |cur| cur.min(candidate)));
    };
    for c in &vol.continuations {
        consider(c.due_ms);
    }
    for t in &vol.timers {
        consider(t.due_ms);
    }
    for e in &vol.every {
        consider(e.due_ms);
    }
    if let Some(rules) = rules {
        if wall_triggers_exist(rules, enabled) {
            consider(now_ms.saturating_add(WALL_POLL_MS));
        }
    }
    next
}

fn wall_triggers_exist(rules: &RuleSet, enabled: &[bool]) -> bool {
    rules.rules.iter().enumerate().any(|(idx, rule)| {
        enabled.get(idx).copied().unwrap_or(false)
            && rule
                .triggers
                .iter()
                .any(|t| matches!(t, Trigger::AtTime { .. } | Trigger::AtSolar { .. }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyclic_distance_wraps_the_week() {
        assert_eq!(cyclic_dist(WEEK_MINUTES - 1, 0), 1);
        assert_eq!(cyclic_dist(0, 0), 0);
        assert_eq!(cyclic_dist(10, 40), 30);
        assert_eq!(cyclic_dist(40, 10), WEEK_MINUTES - 30);
    }

    #[test]
    fn window_membership_is_exclusive_of_base_and_inclusive_of_end() {
        assert!(!in_window(100, 5, 100));
        assert!(in_window(100, 5, 101));
        assert!(in_window(100, 5, 105));
        assert!(!in_window(100, 5, 106));
        assert!(in_window(WEEK_MINUTES - 2, 4, 1));
    }
}
