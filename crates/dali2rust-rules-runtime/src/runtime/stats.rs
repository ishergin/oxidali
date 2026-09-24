use std::sync::atomic::{AtomicU32, Ordering};

use crate::runtime::engine::EngineCountersSnapshot;

#[derive(Debug, Default)]
pub struct RulesEngineCells {
    pub activations_total: AtomicU32,
    pub activations_dry: AtomicU32,
    pub suppressed_cooldown: AtomicU32,
    pub suppressed_disabled: AtomicU32,
    pub conditions_rejected: AtomicU32,
    pub partial_outcomes: AtomicU32,
    pub chain_depth_exceeded: AtomicU32,
    pub effects_emitted: AtomicU32,
    pub actions_failed: AtomicU32,
    pub continuations_scheduled: AtomicU32,
    pub continuations_fired: AtomicU32,
    pub continuations_dropped: AtomicU32,
    pub timers_active: AtomicU32,
    pub ticks_time_unsynced: AtomicU32,
    pub rules_loaded: AtomicU32,
    pub vars_in_use: AtomicU32,
    pub latency_p50_ms: AtomicU32,
    pub latency_p95_ms: AtomicU32,
    pub latency_max_ms: AtomicU32,
}

impl RulesEngineCells {
    pub(crate) fn store(&self, snap: &EngineCountersSnapshot) {
        self.activations_total.store(snap.activations_total, Ordering::Relaxed);
        self.activations_dry.store(snap.activations_dry, Ordering::Relaxed);
        self.suppressed_cooldown.store(snap.suppressed_cooldown, Ordering::Relaxed);
        self.suppressed_disabled.store(snap.suppressed_disabled, Ordering::Relaxed);
        self.conditions_rejected.store(snap.conditions_rejected, Ordering::Relaxed);
        self.partial_outcomes.store(snap.partial_outcomes, Ordering::Relaxed);
        self.chain_depth_exceeded.store(snap.chain_depth_exceeded, Ordering::Relaxed);
        self.effects_emitted.store(snap.effects_emitted, Ordering::Relaxed);
        self.actions_failed.store(snap.actions_failed, Ordering::Relaxed);
        self.continuations_scheduled.store(snap.continuations_scheduled, Ordering::Relaxed);
        self.continuations_fired.store(snap.continuations_fired, Ordering::Relaxed);
        self.continuations_dropped.store(snap.continuations_dropped, Ordering::Relaxed);
        self.timers_active.store(snap.timers_active, Ordering::Relaxed);
        self.ticks_time_unsynced.store(snap.ticks_time_unsynced, Ordering::Relaxed);
        self.rules_loaded.store(snap.rules_loaded, Ordering::Relaxed);
        self.vars_in_use.store(snap.vars_in_use, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
pub(crate) struct LatencyWindow {
    slots: [u16; 32],
    filled: usize,
    next: usize,
    max: u16,
}

impl LatencyWindow {
    pub(crate) fn note(&mut self, ms: u16, cells: &RulesEngineCells) {
        self.slots[self.next] = ms;
        self.next = (self.next + 1) % self.slots.len();
        self.filled = (self.filled + 1).min(self.slots.len());
        self.max = self.max.max(ms);
        let mut sorted = self.slots[..self.filled].to_vec();
        sorted.sort_unstable();
        let p50 = sorted[self.filled.saturating_sub(1) / 2];
        let p95 = sorted[(self.filled.saturating_sub(1)) * 95 / 100];
        cells.latency_p50_ms.store(u32::from(p50), Ordering::Relaxed);
        cells.latency_p95_ms.store(u32::from(p95), Ordering::Relaxed);
        cells.latency_max_ms.store(u32::from(self.max), Ordering::Relaxed);
    }
}
