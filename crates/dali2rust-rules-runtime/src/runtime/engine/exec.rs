use std::collections::VecDeque;

use dali2rust_rules_model::limits::{
    MAX_ACTIONS_EXPANDED, MAX_BLOCK_CALL_DEPTH, MAX_INSTANCE_GROUP, MAX_LEVEL, MAX_REPEAT,
    MAX_SCENE, MAX_VAR_TEXT_BYTES,
};
use dali2rust_rules_model::{
    Action, CctSpec, Condition, DefBlock, FlowAction, HclAction, InputAction, LevelSpec,
    LightAction, LightOp, LightTarget, RuleSet, SceneAction, StateAction, ValueExpr, VarValue,
};

use super::counters::{bump, EngineCounters};
use super::eval::{eval_condition, eval_value, EvalEnv, Unevaluable};
use super::outcome::{Effect, LightVerb, PartialReason};
use super::state::{
    ChainKey, Continuation, EventCtx, Volatile, DIM_HOLD_RESET_MS, MAX_PENDING_CONTINUATIONS,
    MAX_TIMERS, MAX_VARS,
};
use super::world::WorldSnapshot;

const FLATTEN_STEP_CAP: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FlatStep {
    Prim(Action),
    Wait(u32),
    Branch {
        condition: Condition,
        then_len: usize,
        else_len: usize,
    },
    AfterSched { delay_ms: u32, body: Vec<FlatStep> },
    Fail,
}

#[derive(Clone, Copy, Default)]
struct FlattenCtx {
    call_depth: usize,
    in_repeat: bool,
    in_conditional: bool,
    in_after: bool,
}

pub(crate) fn flatten_rule(actions: &[Action], blocks: &[DefBlock]) -> VecDeque<FlatStep> {
    let mut out = Vec::new();
    flatten_list(actions, blocks, &mut out, FlattenCtx::default());
    out.into()
}

fn flatten_list(actions: &[Action], blocks: &[DefBlock], out: &mut Vec<FlatStep>, ctx: FlattenCtx) {
    for action in actions {
        if out.len() >= FLATTEN_STEP_CAP {
            out.push(FlatStep::Fail);
            return;
        }
        flatten_action(action, blocks, out, ctx);
    }
}

fn flatten_action(action: &Action, blocks: &[DefBlock], out: &mut Vec<FlatStep>, ctx: FlattenCtx) {
    let Action::Flow(flow) = action else {
        out.push(FlatStep::Prim(action.clone()));
        return;
    };
    match flow {
        FlowAction::Wait { duration_ms } => out.push(FlatStep::Wait(duration_ms.0)),
        FlowAction::After { delay_ms, actions } => flatten_after(*delay_ms, actions, blocks, out, ctx),
        FlowAction::Call { block } => flatten_call(block, blocks, out, ctx),
        FlowAction::Repeat { count, actions } => flatten_repeat(*count, actions, blocks, out, ctx),
        FlowAction::Conditional {
            condition,
            then_actions,
            else_actions,
        } => flatten_branch(condition, then_actions, else_actions, blocks, out, ctx),
        _ => out.push(FlatStep::Prim(action.clone())),
    }
}

fn flatten_after(
    delay: dali2rust_rules_model::DurationMs,
    actions: &[Action],
    blocks: &[DefBlock],
    out: &mut Vec<FlatStep>,
    ctx: FlattenCtx,
) {
    if ctx.in_after {
        out.push(FlatStep::Fail);
        return;
    }
    let mut body = Vec::new();
    flatten_list(actions, blocks, &mut body, FlattenCtx { in_after: true, ..ctx });
    out.push(FlatStep::AfterSched { delay_ms: delay.0, body });
}

fn flatten_call(block: &str, blocks: &[DefBlock], out: &mut Vec<FlatStep>, ctx: FlattenCtx) {
    let next_depth = ctx.call_depth + 1;
    let Some(def) = blocks.iter().find(|b| b.name == block) else {
        out.push(FlatStep::Fail);
        return;
    };
    if next_depth > MAX_BLOCK_CALL_DEPTH {
        out.push(FlatStep::Fail);
        return;
    }
    flatten_list(&def.actions, blocks, out, FlattenCtx { call_depth: next_depth, ..ctx });
}

