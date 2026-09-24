mod activate;
mod counters;
mod eval;
mod exec;
mod input;
mod matching;
mod outcome;
mod state;
mod wheel;
mod world;

pub use counters::EngineCountersSnapshot;
pub use input::{EngineInput, InputEventKind};
pub use outcome::{ActivationOutcome, Effect, LightVerb, PartialReason};
pub use state::{CHAIN_WINDOW_MS, MAX_CHAIN_DEPTH, MAX_PENDING_CONTINUATIONS, MAX_TIMERS, MAX_VARS};
pub use world::{
    DeviceState, GroupState, HclTargetKey, HclTargetState, InputState, LampState, SunTimes,
    WallTime, WorldSnapshot,
};

use dali2rust_rules_model::{RuleSet, TriggerKind};

use activate::{activate, Mode};
use counters::{bump, EngineCounters};
use exec::ExecEnv;
use input::EngineInput as In;
use state::{ChainKey, EventCtx, Volatile};

pub struct Engine {
    rules: Option<RuleSet>,
    enabled: Vec<bool>,
    vol: Volatile,
    counters: EngineCounters,
    now_ms: u64,
}

impl Engine {
    #[must_use]
    pub fn new(now_ms: u64) -> Engine {
        Engine {
            rules: None,
            enabled: Vec::new(),
            vol: Volatile::default(),
            counters: EngineCounters::default(),
            now_ms,
        }
    }

    pub fn set_rules(&mut self, set: Option<RuleSet>, now_ms: u64) {
        self.now_ms = now_ms;
        let dropped = self.vol.continuations.len();
        self.vol.continuations.clear();
        self.counters.continuations_dropped = self
            .counters
            .continuations_dropped
            .saturating_add(dropped as u32);
        let empty = RuleSet::empty(0);
        let next = set.as_ref().unwrap_or(&empty);
        self.vol
            .retain_rule_names(&|name| next.rules.iter().any(|r| r.name == name));
        wheel::rebuild_every(&mut self.vol, next, now_ms);
        self.enabled = next.rules.iter().map(|r| r.enabled).collect();
        self.rules = set;
    }

    pub fn handle(&mut self, input: EngineInput<'_>, world: &WorldSnapshot) -> Vec<ActivationOutcome> {
        self.now_ms = world.now_ms;
        self.vol.prune_chain(world.now_ms);
        let mut out = Vec::new();
        let Some(rules) = self.rules.take() else {
            return out;
        };
        let mut env = ExecEnv {
            world,
            rules: &rules,
            enabled: &mut self.enabled,
            vol: &mut self.vol,
            counters: &mut self.counters,
        };
        dispatch(&mut env, &input, &mut out);
        self.rules = Some(rules);
        out
    }

    #[must_use]
    pub fn next_deadline_ms(&self) -> Option<u64> {
        wheel::next_deadline(&self.vol, self.rules.as_ref(), &self.enabled, self.now_ms)
    }

    #[must_use]
    pub fn watches_groups(&self) -> bool {
        self.rules.as_ref().is_some_and(|set| {
            set.rules.iter().any(|rule| {
                rule.triggers
                    .iter()
                    .any(|t| matches!(t, dali2rust_rules_model::Trigger::GroupBecomes { .. }))
            })
        })
    }

    #[must_use]
    pub fn counters(&self) -> EngineCountersSnapshot {
        self.counters.snapshot(
            self.vol.timers.len() as u32,
            self.rules.as_ref().map_or(0, |r| r.rules.len()) as u32,
            self.vol.vars.len() as u32,
        )
    }
}

fn dispatch(env: &mut ExecEnv<'_>, input: &In<'_>, out: &mut Vec<ActivationOutcome>) {
    match input {
        In::Tick => wheel::service_tick(env, out),
        In::RunRule { name, dry } => run_rule(env, name, *dry, out),
        other => on_external(env, other, out),
    }
}

fn run_rule(env: &mut ExecEnv<'_>, name: &str, dry: bool, out: &mut Vec<ActivationOutcome>) {
    let Some(idx) = env.rules.rules.iter().position(|r| r.name == name) else {
        return;
    };
    let enabled = env.enabled.get(idx).copied().unwrap_or(false);
    if !dry && !enabled {
        bump(&mut env.counters.suppressed_disabled);
        return;
    }
    let mode = if dry { Mode::Dry } else { Mode::Manual };
    activate(env, idx, TriggerKind::HttpTrigger, EventCtx::default(), 0, mode, out);
}

