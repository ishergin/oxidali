use std::collections::BTreeMap;

use dali2rust_rules_model::{LightTarget, VarValue};

use super::exec::FlatStep;
use super::world::HclTargetKey;

pub const MAX_CHAIN_DEPTH: u32 = 4;
pub const CHAIN_WINDOW_MS: u64 = 2_000;
pub const MAX_PENDING_CONTINUATIONS: usize = 32;
pub const MAX_TIMERS: usize = 16;
pub const MAX_VARS: usize = 16;
pub const DIM_HOLD_RESET_MS: u64 = 2_000;
pub const INPUT_VALUE_MASK_10BIT: u16 = 0x3FF;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EventCtx {
    pub value: Option<i64>,
    pub device: Option<i64>,
    pub instance: Option<i64>,
    pub option: Option<i64>,
    pub source: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChainKey {
    Lamp(u8, u16),
    Group(u8, u16),
    AdapterWide(u8),
    Scene(u8),
    Hcl(HclTargetKey),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChainEntry {
    pub key: ChainKey,
    pub depth: u32,
    pub at_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct NamedTimer {
    pub name: String,
    pub due_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct Continuation {
    pub seq: u64,
    pub due_ms: u64,
    pub rule: String,
    pub trigger_kind: &'static str,
    pub depth: u32,
    pub budget_left: u32,
    pub ctx: EventCtx,
    pub steps: Vec<FlatStep>,
}

#[derive(Debug, Clone)]
pub(crate) struct EverySlot {
    pub rule: String,
    pub trigger_idx: usize,
    pub period_ms: u32,
    pub due_ms: u64,
}

#[derive(Debug, Default)]
pub(crate) struct Volatile {
    pub last_activation_ms: BTreeMap<String, u64>,
    pub cycle_pos: BTreeMap<String, usize>,
    pub vars: BTreeMap<String, VarValue>,
    pub timers: Vec<NamedTimer>,
    pub continuations: Vec<Continuation>,
    pub every: Vec<EverySlot>,
    pub chain: Vec<ChainEntry>,
    pub hold_marks: BTreeMap<(u8, u8, u8), u64>,
    pub light_edge: BTreeMap<(u8, u8, u8), u16>,
    pub prev_week_min: Option<u32>,
    pub seq: u64,
}

impl Volatile {
    pub fn next_seq(&mut self) -> u64 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    pub fn prune_chain(&mut self, now_ms: u64) {
        self.chain
            .retain(|e| now_ms.saturating_sub(e.at_ms) <= CHAIN_WINDOW_MS);
    }

    pub fn chain_depth_for(&self, keys: &[ChainKey], now_ms: u64) -> u32 {
        let parent = self
            .chain
            .iter()
            .filter(|e| now_ms.saturating_sub(e.at_ms) <= CHAIN_WINDOW_MS)
            .filter(|e| keys.contains(&e.key))
            .map(|e| e.depth)
            .max();
        parent.map_or(0, |d| d.saturating_add(1))
    }

    pub fn record_chain(&mut self, key: ChainKey, depth: u32, now_ms: u64) {
        self.chain.push(ChainEntry {
            key,
            depth,
            at_ms: now_ms,
        });
    }

    pub fn timer(&self, name: &str) -> Option<&NamedTimer> {
        self.timers.iter().find(|t| t.name == name)
    }

    pub fn cancel_timer(&mut self, name: &str) {
        self.timers.retain(|t| t.name != name);
    }

    pub fn retain_rule_names(&mut self, survives: &dyn Fn(&str) -> bool) {
        self.last_activation_ms.retain(|name, _| survives(name));
        self.cycle_pos.retain(|name, _| survives(name));
    }
}

pub(crate) fn lamp_chain_keys(adapter_id: u8, lamp_id: u16) -> [ChainKey; 2] {
    [
        ChainKey::Lamp(adapter_id, lamp_id),
        ChainKey::AdapterWide(adapter_id),
    ]
}

pub(crate) fn group_chain_keys(adapter_id: u8, group_id: u16) -> [ChainKey; 2] {
    [
        ChainKey::Group(adapter_id, group_id),
        ChainKey::AdapterWide(adapter_id),
    ]
}

pub(crate) fn target_chain_keys(target: &LightTarget) -> [Option<ChainKey>; 2] {
    match target {
        LightTarget::Lamp(l) => [Some(ChainKey::Lamp(l.adapter_id, l.id)), None],
        LightTarget::Group(g) => [
            Some(ChainKey::Group(g.adapter_id, g.id)),
            Some(ChainKey::AdapterWide(g.adapter_id)),
        ],
        LightTarget::Broadcast { adapter_id } => {
            [Some(ChainKey::AdapterWide(*adapter_id)), None]
        }
    }
}