fn flatten_repeat(
    count: u8,
    actions: &[Action],
    blocks: &[DefBlock],
    out: &mut Vec<FlatStep>,
    ctx: FlattenCtx,
) {
    if ctx.in_repeat || count == 0 || count > MAX_REPEAT {
        out.push(FlatStep::Fail);
        return;
    }
    let mut body = Vec::new();
    flatten_list(actions, blocks, &mut body, FlattenCtx { in_repeat: true, ..ctx });
    for _ in 0..count {
        if out.len() + body.len() > FLATTEN_STEP_CAP {
            out.push(FlatStep::Fail);
            return;
        }
        out.extend(body.iter().cloned());
    }
}

fn flatten_branch(
    condition: &Condition,
    then_actions: &[Action],
    else_actions: &[Action],
    blocks: &[DefBlock],
    out: &mut Vec<FlatStep>,
    ctx: FlattenCtx,
) {
    if ctx.in_conditional {
        out.push(FlatStep::Fail);
        return;
    }
    let node = out.len();
    out.push(FlatStep::Branch {
        condition: condition.clone(),
        then_len: 0,
        else_len: 0,
    });
    let nested = FlattenCtx { in_conditional: true, ..ctx };
    flatten_list(then_actions, blocks, out, nested);
    let then_len = out.len() - node - 1;
    flatten_list(else_actions, blocks, out, nested);
    let else_len = out.len() - node - 1 - then_len;
    if let Some(FlatStep::Branch {
        then_len: t,
        else_len: e,
        ..
    }) = out.get_mut(node)
    {
        *t = then_len;
        *e = else_len;
    }
}

pub(crate) struct ExecEnv<'a> {
    pub world: &'a WorldSnapshot,
    pub rules: &'a RuleSet,
    pub enabled: &'a mut Vec<bool>,
    pub vol: &'a mut Volatile,
    pub counters: &'a mut EngineCounters,
}

pub(crate) struct Job {
    pub rule: String,
    pub trigger_kind: &'static str,
    pub depth: u32,
    pub dry: bool,
    pub hold_hcl: bool,
    pub ctx: EventCtx,
    pub budget_left: u32,
    pub steps: VecDeque<FlatStep>,
}

impl Job {
    pub fn budget_for_activation() -> u32 {
        MAX_ACTIONS_EXPANDED as u32
    }
}

pub(crate) struct RunOutput {
    pub effects: Vec<Effect>,
    pub partial: Option<PartialReason>,
}

fn note_partial(slot: &mut Option<PartialReason>, reason: PartialReason) {
    if slot.is_none() {
        *slot = Some(reason);
    }
}

pub(crate) fn run_steps(env: &mut ExecEnv<'_>, mut job: Job) -> RunOutput {
    let mut out = RunOutput {
        effects: Vec::new(),
        partial: None,
    };
    while let Some(step) = job.steps.pop_front() {
        if job.budget_left == 0 {
            note_partial(&mut out.partial, PartialReason::EffectBudget);
            break;
        }
        job.budget_left -= 1;
        match step {
            FlatStep::Prim(action) => run_prim(env, &mut job, &action, &mut out),
            FlatStep::Wait(ms) => {
                if run_wait(env, &mut job, ms, &mut out) {
                    break;
                }
            }
            FlatStep::Branch {
                condition,
                then_len,
                else_len,
            } => run_branch(env, &mut job, &condition, then_len, else_len, &mut out),
            FlatStep::AfterSched { delay_ms, body } => {
                run_after(env, &mut job, delay_ms, body, &mut out);
            }
            FlatStep::Fail => {
                bump(&mut env.counters.actions_failed);
                note_partial(&mut out.partial, PartialReason::ConditionUnevaluable);
            }
        }
    }
    out
}

fn run_wait(env: &mut ExecEnv<'_>, job: &mut Job, ms: u32, out: &mut RunOutput) -> bool {
    if job.dry {
        return false;
    }
    let steps: Vec<FlatStep> = job.steps.iter().cloned().collect();
    schedule_continuation(env, job, ms, steps, out);
    true
}

fn run_after(
    env: &mut ExecEnv<'_>,
    job: &mut Job,
    delay_ms: u32,
    body: Vec<FlatStep>,
    out: &mut RunOutput,
) {
    if job.dry {
        job.steps.extend(body);
        return;
    }
    schedule_continuation(env, job, delay_ms, body, out);
}

