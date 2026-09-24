use serde::Serialize;

macro_rules! declare_widest_dtos {
    ( $( $ty:ident { $( $field:ident $( = $widest:expr )? ),* $(,)? } )* ) => {
        $(
            impl $ty {
                #[cfg(test)]
                pub(crate) fn widest() -> Self {
                    Self { $( $field: declare_widest_dtos!(@widest $($widest)?), )* }
                }
            }
        )*
    };
    (@widest $widest:expr) => { $widest };
    (@widest) => { u32::MAX };
}

#[cfg(test)]
const WORST_COMMAND_SUBSCRIBERS: usize = 9;
#[cfg(test)]
const WORST_CONFIRMATION_SUBSCRIBERS: usize = 6;
#[cfg(test)]
const WORST_EVENT_SUBSCRIBERS: usize = 12;

#[cfg(test)]
fn widest_subscribers(rows: usize) -> Vec<SubscriberCountersDto> {
    vec![SubscriberCountersDto::widest(); rows]
}

#[cfg(test)]
fn widest_event_subscribers() -> Vec<EventSubscriberCountersDto> {
    vec![
        EventSubscriberCountersDto {
            delivered: u32::MAX,
            receiver_overflow: u32::MAX,
            name: "wwwwwwwwwwwwwwwwwwwwwwww",
        };
        WORST_EVENT_SUBSCRIBERS
    ]
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ChannelCountersDto {
    pub publish_attempted: u32,
    pub publish_queued: u32,
    pub ingress_overflow: u32,
    pub oversize_rejected: u32,
    pub kind_mismatch: u32,
}

impl From<dali2rust_bus::ChannelCounters> for ChannelCountersDto {
    fn from(c: dali2rust_bus::ChannelCounters) -> Self {
        Self {
            publish_attempted: c.publish_attempted,
            publish_queued: c.publish_queued,
            ingress_overflow: c.ingress_overflow,
            oversize_rejected: c.oversize_rejected,
            kind_mismatch: c.kind_mismatch,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct SubscriberCountersDto {
    pub delivered: u32,
    pub receiver_overflow: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct EventSubscriberCountersDto {
    pub delivered: u32,
    pub receiver_overflow: u32,
    pub name: &'static str,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BusCountersDto {
    pub commands: ChannelCountersDto,
    pub confirmations: ChannelCountersDto,
    pub events: ChannelCountersDto,
    pub commands_unrouted: u32,
    pub delivery_rejected_dropped: u32,
    pub command_subscribers: Vec<SubscriberCountersDto>,
    pub confirmation_subscribers: Vec<SubscriberCountersDto>,
    pub event_subscribers: Vec<EventSubscriberCountersDto>,
}

fn subscribers(src: &[dali2rust_bus::SubscriberCounters]) -> Vec<SubscriberCountersDto> {
    src.iter()
        .map(|s| SubscriberCountersDto {
            delivered: s.delivered,
            receiver_overflow: s.receiver_overflow,
        })
        .collect()
}

fn event_subscribers(
    src: &[dali2rust_bus::SubscriberCounters],
) -> Vec<EventSubscriberCountersDto> {
    src.iter()
        .map(|s| EventSubscriberCountersDto {
            delivered: s.delivered,
            receiver_overflow: s.receiver_overflow,
            name: s.name,
        })
        .collect()
}

impl From<&dali2rust_bus::BusCounters> for BusCountersDto {
    fn from(c: &dali2rust_bus::BusCounters) -> Self {
        Self {
            commands: c.commands.into(),
            confirmations: c.confirmations.into(),
            events: c.events.into(),
            commands_unrouted: c.commands_unrouted,
            delivery_rejected_dropped: c.delivery_rejected_dropped,
            command_subscribers: subscribers(&c.command_subscribers),
            confirmation_subscribers: subscribers(&c.confirmation_subscribers),
            event_subscribers: event_subscribers(&c.event_subscribers),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ConfirmationBridgeDto {
    pub unmatched: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct DaliWorkerDto {
    pub invalid_command: u32,
    pub execution_failed: u32,
    pub confirmation_publish_failed: u32,
    pub event_publish_failed: u32,
    pub evidence_publish_failed: u32,
    pub event_publish_retried: u32,
    pub event_publish_backoff_ms: u32,
    pub target_state_superseded: u32,
    pub read_attributes_contended_aborts: u32,
    pub read_attributes_transport_aborts: u32,
    pub read_attributes_preempted: u32,
    pub write_attributes_preempted: u32,
    pub read_attributes_device_absent: u32,
    pub read_attributes_sequence_incomplete: u32,
    pub discovery_device_type_degraded: u32,
    pub bus_health_probe_failed: u32,
    pub memory_bank_short_reads: u32,
    pub input_scans_completed: u32,
    pub input_devices_addressed: u32,
    pub input_config_rejected: u32,
    pub tx_suppressed_passive: u32,
    pub ignored_commands: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct SnifferTranslatorDto {
    pub observed_published: u32,
    pub unknown_seen: u32,
    pub special_tracked: u32,
    pub dt8_staged: u32,
    pub backward_ignored: u32,
    pub publish_failed: u32,
    pub input_events_typed: u32,
    pub input_events_generic: u32,
    pub input_events_ambiguous_scheme: u32,
    pub input_lifecycle: u32,
    pub input_publish_retried: u32,
    pub app_control_pairs: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RulesWorkerDto {
    pub commits_applied: u32,
    pub commits_rejected: u32,
    pub enable_toggles: u32,
    pub hydrate_failed: u32,
    pub persist_failed: u32,
    pub ignored_commands: u32,
    pub effects_published: u32,
    pub effects_ingress_rejected: u32,
    pub effects_skipped_dark: u32,
    pub hcl_hold_unmapped: u32,
    pub hcl_schedule_unmapped: u32,
    pub input_action_unmapped: u32,
    pub log_lines: u32,
    pub stat_counts: u32,
    pub activations_published: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ProjectorDto {
    pub runtime_updates_published: u32,
    pub runtime_updates_retried: u32,
    pub group_expansions: u32,
    pub scene_expansions: u32,
    pub broadcast_expansions: u32,
    pub coalesced_observed: u32,
    pub skipped_unknown_observed: u32,
    pub skipped_unbound: u32,
    pub publish_failed: u32,
    pub ignored_events: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RegistryDto {
    pub runtime_updates_superseded: u32,
    pub config_write_signal_publish_failed: u32,
    pub ignored_events: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct HclSchedulerDto {
    pub ticks: u32,
    pub ticks_time_unsynced: u32,
    pub commands_published: u32,
    pub commands_dropped_cap: u32,
    pub deferred_dropped_cap: u32,
    pub command_timeouts: u32,
    pub command_failures: u32,
    pub ingress_rejections: u32,
    pub overrides_started: u32,
    pub overrides_cleared: u32,
    pub overrides_reset: u32,
    pub ignored_commands: u32,
    pub ignored_events: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PollerDto {
    pub cycles_total: u32,
    pub reads_published: u32,
    pub reads_completed: u32,
    pub reads_failed: u32,
    pub reads_preempted: u32,
    pub reads_absent: u32,
    pub outstanding_expired: u32,
    pub skipped_inbox_full: u32,
    pub duty_deferred: u32,
    pub interactive_deferred: u32,
    pub targets_excluded: u32,
    pub window_deferred: u32,
    pub device_cooldowns: u32,
    pub health_probes_published: u32,
    pub health_probes_invalid: u32,
    pub health_probes_clear: u32,
    pub health_probes_one_failure: u32,
    pub health_probes_several_failures: u32,
    pub health_probes_expired: u32,
    pub ignored_events: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PersistenceDto {
    pub flush_success_total: u32,
    pub flush_error_total: u32,
    pub no_space_total: u32,
    pub hydrate_loaded_total: u32,
    pub hydrate_default_total: u32,
    pub hydrate_error_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PhySnifferDto {
    pub frames: u32,
    pub backward8: u32,
    pub forward16: u32,
    pub forward24: u32,
    pub decode_failed: u32,
    pub unsupported_len: u32,
    pub dropped: u32,
    pub poll_fast: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct DaliWireDto {
    pub transactions_started: u32,
    pub transactions_completed: u32,
    pub transactions_by_class: [u32; 4],
    pub transaction_reopened: u32,
    pub transaction_should_exceedances: u32,
    pub transaction_budget_exceeded: u32,
    pub transaction_leaks: u32,
    pub session_spins: u32,
    pub frames_sent_by_priority: [u32; 5],
    pub p1_window_late: u32,
    pub bus_releases: u32,
    pub bus_acquire_timeout: u32,
    pub collisions: u32,
    pub foreign_in_window: u32,
    pub corrupted_in_window: u32,
    pub exchange_retries: u32,
    pub retry_exhausted: u32,
    pub send_twice_over_transmitter_max: u32,
    pub send_twice_split: u32,
    pub bus_power_down_active: u32,
    pub bus_power_down_entries: u32,
    pub system_failure_active: u32,
    pub system_failure_entries: u32,
    pub wire_ticks_total: u32,
    pub wire_ticks_active: u32,
    pub wire_ticks_tx: u32,
    pub load_permille: u32,
    pub load_own_permille: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct WebSocketDto {
    pub clients: u32,
    pub events_sent_total: u32,
    pub events_dropped_total: u32,
    pub events_coalesced_total: u32,
    pub inbox_overflow_total: u32,
    pub upgrades_rejected_total: u32,
    pub sniffer_records_total: u32,
    pub sniffer_dropped_total: u32,
    pub logs_lines_total: u32,
    pub logs_dropped_total: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MqttBridgeDto {
    pub connected: bool,
    pub connects_total: u32,
    pub publishes_total: u32,
    pub publish_failures_total: u32,
    pub commands_received_total: u32,
    pub commands_dropped_total: u32,
    pub commands_unroutable_total: u32,
    pub commands_ingress_rejected_total: u32,
    pub discovery_published_total: u32,
    pub discovery_failed_total: u32,
    pub terminal_event_publish_failed_total: u32,
    pub terminal_event_publish_retried_total: u32,
    pub bus_discarded_total: u32,
    pub bus_coalesced_total: u32,
    pub rule_publishes_dropped_total: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ApplyOrchestratorDto {
    pub runs_started: u32,
    pub cells_published: u32,
    pub skips_published: u32,
    pub cell_retries: u32,
    pub outcome_timeouts: u32,
    pub ingress_backoffs: u32,
    pub runs_aborted: u32,
    pub terminal_signal_publish_failed: u32,
    pub ignored_commands: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct OperationTrackerDto {
    pub pending_outcomes_expired: u32,
    pub ignored_commands: u32,
    pub ignored_events: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RedundancyDto {
    pub defended: u32,
    pub worker_stale: u32,
    pub answered: u32,
    pub suppressed: u32,
    pub cell_busy: u32,
    pub aborted: u32,
    pub window_closed: u32,
    pub late: u32,
    pub probe_failed: u32,
    pub handover_incomplete: u32,
    pub armed: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DiagnosticsDto {
    pub uptime_ms: u64,
    pub bus: BusCountersDto,
    pub confirmation_bridge: ConfirmationBridgeDto,
    pub dali_worker: DaliWorkerDto,
    pub sniffer_translator: SnifferTranslatorDto,
    pub projector: ProjectorDto,
    pub apply_orchestrator: ApplyOrchestratorDto,
    pub operations: OperationTrackerDto,
    pub registry: RegistryDto,
    pub persistence: PersistenceDto,
    pub phy_sniffer: PhySnifferDto,
    pub dali_wire: DaliWireDto,
    pub hcl: HclSchedulerDto,
    pub poller: PollerDto,
    pub websocket: WebSocketDto,
    pub mqtt: MqttBridgeDto,
    pub rules: RulesWorkerDto,
    pub redundancy: RedundancyDto,
}

pub trait DiagnosticsHttpState: Send + Sync {
    fn diagnostics_dto(&self) -> DiagnosticsDto;

    fn diagnostics_dto_into(&self, out: &mut DiagnosticsDto) {
        *out = self.diagnostics_dto();
    }
}

declare_widest_dtos! {
    ChannelCountersDto {
        publish_attempted, publish_queued, ingress_overflow, oversize_rejected,
        kind_mismatch,
    }
    SubscriberCountersDto {
        delivered, receiver_overflow,
    }
    BusCountersDto {
        commands = ChannelCountersDto::widest(),
        confirmations = ChannelCountersDto::widest(), events = ChannelCountersDto::widest(),
        commands_unrouted, delivery_rejected_dropped,
        command_subscribers = widest_subscribers(WORST_COMMAND_SUBSCRIBERS),
        confirmation_subscribers = widest_subscribers(WORST_CONFIRMATION_SUBSCRIBERS),
        event_subscribers = widest_event_subscribers(),
    }
    ConfirmationBridgeDto {
        unmatched,
    }
    DaliWorkerDto {
        invalid_command, execution_failed, confirmation_publish_failed,
        event_publish_failed, evidence_publish_failed, event_publish_retried,
        event_publish_backoff_ms, target_state_superseded, read_attributes_contended_aborts,
        read_attributes_transport_aborts, read_attributes_preempted,
        write_attributes_preempted, read_attributes_device_absent,
        read_attributes_sequence_incomplete, discovery_device_type_degraded,
        bus_health_probe_failed, memory_bank_short_reads,
        input_scans_completed, input_devices_addressed, input_config_rejected,
        tx_suppressed_passive, ignored_commands,
    }
    SnifferTranslatorDto {
        observed_published, unknown_seen, special_tracked, dt8_staged, backward_ignored,
        publish_failed, input_events_typed, input_events_generic,
        input_events_ambiguous_scheme, input_lifecycle, input_publish_retried,
        app_control_pairs,
    }
    RulesWorkerDto {
        commits_applied, commits_rejected, enable_toggles, hydrate_failed,
        persist_failed, ignored_commands, effects_published, effects_ingress_rejected,
        effects_skipped_dark, hcl_hold_unmapped, hcl_schedule_unmapped,
        input_action_unmapped, log_lines, stat_counts, activations_published,
    }
    ProjectorDto {
        runtime_updates_published, runtime_updates_retried,
        group_expansions, scene_expansions, broadcast_expansions,
        coalesced_observed, skipped_unknown_observed, skipped_unbound, publish_failed,
        ignored_events,
    }
    RegistryDto {
        runtime_updates_superseded, config_write_signal_publish_failed, ignored_events,
    }
    HclSchedulerDto {
        ticks, ticks_time_unsynced, commands_published, commands_dropped_cap,
        deferred_dropped_cap, command_timeouts, command_failures, ingress_rejections,
        overrides_started, overrides_cleared, overrides_reset, ignored_commands,
        ignored_events,
    }
    PollerDto {
        cycles_total, reads_published, reads_completed, reads_failed, reads_preempted,
        reads_absent, outstanding_expired, skipped_inbox_full, duty_deferred,
        interactive_deferred, targets_excluded, window_deferred, device_cooldowns,
        health_probes_published, health_probes_invalid, health_probes_clear,
        health_probes_one_failure, health_probes_several_failures, health_probes_expired,
        ignored_events,
    }
    PersistenceDto {
        flush_success_total, flush_error_total, no_space_total, hydrate_loaded_total,
        hydrate_default_total, hydrate_error_total,
    }
    PhySnifferDto {
        frames, backward8, forward16, forward24, decode_failed, unsupported_len, dropped,
        poll_fast,
    }
    DaliWireDto {
        transactions_started, transactions_completed, transactions_by_class = [u32::MAX; 4],
        transaction_reopened, transaction_should_exceedances, transaction_budget_exceeded,
        transaction_leaks, session_spins,
        frames_sent_by_priority = [u32::MAX; 5], p1_window_late,
        bus_releases, bus_acquire_timeout, collisions, foreign_in_window,
        corrupted_in_window, exchange_retries, retry_exhausted,
        send_twice_over_transmitter_max, send_twice_split, bus_power_down_active,
        bus_power_down_entries, system_failure_active, system_failure_entries,
        wire_ticks_total, wire_ticks_active, wire_ticks_tx, load_permille,
        load_own_permille,
    }
    WebSocketDto {
        clients, events_sent_total, events_dropped_total, events_coalesced_total,
        inbox_overflow_total, upgrades_rejected_total, sniffer_records_total,
        sniffer_dropped_total, logs_lines_total, logs_dropped_total,
    }
    MqttBridgeDto {
        connected = false, connects_total, publishes_total, publish_failures_total,
        commands_received_total, commands_dropped_total, commands_unroutable_total,
        commands_ingress_rejected_total, discovery_published_total, discovery_failed_total,
        terminal_event_publish_failed_total, terminal_event_publish_retried_total,
        bus_discarded_total, bus_coalesced_total, rule_publishes_dropped_total,
    }
    ApplyOrchestratorDto {
        runs_started, cells_published, skips_published, cell_retries, outcome_timeouts,
        ingress_backoffs, runs_aborted, terminal_signal_publish_failed, ignored_commands,
    }
    OperationTrackerDto {
        pending_outcomes_expired, ignored_commands, ignored_events,
    }
    RedundancyDto {
        defended, worker_stale, answered, suppressed, cell_busy, aborted, window_closed,
        late, probe_failed, handover_incomplete, armed = false,
    }
    DiagnosticsDto {
        uptime_ms = u64::MAX, bus = BusCountersDto::widest(),
        confirmation_bridge = ConfirmationBridgeDto::widest(),
        dali_worker = DaliWorkerDto::widest(),
        sniffer_translator = SnifferTranslatorDto::widest(),
        projector = ProjectorDto::widest(),
        apply_orchestrator = ApplyOrchestratorDto::widest(),
        operations = OperationTrackerDto::widest(), registry = RegistryDto::widest(),
        persistence = PersistenceDto::widest(), phy_sniffer = PhySnifferDto::widest(),
        dali_wire = DaliWireDto::widest(), hcl = HclSchedulerDto::widest(),
        poller = PollerDto::widest(), websocket = WebSocketDto::widest(),
        mqtt = MqttBridgeDto::widest(), rules = RulesWorkerDto::widest(),
        redundancy = RedundancyDto::widest(),
    }
}
