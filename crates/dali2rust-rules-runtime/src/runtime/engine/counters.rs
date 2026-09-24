#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineCountersSnapshot {
    pub activations_total: u32,
    pub activations_dry: u32,
    pub suppressed_cooldown: u32,
    pub suppressed_disabled: u32,
    pub conditions_rejected: u32,
    pub partial_outcomes: u32,
    pub chain_depth_exceeded: u32,
    pub effects_emitted: u32,
    pub actions_failed: u32,
    pub continuations_scheduled: u32,
    pub continuations_fired: u32,
    pub continuations_dropped: u32,
    pub ticks_time_unsynced: u32,
    pub timers_active: u32,
    pub rules_loaded: u32,
    pub vars_in_use: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EngineCounters {
    pub activations_total: u32,
    pub activations_dry: u32,
    pub suppressed_cooldown: u32,
    pub suppressed_disabled: u32,
    pub conditions_rejected: u32,
    pub partial_outcomes: u32,
    pub chain_depth_exceeded: u32,
    pub effects_emitted: u32,
    pub actions_failed: u32,
    pub continuations_scheduled: u32,
    pub continuations_fired: u32,
    pub continuations_dropped: u32,
    pub ticks_time_unsynced: u32,
}

impl EngineCounters {
    pub(crate) fn snapshot(
        &self,
        timers_active: u32,
        rules_loaded: u32,
        vars_in_use: u32,
    ) -> EngineCountersSnapshot {
        EngineCountersSnapshot {
            activations_total: self.activations_total,
            activations_dry: self.activations_dry,
            suppressed_cooldown: self.suppressed_cooldown,
            suppressed_disabled: self.suppressed_disabled,
            conditions_rejected: self.conditions_rejected,
            partial_outcomes: self.partial_outcomes,
            chain_depth_exceeded: self.chain_depth_exceeded,
            effects_emitted: self.effects_emitted,
            actions_failed: self.actions_failed,
            continuations_scheduled: self.continuations_scheduled,
            continuations_fired: self.continuations_fired,
            continuations_dropped: self.continuations_dropped,
            ticks_time_unsynced: self.ticks_time_unsynced,
            timers_active,
            rules_loaded,
            vars_in_use,
        }
    }
}

pub(crate) fn bump(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}
