#[cfg(test)]
use dali2rust_rules_model::limits::{MAX_NAME_BYTES, MAX_STAT_NAMES};
use serde::Serialize;

#[cfg(test)]
const WIDEST_STAT_NAME_CHAR: &str = "\\";

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsControllerDto {
    pub uptime_ms: u64,
    pub free_heap_bytes: Option<u32>,
    pub internal_free_bytes: Option<u32>,
    pub internal_largest_block_bytes: Option<u32>,
    pub internal_min_free_bytes: Option<u32>,
    pub internal_total_bytes: Option<u32>,
    pub internal_allocated_blocks: Option<u32>,
    pub rust_internal_live_bytes: Option<u32>,
    pub rust_internal_peak_bytes: Option<u32>,
    pub rust_psram_live_bytes: Option<u32>,
    pub rust_psram_peak_bytes: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsBusDto {
    pub commands_published_total: u32,
    pub events_published_total: u32,
    pub commands_ingress_overflow_total: u32,
    pub confirmation_timeouts_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsDaliDto {
    pub commands_executed_total: u32,
    pub errors_total: u32,
    pub target_state_superseded_total: u32,
    pub wire_load_permille: u32,
    pub wire_load_own_permille: u32,
    pub collision_restarts_total: u32,
    pub foreign_frames_total: u32,
    pub foreign_verbs_projected_total: u32,
    pub foreign_dimming_unprojected_total: u32,
    pub foreign_scene_writes_total: u32,
    pub foreign_unaddressed_ignored_total: u32,
    #[serde(flatten)]
    pub backward: StatsDaliBackwardDto,
    #[serde(flatten)]
    pub console: StatsDaliConsoleDto,
    pub isr_ticks_deficit_raw_total: u32,
    pub isr_ticks_surplus_raw_total: u32,
    pub isr_ticks_lost_total: u32,
    pub isr_ticks_extra_total: u32,
    pub isr_late_ticks_total: u32,
    pub isr_max_gap_us: u32,
    #[serde(flatten)]
    pub task_timing: StatsDaliTaskTimingDto,
    #[serde(flatten)]
    pub readback: StatsDaliReadbackDto,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsDaliReadbackDto {
    pub readback_groups_corrected_total: u32,
    pub readback_colour_features_corrected_total: u32,
    pub readback_extended_fade_corrected_total: u32,
    pub program_repairs_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsDaliBackwardDto {
    pub backward_undecodable_total: u32,
    pub backward_frame_size_total: u32,
    pub backward_incomplete_total: u32,
    pub backward_early_rejected_total: u32,
    pub backward_late_rejected_total: u32,
    pub backward_multi_answer_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsDaliConsoleDto {
    pub console_log_dropped_total: u32,
    pub console_log_busy_total: u32,
    pub console_log_truncated_total: u32,
    pub console_log_unavailable_total: u32,
    pub console_uart_errors_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsDaliTaskTimingDto {
    pub answer_staged_total: u32,
    pub answer_stage_late_total: u32,
    pub answer_stage_max_ticks: u32,
    pub sniff_poll_late_total: u32,
    pub sniff_poll_gap_max_us: u32,
    pub persist_flush_total: u32,
    pub persist_flush_slow_total: u32,
    pub persist_flush_ms_total: u32,
    pub persist_flush_max_ms: u32,
    pub persist_gate_waits_total: u32,
    pub persist_gate_timeouts_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsOperationsDto {
    pub running: u32,
    pub succeeded_total: u32,
    pub failed_total: u32,
    pub timed_out_total: u32,
    pub cancelled_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsWebSocketDto {
    pub clients: u32,
    pub events_sent_total: u32,
    pub events_dropped_total: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StatsRuleCountDto {
    pub name: String,
    pub count: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StatsRulesDto {
    pub activations_total: u32,
    pub activations_dry: u32,
    pub suppressed_cooldown: u32,
    pub suppressed_disabled: u32,
    pub conditions_rejected: u32,
    pub partial_outcomes: u32,
    pub chain_depth_exceeded: u32,
    pub effects_emitted: u32,
    pub actions_failed: u32,
    pub effects_unbound: u32,
    pub hcl_switches_coalesced: u32,
    pub continuations_scheduled: u32,
    pub continuations_fired: u32,
    pub continuations_dropped: u32,
    pub continuations_pending: u32,
    pub timers_active: u32,
    pub ticks_time_unsynced: u32,
    pub rules_loaded: u32,
    pub vars_in_use: u32,
    pub latency_p50_ms: u32,
    pub latency_p95_ms: u32,
    pub latency_max_ms: u32,
    pub stats: Vec<StatsRuleCountDto>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsInputDto {
    pub frames24_rx: u32,
    pub events_typed: u32,
    pub events_generic: u32,
    pub events_typed_from_registry: u32,
    pub events_ambiguous_scheme: u32,
    pub events_unattributed: u32,
    pub lifecycle_events: u32,
    pub scans_completed: u32,
    pub devices_addressed: u32,
    pub config_rejected: u32,
    pub addresses_contended: u32,
    pub presence_reprobed: u32,
    pub readbacks_applied: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsHclDto {
    pub ticks_cut_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsMqttDto {
    pub connected: bool,
    pub publishes_total: u32,
    pub publish_failures_total: u32,
    pub rule_messages_total: u32,
    pub rule_messages_coalesced_total: u32,
    pub rule_messages_lost_total: u32,
    pub subscriptions_refused_total: u32,
    pub own_topics_refused_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsNetworkDto {
    pub rx_packets_total: u32,
    pub tx_packets_total: u32,
    pub rx_dropped_total: u32,
    pub tx_dropped_total: u32,
    pub rx_ring_overruns_total: u32,
    pub rx_fifo_overflows_total: u32,
    pub link_up_events_total: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StatsReportDto {
    pub sample_ms: u64,
    pub controller: StatsControllerDto,
    pub bus: StatsBusDto,
    pub dali: StatsDaliDto,
    pub operations: StatsOperationsDto,
    pub websocket: StatsWebSocketDto,
    pub mqtt: StatsMqttDto,
    pub input: StatsInputDto,
    pub hcl: StatsHclDto,
    pub rules: StatsRulesDto,
    pub network: Option<StatsNetworkDto>,
}

#[cfg(test)]
fn widest_rule_stats() -> Vec<StatsRuleCountDto> {
    let name = WIDEST_STAT_NAME_CHAR.repeat(MAX_NAME_BYTES);
    vec![StatsRuleCountDto { name, count: u32::MAX }; MAX_STAT_NAMES]
}

declare_widest_dtos! {
    StatsControllerDto {
        uptime_ms = u64::MAX, free_heap_bytes = Some(u32::MAX),
        internal_free_bytes = Some(u32::MAX), internal_largest_block_bytes = Some(u32::MAX),
        internal_min_free_bytes = Some(u32::MAX), internal_total_bytes = Some(u32::MAX),
        internal_allocated_blocks = Some(u32::MAX), rust_internal_live_bytes = Some(u32::MAX),
        rust_internal_peak_bytes = Some(u32::MAX), rust_psram_live_bytes = Some(u32::MAX),
        rust_psram_peak_bytes = Some(u32::MAX),
    }
    StatsBusDto {
        commands_published_total, events_published_total, commands_ingress_overflow_total,
        confirmation_timeouts_total,
    }
    StatsDaliDto {
        commands_executed_total, errors_total, target_state_superseded_total,
        wire_load_permille, wire_load_own_permille, collision_restarts_total,
        foreign_frames_total, foreign_verbs_projected_total, foreign_dimming_unprojected_total,
        foreign_scene_writes_total, foreign_unaddressed_ignored_total,
        backward = StatsDaliBackwardDto::widest(), console = StatsDaliConsoleDto::widest(),
        isr_ticks_deficit_raw_total, isr_ticks_surplus_raw_total, isr_ticks_lost_total,
        isr_ticks_extra_total, isr_late_ticks_total, isr_max_gap_us,
        task_timing = StatsDaliTaskTimingDto::widest(), readback = StatsDaliReadbackDto::widest(),
    }
    StatsDaliReadbackDto {
        readback_groups_corrected_total, readback_colour_features_corrected_total,
        readback_extended_fade_corrected_total, program_repairs_total,
    }
    StatsDaliBackwardDto {
        backward_undecodable_total, backward_frame_size_total, backward_incomplete_total,
        backward_early_rejected_total, backward_late_rejected_total, backward_multi_answer_total,
    }
    StatsDaliConsoleDto {
        console_log_dropped_total, console_log_busy_total, console_log_truncated_total,
        console_log_unavailable_total, console_uart_errors_total,
    }
    StatsDaliTaskTimingDto {
        answer_staged_total, answer_stage_late_total, answer_stage_max_ticks,
        sniff_poll_late_total, sniff_poll_gap_max_us, persist_flush_total,
        persist_flush_slow_total, persist_flush_ms_total, persist_flush_max_ms,
        persist_gate_waits_total, persist_gate_timeouts_total,
    }
    StatsOperationsDto {
        running, succeeded_total, failed_total, timed_out_total, cancelled_total,
    }
    StatsWebSocketDto {
        clients, events_sent_total, events_dropped_total,
    }
    StatsRulesDto {
        activations_total, activations_dry, suppressed_cooldown, suppressed_disabled,
        conditions_rejected, partial_outcomes, chain_depth_exceeded, effects_emitted,
        actions_failed, effects_unbound, hcl_switches_coalesced, continuations_scheduled,
        continuations_fired, continuations_dropped, continuations_pending, timers_active,
        ticks_time_unsynced, rules_loaded, vars_in_use, latency_p50_ms, latency_p95_ms,
        latency_max_ms, stats = widest_rule_stats(),
    }
    StatsInputDto {
        frames24_rx, events_typed, events_generic, events_typed_from_registry,
        events_ambiguous_scheme, events_unattributed, lifecycle_events, scans_completed,
        devices_addressed, config_rejected, addresses_contended, presence_reprobed,
        readbacks_applied,
    }
    StatsMqttDto {
        connected = false, publishes_total, publish_failures_total, rule_messages_total,
        rule_messages_coalesced_total, rule_messages_lost_total, subscriptions_refused_total,
        own_topics_refused_total,
    }
    StatsHclDto {
        ticks_cut_total,
    }
    StatsNetworkDto {
        rx_packets_total, tx_packets_total, rx_dropped_total, tx_dropped_total,
        rx_ring_overruns_total, rx_fifo_overflows_total, link_up_events_total,
    }
    StatsReportDto {
        sample_ms = u64::MAX, controller = StatsControllerDto::widest(),
        bus = StatsBusDto::widest(), dali = StatsDaliDto::widest(),
        operations = StatsOperationsDto::widest(), websocket = StatsWebSocketDto::widest(),
        mqtt = StatsMqttDto::widest(), input = StatsInputDto::widest(),
        hcl = StatsHclDto::widest(), rules = StatsRulesDto::widest(),
        network = Some(StatsNetworkDto::widest()),
    }
}

pub trait StatsHttpState: Send + Sync {
    fn stats_dto(&self) -> StatsReportDto;

    fn stats_dto_into(&self, out: &mut StatsReportDto) {
        *out = self.stats_dto();
    }
}