fn schedule_continuation(
    env: &mut ExecEnv<'_>,
    job: &Job,
    delay_ms: u32,
    steps: Vec<FlatStep>,
    out: &mut RunOutput,
) {
    if env.vol.continuations.len() >= MAX_PENDING_CONTINUATIONS {
        bump(&mut env.counters.continuations_dropped);
        note_partial(&mut out.partial, PartialReason::EffectBudget);
        return;
    }
    let seq = env.vol.next_seq();
    env.vol.continuations.push(Continuation {
        seq,
        due_ms: env.world.now_ms.saturating_add(u64::from(delay_ms)),
        rule: job.rule.clone(),
        trigger_kind: job.trigger_kind,
        depth: job.depth,
        budget_left: job.budget_left,
        ctx: job.ctx,
        steps,
    });
    bump(&mut env.counters.continuations_scheduled);
}

fn run_branch(
    env: &mut ExecEnv<'_>,
    job: &mut Job,
    condition: &Condition,
    then_len: usize,
    else_len: usize,
    out: &mut RunOutput,
) {
    let verdict = eval_in(env, &job.ctx, condition);
    let then_region: Vec<FlatStep> = drain_front(&mut job.steps, then_len);
    let else_region: Vec<FlatStep> = drain_front(&mut job.steps, else_len);
    match verdict {
        Ok(true) => prepend(&mut job.steps, then_region),
        Ok(false) => prepend(&mut job.steps, else_region),
        Err(Unevaluable) => {
            bump(&mut env.counters.actions_failed);
            note_partial(&mut out.partial, PartialReason::ConditionUnevaluable);
        }
    }
}

fn drain_front(steps: &mut VecDeque<FlatStep>, len: usize) -> Vec<FlatStep> {
    let take = len.min(steps.len());
    steps.drain(..take).collect()
}

fn prepend(steps: &mut VecDeque<FlatStep>, region: Vec<FlatStep>) {
    for step in region.into_iter().rev() {
        steps.push_front(step);
    }
}

fn eval_in(env: &ExecEnv<'_>, ctx: &EventCtx, condition: &Condition) -> Result<bool, Unevaluable> {
    let eval_env = EvalEnv {
        world: env.world,
        vol: env.vol,
        rules: env.rules,
        enabled: env.enabled,
        ctx,
    };
    eval_condition(&eval_env, condition)
}

fn value_in(env: &ExecEnv<'_>, ctx: &EventCtx, expr: &ValueExpr) -> Result<i64, Unevaluable> {
    let eval_env = EvalEnv {
        world: env.world,
        vol: env.vol,
        rules: env.rules,
        enabled: env.enabled,
        ctx,
    };
    eval_value(&eval_env, expr)
}

fn run_prim(env: &mut ExecEnv<'_>, job: &mut Job, action: &Action, out: &mut RunOutput) {
    let executed = match action {
        Action::Light(light) => prim_light(env, job, light, out),
        Action::Scene(scene) => prim_scene(env, job, scene, out),
        Action::Hcl(hcl) => Ok(Some(prim_hcl(hcl))),
        Action::Input(input) => prim_input(env, job, input),
        Action::Flow(flow) => prim_timer(env, job, flow).map(|()| None),
        Action::State(state) => prim_state(env, job, state),
    };
    match executed {
        Ok(Some(effect)) => out.effects.push(effect),
        Ok(None) => {}
        Err(Unevaluable) => {
            bump(&mut env.counters.actions_failed);
            note_partial(&mut out.partial, PartialReason::ConditionUnevaluable);
        }
    }
}

fn clamp_level(value: i64) -> u8 {
    value.clamp(0, i64::from(MAX_LEVEL)) as u8
}

fn clamp_kelvin(value: i64) -> u16 {
    value.clamp(0, i64::from(u16::MAX)) as u16
}

