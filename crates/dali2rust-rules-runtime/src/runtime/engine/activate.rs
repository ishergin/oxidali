use dali2rust_rules_model::TriggerKind;

use super::counters::bump;
use super::eval::{eval_condition, EvalEnv, Unevaluable};
use super::exec::{flatten_rule, record_effect_chain, run_steps, ExecEnv, Job};
use super::outcome::{ActivationOutcome, PartialReason};
use super::state::{EventCtx, MAX_CHAIN_DEPTH};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Auto,
    Manual,
    Dry,
}

pub(crate) fn activate(
    env: &mut ExecEnv<'_>,
    rule_idx: usize,
    kind: TriggerKind,
    ctx: EventCtx,
    depth: u32,
    mode: Mode,
    out: &mut Vec<ActivationOutcome>,
) {
    let Some(rule) = env.rules.rules.get(rule_idx) else {
        return;
    };
    let name = rule.name.clone();
    if mode == Mode::Auto && in_cooldown(env, &name, rule.cooldown_ms) {
        bump(&mut env.counters.suppressed_cooldown);
        return;
    }
    if depth > MAX_CHAIN_DEPTH {
        bump(&mut env.counters.chain_depth_exceeded);
        out.push(refusal(name, mode, kind, PartialReason::ChainDepth));
        return;
    }
    match conditions_hold(env, rule_idx, &ctx) {
        Ok(true) => {}
        Ok(false) => {
            bump(&mut env.counters.conditions_rejected);
            return;
        }
        Err(Unevaluable) => {
            note_wet_partial(env, mode);
            out.push(refusal(name, mode, kind, PartialReason::ConditionUnevaluable));
            return;
        }
    }
    out.push(execute(env, rule_idx, name, kind, ctx, depth, mode));
}

fn in_cooldown(env: &ExecEnv<'_>, name: &str, cooldown_ms: u32) -> bool {
    if cooldown_ms == 0 {
        return false;
    }
    match env.vol.last_activation_ms.get(name) {
        Some(at) => env.world.now_ms.saturating_sub(*at) < u64::from(cooldown_ms),
        None => false,
    }
}

fn conditions_hold(
    env: &ExecEnv<'_>,
    rule_idx: usize,
    ctx: &EventCtx,
) -> Result<bool, Unevaluable> {
    let rule = &env.rules.rules[rule_idx];
    let eval_env = EvalEnv {
        world: env.world,
        vol: env.vol,
        rules: env.rules,
        enabled: env.enabled,
        ctx,
    };
    for condition in &rule.conditions {
        if !eval_condition(&eval_env, condition)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn refusal(
    rule: String,
    mode: Mode,
    kind: TriggerKind,
    reason: PartialReason,
) -> ActivationOutcome {
    ActivationOutcome {
        rule,
        dry: mode == Mode::Dry,
        effects: Vec::new(),
        partial: Some(reason),
        trigger_kind: kind.name(),
    }
}

fn note_wet_partial(env: &mut ExecEnv<'_>, mode: Mode) {
    if mode != Mode::Dry {
        bump(&mut env.counters.partial_outcomes);
    }
}

fn execute(
    env: &mut ExecEnv<'_>,
    rule_idx: usize,
    name: String,
    kind: TriggerKind,
    ctx: EventCtx,
    depth: u32,
    mode: Mode,
) -> ActivationOutcome {
    let rule = &env.rules.rules[rule_idx];
    let dry = mode == Mode::Dry;
    let job = Job {
        rule: name.clone(),
        trigger_kind: kind.name(),
        depth,
        dry,
        hold_hcl: rule.hold_hcl,
        ctx,
        budget_left: Job::budget_for_activation(),
        steps: flatten_rule(&rule.actions, &env.rules.blocks),
    };
    if dry {
        bump(&mut env.counters.activations_dry);
    } else {
        bump(&mut env.counters.activations_total);
        env.vol.last_activation_ms.insert(name.clone(), env.world.now_ms);
    }
    let run = run_steps(env, job);
    finish_wet(env, &run, depth, dry);
    ActivationOutcome {
        rule: name,
        dry,
        effects: run.effects,
        partial: run.partial,
        trigger_kind: kind.name(),
    }
}

pub(crate) fn finish_wet(env: &mut ExecEnv<'_>, run: &super::exec::RunOutput, depth: u32, dry: bool) {
    if dry {
        return;
    }
    for effect in &run.effects {
        record_effect_chain(env.vol, effect, depth, env.world.now_ms);
    }
    env.counters.effects_emitted = env
        .counters
        .effects_emitted
        .saturating_add(run.effects.len() as u32);
    if run.partial.is_some() {
        bump(&mut env.counters.partial_outcomes);
    }
}
