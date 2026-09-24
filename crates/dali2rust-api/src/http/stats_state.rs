use serde::Serialize;

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
    pub foreign_frames_total: u32,
    pub foreign_verbs_projected_total: u32,
    pub foreign_dimming_unprojected_total: u32,
    pub backward_undecodable_total: u32,
    pub backward_frame_size_total: u32,
    pub backward_incomplete_total: u32,
    pub backward_early_rejected_total: u32,
    pub backward_late_rejected_total: u32,
    pub backward_multi_answer_total: u32,
    pub console_log_dropped_total: u32,
    pub console_log_busy_total: u32,
    pub console_log_truncated_total: u32,
    pub console_log_unavailable_total: u32,
    pub console_uart_errors_total: u32,
    pub isr_ticks_deficit_raw_total: u32,
    pub isr_ticks_surplus_raw_total: u32,
    pub isr_ticks_lost_total: u32,
    pub isr_ticks_extra_total: u32,
    pub isr_late_ticks_total: u32,
    pub isr_max_gap_us: u32,
    #[serde(flatten)]
    pub task_timing: StatsDaliTaskTimingDto,
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

#[derive(Clone, Copy, Debug, Default, Serialize)]
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
    pub continuations_scheduled: u32,
    pub continuations_fired: u32,
    pub continuations_dropped: u32,
    pub timers_active: u32,
    pub ticks_time_unsynced: u32,
    pub rules_loaded: u32,
    pub vars_in_use: u32,
    pub latency_p50_ms: u32,
    pub latency_p95_ms: u32,
    pub latency_max_ms: u32,
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
pub struct StatsMqttDto {
    pub connected: bool,
    pub publishes_total: u32,
    pub publish_failures_total: u32,
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

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StatsReportDto {
    pub sample_ms: u64,
    pub controller: StatsControllerDto,
    pub bus: StatsBusDto,
    pub dali: StatsDaliDto,
    pub operations: StatsOperationsDto,
    pub websocket: StatsWebSocketDto,
    pub mqtt: StatsMqttDto,
    pub input: StatsInputDto,
    pub rules: StatsRulesDto,
    pub network: Option<StatsNetworkDto>,
}

pub trait StatsHttpState: Send + Sync {
    fn stats_dto(&self) -> StatsReportDto;

    fn stats_dto_into(&self, out: &mut StatsReportDto) {
        *out = self.stats_dto();
    }
}