fn on_external(env: &mut ExecEnv<'_>, input: &In<'_>, out: &mut Vec<ActivationOutcome>) {
    maintain_hold_ledger(env, input);
    let depth = input_chain_depth(env, input);
    let ctx = event_ctx(input);
    for idx in wheel::name_order(env.rules) {
        let hit = matching::rule_matches(&env.rules.rules[idx], input, env.vol);
        let Some(kind) = hit else { continue };
        if !env.enabled.get(idx).copied().unwrap_or(false) {
            bump(&mut env.counters.suppressed_disabled);
            continue;
        }
        activate(env, idx, kind, ctx, depth, Mode::Auto, out);
    }
    maintain_edge_cache(env, input);
}

fn maintain_hold_ledger(env: &mut ExecEnv<'_>, input: &In<'_>) {
    let In::InputEvent {
        adapter_id,
        short_address: Some(sa),
        instance_number: Some(inst),
        kind,
        ..
    } = input
    else {
        return;
    };
    let key = (*adapter_id, *sa, *inst);
    match kind {
        InputEventKind::LongPressStart => {
            env.vol.hold_marks.insert(key, env.world.now_ms);
        }
        InputEventKind::LongPressStop
        | InputEventKind::Release
        | InputEventKind::ButtonFree => {
            env.vol.hold_marks.remove(&key);
        }
        _ => {}
    }
}

fn maintain_edge_cache(env: &mut ExecEnv<'_>, input: &In<'_>) {
    let In::InputEvent {
        adapter_id,
        short_address: Some(sa),
        instance_number: Some(inst),
        kind,
        value,
        ..
    } = input
    else {
        return;
    };
    if !matches!(
        kind,
        InputEventKind::LightCrossedAbove | InputEventKind::LightCrossedBelow
    ) {
        return;
    }
    let key = (*adapter_id, *sa, *inst);
    if *value == state::INPUT_VALUE_MASK_10BIT {
        env.vol.light_edge.remove(&key);
    } else {
        env.vol.light_edge.insert(key, *value);
    }
}

fn input_chain_depth(env: &ExecEnv<'_>, input: &In<'_>) -> u32 {
    let now = env.world.now_ms;
    match input {
        In::LampChanged {
            adapter_id, lamp_id, ..
        } => env
            .vol
            .chain_depth_for(&state::lamp_chain_keys(*adapter_id, *lamp_id), now),
        In::GroupChanged {
            adapter_id, group_id, ..
        } => env
            .vol
            .chain_depth_for(&state::group_chain_keys(*adapter_id, *group_id), now),
        In::SceneRecalled {
            adapter_id, scene_id,
        } => env.vol.chain_depth_for(
            &[
                ChainKey::Scene(*scene_id),
                ChainKey::AdapterWide(*adapter_id),
            ],
            now,
        ),
        In::HclOverride { target, .. } => {
            env.vol.chain_depth_for(&[ChainKey::Hcl(*target)], now)
        }
        _ => 0,
    }
}

fn event_ctx(input: &In<'_>) -> EventCtx {
    match input {
        In::InputEvent {
            adapter_id,
            short_address,
            instance_number,
            instance_groups,
            value,
            ..
        } => EventCtx {
            value: Some(i64::from(*value)),
            device: short_address.map(i64::from),
            instance: instance_number.map(i64::from),
            option: instance_groups[0].map(i64::from),
            source: match (short_address, instance_number) {
                (Some(sa), Some(inst)) => Some((*adapter_id, *sa, *inst)),
                _ => None,
            },
        },
        In::LampChanged { level, .. } => EventCtx {
            value: Some(i64::from(*level)),
            ..EventCtx::default()
        },
        In::GroupChanged { any_on, .. } => EventCtx {
            value: Some(i64::from(*any_on)),
            ..EventCtx::default()
        },
        In::SceneRecalled { scene_id, .. } => EventCtx {
            value: Some(i64::from(*scene_id)),
            ..EventCtx::default()
        },
        _ => EventCtx::default(),
    }
}