fn clamp_step(value: i64) -> i16 {
    value.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

fn prim_light(
    env: &mut ExecEnv<'_>,
    job: &mut Job,
    light: &LightAction,
    out: &mut RunOutput,
) -> Result<Option<Effect>, Unevaluable> {
    let Some(verb) = light_verb(env, job, &light.op, &light.target)? else {
        return Ok(None);
    };
    let _ = out;
    Ok(Some(Effect::Light {
        target: light.target,
        verb,
        hold_hcl: light.hold_hcl.unwrap_or(job.hold_hcl),
    }))
}

fn light_verb(
    env: &mut ExecEnv<'_>,
    job: &Job,
    op: &LightOp,
    target: &LightTarget,
) -> Result<Option<LightVerb>, Unevaluable> {
    match op {
        LightOp::On { level } => Ok(Some(LightVerb::On {
            level: opt_level(env, job, level)?,
        })),
        LightOp::Off => Ok(Some(LightVerb::Off)),
        LightOp::Toggle { level } => toggle_verb(env, job, level, target).map(Some),
        LightOp::Level { level } => level_verb(env, job, level, target).map(Some),
        LightOp::Dim { delta } => Ok(Some(LightVerb::Dim { delta: *delta })),
        LightOp::DimHold { rate_per_s } => dim_hold_verb(env, job, *rate_per_s),
        LightOp::Cct { cct } => cct_verb(env, job, cct, target).map(Some),
        LightOp::Xy { x_1e4, y_1e4 } => Ok(Some(LightVerb::Xy {
            x_1e4: *x_1e4,
            y_1e4: *y_1e4,
        })),
        LightOp::Rgb { r, g, b } => Ok(Some(LightVerb::Rgb {
            r: *r,
            g: *g,
            b: *b,
        })),
        LightOp::LastActive => Ok(Some(LightVerb::LastActive)),
        LightOp::StopFade => Ok(Some(LightVerb::StopFade)),
    }
}

fn opt_level(
    env: &ExecEnv<'_>,
    job: &Job,
    level: &Option<ValueExpr>,
) -> Result<Option<u8>, Unevaluable> {
    match level {
        None => Ok(None),
        Some(expr) => Ok(Some(clamp_level(value_in(env, &job.ctx, expr)?))),
    }
}

fn toggle_verb(
    env: &ExecEnv<'_>,
    job: &Job,
    level: &Option<ValueExpr>,
    target: &LightTarget,
) -> Result<LightVerb, Unevaluable> {
    let currently_on = match target {
        LightTarget::Lamp(l) => env.world.lamp(l.adapter_id, l.id).ok_or(Unevaluable)?.is_on,
        LightTarget::Group(g) => env.world.group(g.adapter_id, g.id).ok_or(Unevaluable)?.any_on,
        LightTarget::Broadcast { adapter_id } => env.world.any_lamp_on(*adapter_id),
    };
    if currently_on {
        Ok(LightVerb::Off)
    } else {
        Ok(LightVerb::On {
            level: opt_level(env, job, level)?,
        })
    }
}

fn level_verb(
    env: &ExecEnv<'_>,
    job: &Job,
    level: &LevelSpec,
    target: &LightTarget,
) -> Result<LightVerb, Unevaluable> {
    match level {
        LevelSpec::Absolute(expr) => Ok(LightVerb::Level {
            level: clamp_level(value_in(env, &job.ctx, expr)?),
        }),
        LevelSpec::Relative { delta } => match target {
            LightTarget::Lamp(l) => {
                let lamp = env.world.lamp(l.adapter_id, l.id).ok_or(Unevaluable)?;
                Ok(LightVerb::Level {
                    level: clamp_level(i64::from(lamp.level) + i64::from(*delta)),
                })
            }
            _ => Ok(LightVerb::LevelRelative { delta: *delta }),
        },
    }
}

fn cct_verb(
    env: &ExecEnv<'_>,
    job: &Job,
    cct: &CctSpec,
    target: &LightTarget,
) -> Result<LightVerb, Unevaluable> {
    match cct {
        CctSpec::Absolute(expr) => Ok(LightVerb::Cct {
            kelvin: clamp_kelvin(value_in(env, &job.ctx, expr)?),
        }),
        CctSpec::Relative { delta_k } => match target {
            LightTarget::Lamp(l) => match env.world.lamp(l.adapter_id, l.id).and_then(|s| s.cct_kelvin) {
                Some(cur) => Ok(LightVerb::Cct {
                    kelvin: clamp_kelvin(i64::from(cur) + i64::from(*delta_k)),
                }),
                None => Ok(LightVerb::CctRelative { delta_k: *delta_k }),
            },
            _ => Ok(LightVerb::CctRelative { delta_k: *delta_k }),
        },
    }
}

fn dim_hold_verb(
    env: &mut ExecEnv<'_>,
    job: &Job,
    rate_per_s: i16,
) -> Result<Option<LightVerb>, Unevaluable> {
    let source = job.ctx.source.ok_or(Unevaluable)?;
    let now = env.world.now_ms;
    let base = match env.vol.hold_marks.get(&source) {
        Some(mark) if now.saturating_sub(*mark) <= DIM_HOLD_RESET_MS => *mark,
        _ => now,
    };
    let elapsed = now.saturating_sub(base);
    let delta = clamp_step(i64::from(rate_per_s).saturating_mul(elapsed as i64) / 1_000);
    if !job.dry {
        env.vol.hold_marks.insert(source, if delta == 0 { base } else { now });
    }
    Ok((delta != 0).then_some(LightVerb::Dim { delta }))
}

fn prim_scene(
    env: &mut ExecEnv<'_>,
    job: &mut Job,
    scene: &SceneAction,
    out: &mut RunOutput,
) -> Result<Option<Effect>, Unevaluable> {
    let _ = out;
    match scene {
        SceneAction::Recall { scene, target } => Ok(Some(Effect::SceneRecall {
            scene: scene_number(env, job, scene)?,
            target: *target,
            hold_hcl: job.hold_hcl,
        })),
        SceneAction::Apply { scene } => Ok(Some(Effect::SceneApply {
            scene: scene_number(env, job, scene)?,
            hold_hcl: job.hold_hcl,
        })),
        SceneAction::Cycle { scenes } => cycle_effect(env, job, scenes),
    }
}

fn scene_number(env: &ExecEnv<'_>, job: &Job, expr: &ValueExpr) -> Result<u8, Unevaluable> {
    let value = value_in(env, &job.ctx, expr)?;
    if (0..=i64::from(MAX_SCENE)).contains(&value) {
        Ok(value as u8)
    } else {
        Err(Unevaluable)
    }
}

fn cycle_effect(
    env: &mut ExecEnv<'_>,
    job: &Job,
    scenes: &[u8],
) -> Result<Option<Effect>, Unevaluable> {
    if scenes.is_empty() {
        return Err(Unevaluable);
    }
    let pos = env.vol.cycle_pos.get(&job.rule).copied().unwrap_or(0) % scenes.len();
    if !job.dry {
        env.vol.cycle_pos.insert(job.rule.clone(), (pos + 1) % scenes.len());
    }
    Ok(Some(Effect::SceneRecall {
        scene: scenes[pos],
        target: None,
        hold_hcl: job.hold_hcl,
    }))
}

fn prim_hcl(hcl: &HclAction) -> Effect {
    match hcl {
        HclAction::Resume { target } => Effect::HclResume { target: *target },
        HclAction::Hold { target } => Effect::HclHold { target: *target },
        HclAction::Enable { schedule } => Effect::HclSchedule {
            schedule: schedule.clone(),
            enabled: true,
        },
        HclAction::Disable { schedule } => Effect::HclSchedule {
            schedule: schedule.clone(),
            enabled: false,
        },
    }
}

fn prim_input(
    env: &ExecEnv<'_>,
    job: &Job,
    input: &InputAction,
) -> Result<Option<Effect>, Unevaluable> {
    match input {
        InputAction::FeedbackOn { input } => Ok(Some(Effect::InputFeedback {
            input: *input,
            on: true,
        })),
        InputAction::FeedbackOff { input } => Ok(Some(Effect::InputFeedback {
            input: *input,
            on: false,
        })),
        InputAction::PanelSelect {
            adapter_id,
            group,
            selected,
        } => {
            let value = value_in(env, &job.ctx, selected)?;
            if !(0..=i64::from(MAX_INSTANCE_GROUP)).contains(&value) {
                return Err(Unevaluable);
            }
            Ok(Some(Effect::PanelSelect {
                adapter_id: *adapter_id,
                group: *group,
                selected: value as u8,
            }))
        }
        InputAction::CancelHold { input } => Ok(Some(Effect::CancelHold { input: *input })),
        InputAction::CatchMovement { input } => Ok(Some(Effect::CatchMovement { input: *input })),
    }
}

fn prim_timer(env: &mut ExecEnv<'_>, job: &Job, flow: &FlowAction) -> Result<(), Unevaluable> {
    match flow {
        FlowAction::TimerStart { timer, duration_ms } => {
            if env.vol.timer(timer).is_some() {
                return Ok(());
            }
            arm_timer(env, job, timer, duration_ms.0)
        }
        FlowAction::TimerRestart { timer, duration_ms } => {
            if !job.dry {
                env.vol.cancel_timer(timer);
            }
            arm_timer(env, job, timer, duration_ms.0)
        }
        FlowAction::TimerCancel { timer } => {
            if !job.dry {
                env.vol.cancel_timer(timer);
            }
            Ok(())
        }
        _ => Err(Unevaluable),
    }
}

fn arm_timer(env: &mut ExecEnv<'_>, job: &Job, name: &str, duration_ms: u32) -> Result<(), Unevaluable> {
    if job.dry {
        return Ok(());
    }
    if env.vol.timers.len() >= MAX_TIMERS {
        return Err(Unevaluable);
    }
    env.vol.timers.push(super::state::NamedTimer {
        name: name.to_owned(),
        due_ms: env.world.now_ms.saturating_add(u64::from(duration_ms)),
    });
    Ok(())
}

fn prim_state(
    env: &mut ExecEnv<'_>,
    job: &Job,
    state: &StateAction,
) -> Result<Option<Effect>, Unevaluable> {
    match state {
        StateAction::VarSet { name, value } => var_set(env, job, name, value.clone()).map(|()| None),
        StateAction::VarAdd { name, delta } => var_add(env, job, name, delta).map(|()| None),
        StateAction::RuleEnable { rule } => set_rule_bit(env, job, rule, true).map(|()| None),
        StateAction::RuleDisable { rule } => set_rule_bit(env, job, rule, false).map(|()| None),
        StateAction::MqttPublish {
            topic,
            payload,
            retain,
        } => Ok(Some(Effect::MqttPublish {
            topic: topic.clone(),
            payload: payload.clone(),
            retain: *retain,
        })),
        StateAction::Log { text } => Ok(Some(Effect::Log { text: text.clone() })),
        StateAction::StatCount { name } => Ok(Some(Effect::StatCount { name: name.clone() })),
    }
}

fn var_set(env: &mut ExecEnv<'_>, job: &Job, name: &str, value: VarValue) -> Result<(), Unevaluable> {
    if let VarValue::Text(text) = &value {
        if text.len() > MAX_VAR_TEXT_BYTES {
            return Err(Unevaluable);
        }
    }
    if job.dry {
        return Ok(());
    }
    if !env.vol.vars.contains_key(name) && env.vol.vars.len() >= MAX_VARS {
        return Err(Unevaluable);
    }
    env.vol.vars.insert(name.to_owned(), value);
    Ok(())
}

fn var_add(env: &mut ExecEnv<'_>, job: &Job, name: &str, delta: &ValueExpr) -> Result<(), Unevaluable> {
    let step = value_in(env, &job.ctx, delta)?;
    let next = match env.vol.vars.get(name) {
        Some(VarValue::Int(cur)) => cur.saturating_add(step),
        Some(VarValue::Text(_)) => return Err(Unevaluable),
        None => step,
    };
    var_set(env, job, name, VarValue::Int(next))
}

fn set_rule_bit(env: &mut ExecEnv<'_>, job: &Job, rule: &str, value: bool) -> Result<(), Unevaluable> {
    let idx = env
        .rules
        .rules
        .iter()
        .position(|r| r.name == rule)
        .ok_or(Unevaluable)?;
    if job.dry {
        return Ok(());
    }
    flip_rule_bit(env.enabled, env.vol, env.counters, idx, rule, value);
    Ok(())
}

pub(crate) fn flip_rule_bit(
    enabled: &mut [bool],
    vol: &mut Volatile,
    counters: &mut EngineCounters,
    idx: usize,
    rule: &str,
    value: bool,
) {
    if let Some(bit) = enabled.get_mut(idx) {
        *bit = value;
    }
    if !value {
        let before = vol.continuations.len();
        vol.continuations.retain(|c| c.rule != rule);
        let dropped = before - vol.continuations.len();
        counters.continuations_dropped = counters
            .continuations_dropped
            .saturating_add(dropped as u32);
    }
}

pub(crate) fn record_effect_chain(vol: &mut Volatile, effect: &Effect, depth: u32, now_ms: u64) {
    let mut push = |key: Option<ChainKey>| {
        if let Some(key) = key {
            vol.record_chain(key, depth, now_ms);
        }
    };
    match effect {
        Effect::Light { target, .. } => {
            let [a, b] = super::state::target_chain_keys(target);
            push(a);
            push(b);
        }
        Effect::SceneRecall { scene, target, .. } => {
            push(Some(ChainKey::Scene(*scene)));
            if let Some(target) = target {
                let [a, b] = super::state::target_chain_keys(target);
                push(a);
                push(b);
            }
        }
        Effect::SceneApply { scene, .. } => push(Some(ChainKey::Scene(*scene))),
        Effect::HclResume { target } | Effect::HclHold { target } => {
            push(Some(ChainKey::Hcl(*target)));
        }
        _ => {}
    }
}
