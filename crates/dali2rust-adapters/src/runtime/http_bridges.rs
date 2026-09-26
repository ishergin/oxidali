use core::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use dali2rust_api::confirmation_bridge::PendingConfirmationSlots;
use dali2rust_api::http::dali_settings_state::{
    DaliSettingsApplyWatch, DaliSettingsApplyWatchBridge, DaliSettingsHttpState,
    DaliSettingsHttpStateBridge,
};
use dali2rust_api::http::diagnostics_state::{
    ApplyOrchestratorDto, BusCountersDto, ConfirmationBridgeDto, DaliWireDto, DaliWorkerDto,
    DiagnosticsDto, DiagnosticsHttpState, HclSchedulerDto, MqttBridgeDto, OperationTrackerDto,
    PersistenceDto, PhySnifferDto, PollerDto, ProjectorDto, RulesWorkerDto, SnifferTranslatorDto,
    WebSocketDto,
};
use dali2rust_api::http::home_assistant_settings_state::{
    HomeAssistantSettingsApplyWatch, HomeAssistantSettingsApplyWatchBridge,
    HomeAssistantSettingsHttpState, HomeAssistantSettingsHttpStateBridge,
};
use dali2rust_api::http::policies_state::{
    PoliciesApplyWatch, PoliciesApplyWatchBridge, PoliciesHttpState, PoliciesHttpStateBridge,
};
use dali2rust_api::http::poller_settings_state::{
    PollerSettingsApplyWatch, PollerSettingsApplyWatchBridge, PollerSettingsHttpState,
    PollerSettingsHttpStateBridge,
};
use dali2rust_api::http::redundancy_settings_state::{
    RedundancySettingsApplyWatch, RedundancySettingsApplyWatchBridge, RedundancySettingsHttpState,
    RedundancySettingsHttpStateBridge,
};
use dali2rust_api::http::stats_state::{
    StatsBusDto, StatsControllerDto, StatsDaliDto, StatsDaliTaskTimingDto, StatsHttpState, StatsInputDto, StatsMqttDto,
    StatsNetworkDto, StatsOperationsDto, StatsReportDto, StatsRulesDto, StatsWebSocketDto,
};
use dali2rust_api::http::{
    physical_device_state::PhysicalDeviceHttpState, AdapterHttpState, AdapterHttpStateBridge,
    AdapterSettingsApplyWatch, AdapterSettingsApplyWatchBridge, GroupHttpState,
    GroupHttpStateBridge, GroupMetadataApplyWatch, GroupMetadataApplyWatchBridge,
    HclScheduleHttpState, HclScheduleHttpStateBridge, PhysicalDeviceHttpStateBridge,
    PhysicalDevicePatchWatch, PhysicalDevicePatchWatchBridge, SceneHttpState, SceneHttpStateBridge,
    SceneMetadataApplyWatch, SceneMetadataApplyWatchBridge, VirtualLampBindingApplyWatch,
    VirtualLampBindingApplyWatchBridge, VirtualLampHttpState, VirtualLampHttpStateBridge,
    VirtualLampPatchWatch, VirtualLampPatchWatchBridge,
};
use dali2rust_bus::BusPublisher;
use dali2rust_domain::registry::RegistryReadPort;
use dali2rust_operations_runtime::{ApplyOrchestratorCounters, OperationTrackerCounters};
use dali2rust_platform::clock::Clock;
use dali2rust_platform::heap::{HeapStats, HeapStatsPort};
use dali2rust_platform::net::{LinkStats, NetworkLink};
use dali2rust_registry_runtime::{
    PersistenceCounters, RegistryApplyWatch, RegistryStore, RegistryWorkerCounters,
};
use dali2rust_ws_runtime::WsCounters;

use super::registry_init::RegistryHttpReadPorts;
use super::workers::RuntimeCounterHandles;

pub(crate) struct RulesHttpBridge {
    store: std::sync::Arc<dali2rust_rules_runtime::RulesStore>,
}

impl RulesHttpBridge {
    pub fn new(store: std::sync::Arc<dali2rust_rules_runtime::RulesStore>) -> Self {
        Self { store }
    }
}

impl dali2rust_api::http::rules_state::RulesHttpState for RulesHttpBridge {
    fn document(&self) -> dali2rust_api::http::rules_state::RulesDocumentView {
        let doc = self.store.document();
        dali2rust_api::http::rules_state::RulesDocumentView {
            source: doc.source,
            lang_id: doc.lang_id,
            revision: doc.revision,
            compiled: doc.compiled,
            diagnostic: doc.diagnostic,
        }
    }

    fn revision(&self) -> u32 {
        self.store.revision()
    }

    fn rule_runtime(&self) -> Vec<dali2rust_api::http::rules_state::RuleRuntimeView> {
        self.store
            .rule_runtime()
            .into_iter()
            .map(|row| dali2rust_api::http::rules_state::RuleRuntimeView {
                name: row.name,
                fire_count: row.fire_count,
                last_fired_at_ms: row.last_fired_at_ms,
                last_latency_ms: row.last_latency_ms,
                last_outcome: row.last_outcome.as_str(),
                last_error: row.last_error,
            })
            .collect()
    }
}

pub(crate) struct RegistryHttpPorts {
    pub read_port: Arc<dyn RegistryReadPort>,
    pub input_device_state: Arc<dyn dali2rust_api::http::input_device_state::InputDeviceHttpState>,
    pub adapter_state: Arc<dyn AdapterHttpState>,
    pub group_state: Arc<dyn GroupHttpState>,
    pub physical_state: Arc<dyn PhysicalDeviceHttpState>,
    pub vl_state: Arc<dyn VirtualLampHttpState>,
    pub scene_state: Arc<dyn SceneHttpState>,
    pub hcl_state: Arc<dyn HclScheduleHttpState>,
    pub poller_state: Arc<dyn PollerSettingsHttpState>,
    pub dali_settings_state: Arc<dyn DaliSettingsHttpState>,
    pub home_assistant_state: Arc<dyn HomeAssistantSettingsHttpState>,
    pub adapter_apply_watch: Arc<dyn AdapterSettingsApplyWatch>,
    pub group_metadata_watch: Arc<dyn GroupMetadataApplyWatch>,
    pub scene_metadata_watch: Arc<dyn SceneMetadataApplyWatch>,
    pub pd_apply_watch: Arc<dyn PhysicalDevicePatchWatch>,
    pub vl_patch_watch: Arc<dyn VirtualLampPatchWatch>,
    pub vl_binding_watch: Arc<dyn VirtualLampBindingApplyWatch>,
    pub poller_apply_watch: Arc<dyn PollerSettingsApplyWatch>,
    pub dali_settings_apply_watch: Arc<dyn DaliSettingsApplyWatch>,
    pub redundancy_settings_state: Arc<dyn RedundancySettingsHttpState>,
    pub role_port: Arc<dyn dali2rust_api::http::role::ControllerRolePort>,
    pub redundancy_settings_apply_watch: Arc<dyn RedundancySettingsApplyWatch>,
    pub policies_state: Arc<dyn PoliciesHttpState>,
    pub policies_apply_watch: Arc<dyn PoliciesApplyWatch>,
    pub config_transfer: Arc<dyn dali2rust_api::http::handlers::config_transfer::ConfigTransferPort>,
    pub home_assistant_apply_watch: Arc<dyn HomeAssistantSettingsApplyWatch>,
    pub home_assistant_settings_port:
        Arc<dyn dali2rust_domain::registry::HomeAssistantSettingsReadPort>,
}

fn transfer_port(
    store: &Arc<RegistryStore>,
    transfer: TransferSeams,
) -> Arc<dyn dali2rust_api::http::handlers::config_transfer::ConfigTransferPort> {
    Arc::new(ConfigTransferBridge::new(
        Arc::clone(store),
        transfer.slices,
        transfer.adapter_count,
        transfer.wall_clock,
    ))
}

#[derive(Clone)]
pub(crate) struct TransferSeams {
    pub slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
    pub adapter_count: u8,
    pub wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
}

fn apply_imported_timezone(
    key: dali2rust_platform::slice_store::SliceKey,
    clock: &dyn dali2rust_platform::wall_clock::WallClock,
    slices: Option<&Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
) {
    if key == dali2rust_platform::slice_store::SliceKey::ControllerSettings {
        crate::runtime::time_settings::hydrate_timezone(clock, slices);
    }
}

fn apply_watch(counters: &Arc<RegistryWorkerCounters>) -> Arc<RegistryApplyWatch> {
    Arc::new(RegistryApplyWatch::new(Arc::clone(counters)))
}

pub(crate) fn wire_registry_http_ports(
    worker_read_port: Arc<dyn RegistryReadPort>,
    store: Arc<RegistryStore>,
    http_read: RegistryHttpReadPorts,
    registry_counters: Arc<RegistryWorkerCounters>,
    transfer: TransferSeams,
) -> RegistryHttpPorts {
    let c = &registry_counters;
    let ha_settings_port = Arc::clone(&http_read.home_assistant);
    let config_transfer = transfer_port(&store, transfer);
    RegistryHttpPorts {
        read_port: worker_read_port,
        input_device_state: Arc::new(InputDeviceHttpStateBridge::new(store)),
        adapter_state: Arc::new(AdapterHttpStateBridge::new(http_read.adapter)),
        group_state: Arc::new(GroupHttpStateBridge::new(http_read.group)),
        physical_state: Arc::new(PhysicalDeviceHttpStateBridge::new(http_read.physical)),
        vl_state: Arc::new(VirtualLampHttpStateBridge::new(http_read.vl)),
        scene_state: Arc::new(SceneHttpStateBridge::new(http_read.scene)),
        hcl_state: Arc::new(HclScheduleHttpStateBridge::new(http_read.hcl)),
        poller_state: Arc::new(PollerSettingsHttpStateBridge::new(http_read.poller)),
        dali_settings_state: Arc::new(DaliSettingsHttpStateBridge::new(http_read.dali_settings)),
        home_assistant_state: Arc::new(HomeAssistantSettingsHttpStateBridge::new(
            http_read.home_assistant,
        )),
        adapter_apply_watch: Arc::new(AdapterSettingsApplyWatchBridge::new(apply_watch(c))),
        group_metadata_watch: Arc::new(GroupMetadataApplyWatchBridge::new(apply_watch(c))),
        scene_metadata_watch: Arc::new(SceneMetadataApplyWatchBridge::new(apply_watch(c))),
        pd_apply_watch: Arc::new(PhysicalDevicePatchWatchBridge::new(apply_watch(c))),
        vl_patch_watch: Arc::new(VirtualLampPatchWatchBridge::new(apply_watch(c))),
        vl_binding_watch: Arc::new(VirtualLampBindingApplyWatchBridge::new(apply_watch(c))),
        poller_apply_watch: Arc::new(PollerSettingsApplyWatchBridge::new(apply_watch(c))),
        dali_settings_apply_watch: Arc::new(DaliSettingsApplyWatchBridge::new(apply_watch(c))),
        redundancy_settings_state: Arc::new(RedundancySettingsHttpStateBridge::new(
            http_read.redundancy,
        )),
        role_port: Arc::new(RoleBridge::new(http_read.dali_settings_role)),
        redundancy_settings_apply_watch: Arc::new(RedundancySettingsApplyWatchBridge::new(
            apply_watch(c),
        )),
        policies_state: Arc::new(PoliciesHttpStateBridge::new(http_read.policies)),
        policies_apply_watch: Arc::new(PoliciesApplyWatchBridge::new(apply_watch(c))),
        config_transfer,
        home_assistant_apply_watch: Arc::new(HomeAssistantSettingsApplyWatchBridge::new(
            apply_watch(c),
        )),
        home_assistant_settings_port: ha_settings_port,
    }
}

pub(crate) struct DiagnosticsBridge {
    started: std::time::Instant,
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    counters: RuntimeCounterHandles,
    store: Arc<RegistryStore>,
}

impl DiagnosticsBridge {
    pub(crate) fn new(
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        counters: RuntimeCounterHandles,
        store: Arc<RegistryStore>,
    ) -> Self {
        Self {
            started: std::time::Instant::now(),
            publisher,
            slots,
            counters,
            store,
        }
    }
}

macro_rules! declare_counter_mapping {
    (
        $(
            $(#[$meta:meta])*
            $name:ident ( $c:ident : $src:ty ) -> $dto:ident {
                $( $field:ident $( = $expr:expr )? ),* $(,)?
            }
        )*
    ) => {
        $(
            $(#[$meta])*
            fn $name($c: &$src) -> $dto {
                $dto { $( $field: declare_counter_mapping!(@v $c $field $($expr)?), )* }
            }
        )*
    };
    (@v $c:ident $field:ident $expr:expr) => { $expr };
    (@v $c:ident $field:ident) => { $c.$field.load(Relaxed) };
}

declare_counter_mapping! {
    dali_worker_dto(c: dali2rust_dali_runtime::DaliWorkerCounters) -> DaliWorkerDto {
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

    sniffer_translator_dto(c: dali2rust_fanout_runtime::SnifferTranslatorCounters) -> SnifferTranslatorDto {
        observed_published, unknown_seen, special_tracked, dt8_staged, backward_ignored,
        publish_failed, input_events_typed, input_events_generic,
        input_events_ambiguous_scheme, input_lifecycle, input_publish_retried,
        app_control_pairs,
    }

    rules_worker_dto(c: dali2rust_rules_runtime::RulesWorkerCounters) -> RulesWorkerDto {
        commits_applied, commits_rejected, enable_toggles, hydrate_failed,
        persist_failed, ignored_commands, effects_published, effects_ingress_rejected,
        effects_skipped_dark, hcl_hold_unmapped, hcl_schedule_unmapped,
        input_action_unmapped, log_lines, stat_counts, activations_published,
    }

    projector_dto(c: dali2rust_fanout_runtime::ProjectorCounters) -> ProjectorDto {
        runtime_updates_published, runtime_updates_retried,
        group_expansions, scene_expansions, broadcast_expansions,
        coalesced_observed, skipped_unknown_observed, skipped_unbound, publish_failed,
        ignored_events,
    }

    apply_orchestrator_dto(c: ApplyOrchestratorCounters) -> ApplyOrchestratorDto {
        runs_started, cells_published, skips_published, cell_retries, outcome_timeouts,
        ingress_backoffs, runs_aborted, terminal_signal_publish_failed, ignored_commands,
    }

    operations_dto(c: OperationTrackerCounters) -> OperationTrackerDto {
        pending_outcomes_expired, ignored_commands, ignored_events,
    }

    persistence_dto(c: PersistenceCounters) -> PersistenceDto {
        flush_success_total, flush_error_total, no_space_total, hydrate_loaded_total,
        hydrate_default_total, hydrate_error_total,
    }

    hcl_scheduler_dto(c: dali2rust_hcl_runtime::HclSchedulerCounters) -> HclSchedulerDto {
        ticks, ticks_time_unsynced, commands_published, commands_dropped_cap,
        deferred_dropped_cap, command_timeouts, command_failures, ingress_rejections,
        overrides_started, overrides_cleared, overrides_reset, ignored_commands,
        ignored_events,
    }

    poller_dto(c: dali2rust_poller_runtime::PollerCounters) -> PollerDto {
        cycles_total, reads_published, reads_completed, reads_failed, reads_preempted,
        reads_absent, outstanding_expired, skipped_inbox_full, duty_deferred,
        interactive_deferred, targets_excluded, window_deferred, device_cooldowns,
        health_probes_published, health_probes_invalid, health_probes_clear,
        health_probes_one_failure, health_probes_several_failures, health_probes_expired,
        ignored_events,
    }

    phy_sniffer_dto(c: dali2rust_platform::dali::PhySnifferCounters) -> PhySnifferDto {
        frames, backward8, forward16, forward24, decode_failed, unsupported_len, dropped,
        poll_fast = c.poll_fast.load(Relaxed),
    }

    dali_wire_dto(c: dali2rust_platform::dali::DaliWireCounters) -> DaliWireDto {
        transactions_started, transactions_completed,
        transactions_by_class = load_array(&c.transactions_by_class), transaction_reopened,
        transaction_should_exceedances, transaction_budget_exceeded, transaction_leaks,
        session_spins = c.session_poll_spins.load(Relaxed),
        frames_sent_by_priority = load_array(&c.frames_sent_by_priority), p1_window_late,
        bus_releases, bus_acquire_timeout, collisions, foreign_in_window,
        corrupted_in_window, exchange_retries, retry_exhausted,
        send_twice_over_transmitter_max, send_twice_split, bus_power_down_active,
        bus_power_down_entries, system_failure_active, system_failure_entries,
        wire_ticks_total, wire_ticks_active, wire_ticks_tx, load_permille,
        load_own_permille,
    }

    websocket_dto(c: WsCounters) -> WebSocketDto {
        clients, events_sent_total, events_dropped_total, events_coalesced_total,
        inbox_overflow_total, upgrades_rejected_total, sniffer_records_total,
        sniffer_dropped_total, logs_lines_total, logs_dropped_total,
    }

    mqtt_dto(c: dali2rust_mqtt_runtime::MqttCounters) -> MqttBridgeDto {
        connected = c.is_connected(), connects_total, publishes_total,
        publish_failures_total, commands_received_total, commands_dropped_total,
        commands_unroutable_total, commands_ingress_rejected_total,
        discovery_published_total, discovery_failed_total,
        terminal_event_publish_failed_total, terminal_event_publish_retried_total,
        bus_discarded_total, bus_coalesced_total, rule_publishes_dropped_total,
    }
}

fn load_array<const N: usize>(counters: &[core::sync::atomic::AtomicU32; N]) -> [u32; N] {
    core::array::from_fn(|index| counters[index].load(Relaxed))
}

pub(crate) struct ControllerHaBridge {
    settings: Arc<dyn dali2rust_domain::registry::HomeAssistantSettingsReadPort>,
    counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
}

impl ControllerHaBridge {
    pub(crate) fn new(
        settings: Arc<dyn dali2rust_domain::registry::HomeAssistantSettingsReadPort>,
        counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    ) -> Self {
        Self { settings, counters }
    }
}

impl dali2rust_api::http::handlers::controller::ControllerHaSummary for ControllerHaBridge {
    fn ha_enabled_and_url(&self) -> (bool, String) {
        let view = self.settings.home_assistant_settings_view();
        let url = view.broker_url_view();
        (view.enabled, url)
    }

    fn ha_connected(&self) -> bool {
        self.counters.is_connected()
    }
}

fn stats_mqtt_dto(c: &dali2rust_mqtt_runtime::MqttCounters) -> StatsMqttDto {
    use dali2rust_mqtt_runtime::MqttCounters as M;
    StatsMqttDto {
        connected: c.is_connected(),
        publishes_total: M::load(&c.publishes_total),
        publish_failures_total: M::load(&c.publish_failures_total),
    }
}

fn stats_rules_dto(counters: &RuntimeCounterHandles) -> StatsRulesDto {
    let c = &counters.rules_engine;
    StatsRulesDto {
        activations_total: c.activations_total.load(Relaxed),
        activations_dry: c.activations_dry.load(Relaxed),
        suppressed_cooldown: c.suppressed_cooldown.load(Relaxed),
        suppressed_disabled: c.suppressed_disabled.load(Relaxed),
        conditions_rejected: c.conditions_rejected.load(Relaxed),
        partial_outcomes: c.partial_outcomes.load(Relaxed),
        chain_depth_exceeded: c.chain_depth_exceeded.load(Relaxed),
        effects_emitted: c.effects_emitted.load(Relaxed),
        actions_failed: c.actions_failed.load(Relaxed),
        continuations_scheduled: c.continuations_scheduled.load(Relaxed),
        continuations_fired: c.continuations_fired.load(Relaxed),
        continuations_dropped: c.continuations_dropped.load(Relaxed),
        timers_active: c.timers_active.load(Relaxed),
        ticks_time_unsynced: c.ticks_time_unsynced.load(Relaxed),
        rules_loaded: c.rules_loaded.load(Relaxed),
        vars_in_use: c.vars_in_use.load(Relaxed),
        latency_p50_ms: c.latency_p50_ms.load(Relaxed),
        latency_p95_ms: c.latency_p95_ms.load(Relaxed),
        latency_max_ms: c.latency_max_ms.load(Relaxed),
    }
}

fn stats_input_dto(counters: &RuntimeCounterHandles) -> StatsInputDto {
    let load = |c: &std::sync::atomic::AtomicU32| c.load(std::sync::atomic::Ordering::Relaxed);
    let translator = &counters.sniffer_translator;
    let worker = &counters.dali_worker;
    let registry = &counters.registry;
    StatsInputDto {
        frames24_rx: load(&counters.phy_sniffer.forward24),
        events_typed: load(&translator.input_events_typed),
        events_generic: load(&translator.input_events_generic),
        events_typed_from_registry: load(&translator.input_events_typed_from_registry),
        events_ambiguous_scheme: load(&translator.input_events_ambiguous_scheme),
        events_unattributed: load(&registry.events.input_events_unattributed),
        lifecycle_events: load(&translator.input_lifecycle),
        scans_completed: load(&worker.input_scans_completed),
        devices_addressed: load(&worker.input_devices_addressed),
        config_rejected: load(&worker.input_config_rejected),
        addresses_contended: load(&worker.input_addresses_contended),
        presence_reprobed: load(&worker.input_presence_reprobed),
        readbacks_applied: load(&registry.events.input_instance_readbacks_applied),
    }
}

fn stats_websocket_dto(c: &WsCounters) -> StatsWebSocketDto {
    StatsWebSocketDto {
        clients: WsCounters::load(&c.clients),
        events_sent_total: WsCounters::load(&c.events_sent_total),
        events_dropped_total: WsCounters::load(&c.events_dropped_total),
    }
}

pub(crate) struct StatsBridge {
    clock: Arc<dyn Clock>,
    started_ms: u64,
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    counters: RuntimeCounterHandles,
    heap: Option<Arc<dyn HeapStatsPort>>,
    link: Option<Arc<dyn NetworkLink>>,
}

impl StatsBridge {
    pub(crate) fn new(
        clock: Arc<dyn Clock>,
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        counters: RuntimeCounterHandles,
        platform: PlatformGauges,
    ) -> Self {
        let started_ms = clock.monotonic_ms();
        Self {
            clock,
            started_ms,
            publisher,
            slots,
            counters,
            heap: platform.heap,
            link: platform.link,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct PlatformGauges {
    pub(crate) heap: Option<Arc<dyn HeapStatsPort>>,
    pub(crate) link: Option<Arc<dyn NetworkLink>>,
}

fn stats_network_dto(s: LinkStats) -> StatsNetworkDto {
    StatsNetworkDto {
        rx_packets_total: s.rx_packets,
        tx_packets_total: s.tx_packets,
        rx_dropped_total: s.rx_dropped,
        tx_dropped_total: s.tx_dropped,
        rx_ring_overruns_total: s.rx_ring_overruns,
        rx_fifo_overflows_total: s.rx_fifo_overflows,
        link_up_events_total: s.link_up_events,
    }
}

fn stats_controller_dto(uptime_ms: u64, heap: Option<HeapStats>) -> StatsControllerDto {
    StatsControllerDto {
        uptime_ms,
        free_heap_bytes: heap.map(|h| h.free_bytes),
        internal_free_bytes: heap.map(|h| h.internal_free_bytes),
        internal_largest_block_bytes: heap.map(|h| h.internal_largest_block_bytes),
        internal_min_free_bytes: heap.map(|h| h.internal_min_free_bytes),
        internal_total_bytes: heap.map(|h| h.internal_total_bytes),
        internal_allocated_blocks: heap.map(|h| h.internal_allocated_blocks),
        rust_internal_live_bytes: heap.map(|h| h.rust_internal_live_bytes),
        rust_internal_peak_bytes: heap.map(|h| h.rust_internal_peak_bytes),
        rust_psram_live_bytes: heap.map(|h| h.rust_psram_live_bytes),
        rust_psram_peak_bytes: heap.map(|h| h.rust_psram_peak_bytes),
    }
}

fn stats_bus_dto(c: &dali2rust_bus::BusCounters, confirmation_timeouts: u32) -> StatsBusDto {
    StatsBusDto {
        commands_published_total: c.commands.publish_attempted,
        events_published_total: c.events.publish_attempted,
        commands_ingress_overflow_total: c.commands.ingress_overflow,
        confirmation_timeouts_total: confirmation_timeouts,
    }
}

fn stats_dali_dto(h: &RuntimeCounterHandles) -> StatsDaliDto {
    let c = &h.dali_worker;
    let console = crate::log::console_counters();
    StatsDaliDto {
        commands_executed_total: c.commands_handled.load(Relaxed),
        errors_total: c
            .execution_failed
            .load(Relaxed)
            .wrapping_add(c.invalid_command.load(Relaxed)),
        target_state_superseded_total: c.target_state_superseded.load(Relaxed),
        wire_load_permille: h.dali_wire.load_permille.load(Relaxed),
        wire_load_own_permille: h.dali_wire.load_own_permille.load(Relaxed),
        collision_restarts_total: h.dali_wire.collision_restarts.load(Relaxed),
        foreign_frames_total: h
            .phy_sniffer
            .forward16
            .load(Relaxed)
            .wrapping_add(h.phy_sniffer.forward24.load(Relaxed)),
        foreign_verbs_projected_total: h.projector.transitions_expanded.load(Relaxed),
        foreign_dimming_unprojected_total: h.sniffer_translator.dimming_unprojectable.load(Relaxed),
        backward_undecodable_total: h.phy_sniffer.backward_undecodable.load(Relaxed),
        backward_frame_size_total: h.phy_sniffer.backward_frame_size.load(Relaxed),
        backward_incomplete_total: h.phy_sniffer.backward_incomplete.load(Relaxed),
        backward_early_rejected_total: h.phy_sniffer.backward_early_rejected.load(Relaxed),
        backward_late_rejected_total: h.phy_sniffer.backward_late_rejected.load(Relaxed),
        backward_multi_answer_total: h.phy_sniffer.backward_multi_answer.load(Relaxed),
        console_log_dropped_total: console.dropped.load(Relaxed),
        console_log_busy_total: console.busy.load(Relaxed),
        console_log_truncated_total: console.truncated.load(Relaxed),
        console_log_unavailable_total: console.unavailable.load(Relaxed),
        console_uart_errors_total: console.uart_errors.load(Relaxed),
        isr_ticks_deficit_raw_total: h.phy_sniffer.isr_ticks_deficit_raw.load(Relaxed),
        isr_ticks_surplus_raw_total: h.phy_sniffer.isr_ticks_surplus_raw.load(Relaxed),
        isr_ticks_lost_total: h.phy_sniffer.isr_ticks_lost.load(Relaxed),
        isr_ticks_extra_total: h.phy_sniffer.isr_ticks_extra.load(Relaxed),
        isr_late_ticks_total: h.phy_sniffer.isr_late_ticks.load(Relaxed),
        isr_max_gap_us: h.phy_sniffer.isr_max_gap_us.load(Relaxed),
        task_timing: stats_task_timing_dto(h),
    }
}

fn stats_task_timing_dto(h: &RuntimeCounterHandles) -> StatsDaliTaskTimingDto {
    let s = &h.phy_sniffer;
    let flush = dali2rust_platform::dali::persist_flush_stats();
    let gate = dali2rust_platform::dali::persist_gate_stats();
    StatsDaliTaskTimingDto {
        answer_staged_total: s.answer_staged.load(Relaxed),
        answer_stage_late_total: s.answer_stage_late.load(Relaxed),
        answer_stage_max_ticks: s.answer_stage_max_ticks.load(Relaxed),
        sniff_poll_late_total: s.sniff_poll_late.load(Relaxed),
        sniff_poll_gap_max_us: s.sniff_poll_gap_max_us.load(Relaxed),
        persist_flush_total: flush.0,
        persist_flush_slow_total: flush.1,
        persist_flush_ms_total: flush.2,
        persist_flush_max_ms: flush.3,
        persist_gate_waits_total: gate.0,
        persist_gate_timeouts_total: gate.2,
    }
}

fn stats_operations_dto(c: &OperationTrackerCounters) -> StatsOperationsDto {
    let succeeded = c.succeeded.load(Relaxed);
    let failed = c.failed.load(Relaxed);
    let timed_out = c.timed_out.load(Relaxed);
    let cancelled = c.cancelled.load(Relaxed);
    let terminal = succeeded
        .wrapping_add(failed)
        .wrapping_add(timed_out)
        .wrapping_add(cancelled);
    StatsOperationsDto {
        running: c.accepted.load(Relaxed).wrapping_sub(terminal),
        succeeded_total: succeeded,
        failed_total: failed,
        timed_out_total: timed_out,
        cancelled_total: cancelled,
    }
}

impl StatsHttpState for StatsBridge {
    fn stats_dto(&self) -> StatsReportDto {
        let mut dto = StatsReportDto::default();
        self.stats_dto_into(&mut dto);
        dto
    }

    fn stats_dto_into(&self, out: &mut StatsReportDto) {
        let uptime_ms = self.clock.monotonic_ms().saturating_sub(self.started_ms);
        out.sample_ms = uptime_ms;
        out.controller = stats_controller_dto(uptime_ms, self.heap.as_ref().map(|h| h.snapshot()));
        out.bus = stats_bus_dto(
            &self.publisher.counters_snapshot(),
            self.slots.confirmation_timeouts_load(),
        );
        out.dali = stats_dali_dto(&self.counters);
        out.operations = stats_operations_dto(&self.counters.operation_tracker);
        out.websocket = stats_websocket_dto(&self.counters.websocket);
        out.mqtt = stats_mqtt_dto(&self.counters.mqtt);
        out.input = stats_input_dto(&self.counters);
        out.rules = stats_rules_dto(&self.counters);
        out.network = self.link.as_ref().map(|l| stats_network_dto(l.stats()));
    }
}

pub(crate) struct ConfigTransferBridge {
    store: Arc<RegistryStore>,
    slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
    adapter_count: u8,
    wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
}

impl ConfigTransferBridge {
    pub(crate) fn new(
        store: Arc<RegistryStore>,
        slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
        adapter_count: u8,
        wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    ) -> Self {
        Self {
            store,
            slices,
            adapter_count,
            wall_clock,
        }
    }

    fn key(&self, name: &str) -> Option<dali2rust_platform::slice_store::SliceKey> {
        dali2rust_registry_runtime::slice_key_from_name(name, self.adapter_count)
    }
}

impl dali2rust_api::http::handlers::config_transfer::ConfigTransferPort for ConfigTransferBridge {
    fn slice_manifest(
        &self,
    ) -> Vec<dali2rust_api::http::handlers::config_transfer::SliceManifestEntry> {
        let Some(slices) = self.slices.as_ref() else {
            return Vec::new();
        };
        self.store
            .slice_manifest(slices.as_ref(), self.adapter_count)
            .into_iter()
            .map(
                |row| dali2rust_api::http::handlers::config_transfer::SliceManifestEntry {
                    name: row.name,
                    bytes: row.bytes,
                    crc32: row.crc32,
                },
            )
            .collect()
    }

    fn export_slice(&self, name: &str) -> Option<Vec<u8>> {
        let slices = self.slices.as_ref()?;
        let key = self.key(name)?;
        self.store.export_slice(slices.as_ref(), key)
    }

    fn import_slice(
        &self,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), dali2rust_api::http::handlers::config_transfer::ImportRefusal> {
        use dali2rust_api::http::handlers::config_transfer::ImportRefusal;
        let slices = self
            .slices
            .as_ref()
            .ok_or(ImportRefusal::PersistenceDisabled)?;
        let key = self.key(name).ok_or(ImportRefusal::UnknownSlice)?;
        self.store
            .import_slice(slices.as_ref(), key, bytes)
            .map_err(|e| ImportRefusal::StoreFailed(format!("{e:?}")))?;
        apply_imported_timezone(key, self.wall_clock.as_ref(), Some(slices));
        Ok(())
    }
}

pub(crate) struct ReplicationSinkBridge {
    store: Arc<RegistryStore>,
    slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
    adapter_count: u8,
    wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
}

impl ReplicationSinkBridge {
    pub(crate) fn new(
        store: Arc<RegistryStore>,
        slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
        adapter_count: u8,
        wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    ) -> Self {
        Self {
            store,
            slices,
            adapter_count,
            wall_clock,
        }
    }
}

impl dali2rust_redundancy_runtime::ReplicationSink for ReplicationSinkBridge {
    fn local_digests(&self) -> Vec<dali2rust_redundancy_runtime::SliceDigest> {
        let Some(slices) = self.slices.as_ref() else {
            return Vec::new();
        };
        self.store
            .slice_manifest(slices.as_ref(), self.adapter_count)
            .into_iter()
            .map(|row| dali2rust_redundancy_runtime::SliceDigest {
                name: row.name,
                bytes: row.bytes,
                crc32: row.crc32,
            })
            .collect()
    }

    fn write_slice(&self, name: &str, bytes: &[u8]) -> bool {
        let Some(slices) = self.slices.as_ref() else {
            return false;
        };
        let Some(key) = dali2rust_registry_runtime::slice_key_from_name(name, self.adapter_count)
        else {
            return false;
        };
        let written = self.store.import_slice(slices.as_ref(), key, bytes).is_ok();
        if written {
            apply_imported_timezone(key, self.wall_clock.as_ref(), Some(slices));
        }
        written
    }
}

pub(crate) struct RoleBridge {
    dali: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
}

impl RoleBridge {
    pub(crate) fn new(dali: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>) -> Self {
        Self { dali }
    }
}

impl dali2rust_api::http::role::ControllerRolePort for RoleBridge {
    fn is_active(&self) -> bool {
        self.dali.dali_settings_view().application_active
    }
}

pub(crate) struct RedundancyBridge {
    settings: Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
    dali: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    reflex: Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
    counters: Arc<dali2rust_redundancy_runtime::ArbitrationWorkerCounters>,
    transitions: dali2rust_redundancy_runtime::SharedTransitionLog,
    replication: Arc<dali2rust_redundancy_runtime::ReplicationCounters>,
}

impl RedundancyBridge {
    pub(crate) fn new(
        settings: Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
        dali: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
        reflex: Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
        counters: Arc<dali2rust_redundancy_runtime::ArbitrationWorkerCounters>,
        transitions: dali2rust_redundancy_runtime::SharedTransitionLog,
        replication: Arc<dali2rust_redundancy_runtime::ReplicationCounters>,
    ) -> Self {
        Self {
            settings,
            dali,
            reflex,
            counters,
            transitions,
            replication,
        }
    }

    fn replication_row(&self) -> dali2rust_api::http::redundancy_state::RedundancyReplicationDto {
        let c = &self.replication;
        dali2rust_api::http::redundancy_state::RedundancyReplicationDto {
            passes: c.passes.load(Relaxed),
            peer_unreachable: c.peer_unreachable.load(Relaxed),
            pulled: c.slices_pulled.load(Relaxed),
            rejected: c.slices_rejected.load(Relaxed),
            reload_publish_failed: c.reload_publish_failed.load(Relaxed),
        }
    }
}

impl dali2rust_api::http::redundancy_state::RedundancyHttpState for RedundancyBridge {
    fn redundancy_state_dto(&self) -> dali2rust_api::http::redundancy_state::RedundancyStateDto {
        use dali2rust_api::http::redundancy_state as rs;
        let view = self.settings.redundancy_settings_view();
        let now = dali2rust_platform::liveness::monotonic_ms();
        rs::RedundancyStateDto {
            enabled: view.enabled,
            role: if view.standby_role {
                dali2rust_api::http::redundancy_settings_state::ROLE_STANDBY
            } else {
                dali2rust_api::http::redundancy_settings_state::ROLE_PRIMARY
            },
            active: self.dali.dali_settings_view().application_active,
            answering: self.reflex.is_armed(now),
            lease_remaining_ms: self.reflex.lease_remaining_ms(now),
            probes: rs::RedundancyProbeDto {
                published: self.counters.probes_published.load(Relaxed),
                ingress_rejected: self.counters.probes_ingress_rejected.load(Relaxed),
                owned: self.counters.probes_owned.load(Relaxed),
                unowned: self.counters.probes_unowned.load(Relaxed),
            },
            takeovers: self.counters.takeovers.load(Relaxed),
            stand_downs: self.counters.stand_downs.load(Relaxed),
            role_publish_failed: self.counters.role_publish_failed.load(Relaxed),
            ignored_events: self.counters.ignored_events.load(Relaxed),
            replication: self.replication_row(),
            transitions: self.transition_rows(),
        }
    }
}

impl RedundancyBridge {
    fn transition_rows(
        &self,
    ) -> Vec<dali2rust_api::http::redundancy_state::RedundancyTransitionDto> {
        use dali2rust_api::http::redundancy_state as rs;
        let Ok(rows) = self.transitions.lock() else {
            return Vec::new();
        };
        rows.iter()
            .rev()
            .map(|r| rs::RedundancyTransitionDto {
                now_active: r.now_active,
                reason: rs::transition_reason_name(r.reason),
                detected_at_ms: r.detected_at_ms,
                completed_at_ms: r.completed_at_ms,
                took_ms: r.completed_at_ms.wrapping_sub(r.detected_at_ms),
                last_peer_answer_ms: r.last_peer_answer_ms,
                missed_probes: r.missed_probes,
            })
            .collect()
    }
}

fn redundancy_dto(
    supervisor: &dali2rust_redundancy_runtime::ArbitrationSupervisorCounters,
    reflex: &dali2rust_platform::arbitration::ArbitrationReflex,
    worker: (u32, u32),
) -> dali2rust_api::http::diagnostics_state::RedundancyDto {
    let r = reflex.counters();
    dali2rust_api::http::diagnostics_state::RedundancyDto {
        defended: supervisor.defended.load(Relaxed),
        worker_stale: supervisor.stood_down_worker_stale.load(Relaxed),
        answered: r.answered,
        suppressed: r.suppressed_lease,
        cell_busy: r.cell_busy,
        aborted: r.aborted,
        window_closed: r.window_closed,
        late: r.out_of_window,
        probe_failed: worker.0,
        handover_incomplete: worker.1,
        armed: reflex.is_armed(dali2rust_platform::liveness::monotonic_ms()),
    }
}

impl DiagnosticsBridge {
    fn redundancy_block(&self) -> dali2rust_api::http::diagnostics_state::RedundancyDto {
        let worker = &self.counters.dali_worker;
        redundancy_dto(
            &self.counters.arbitration_supervisor,
            &self.counters.arbitration_reflex,
            (
                worker.arbitration_probe_failed.load(Relaxed),
                worker.handover_stand_down_failed.load(Relaxed),
            ),
        )
    }
}

impl DiagnosticsHttpState for DiagnosticsBridge {
    fn diagnostics_dto_into(&self, out: &mut DiagnosticsDto) {
        out.uptime_ms = self.started.elapsed().as_millis() as u64;
        out.bus = BusCountersDto::from(&self.publisher.counters_snapshot());
        out.confirmation_bridge = ConfirmationBridgeDto {
            unmatched: self.slots.unmatched_confirmations_load(),
        };
        out.dali_worker = dali_worker_dto(&self.counters.dali_worker);
        out.sniffer_translator = sniffer_translator_dto(&self.counters.sniffer_translator);
        out.projector = projector_dto(&self.counters.projector);
        out.rules = rules_worker_dto(&self.counters.rules);
        out.apply_orchestrator = apply_orchestrator_dto(&self.counters.apply_orchestrator);
        out.operations = operations_dto(&self.counters.operation_tracker);
        out.registry = dali2rust_api::http::diagnostics_state::RegistryDto {
            runtime_updates_superseded: self
                .counters
                .registry
                .command
                .runtime_updates_superseded
                .load(Relaxed),
            config_write_signal_publish_failed: self
                .counters
                .registry
                .command
                .config_write_signal_publish_failed
                .load(Relaxed),
            ignored_events: self.counters.registry.events.ignored_events.load(Relaxed),
        };
        out.persistence = persistence_dto(self.store.persistence_counters());
        out.phy_sniffer = phy_sniffer_dto(&self.counters.phy_sniffer);
        out.dali_wire = dali_wire_dto(&self.counters.dali_wire);
        out.hcl = hcl_scheduler_dto(&self.counters.hcl_scheduler);
        out.poller = poller_dto(&self.counters.poller);
        out.websocket = websocket_dto(&self.counters.websocket);
        out.mqtt = mqtt_dto(&self.counters.mqtt);
        out.redundancy = self.redundancy_block();
    }

    fn diagnostics_dto(&self) -> DiagnosticsDto {
        let mut dto = DiagnosticsDto::default();
        self.diagnostics_dto_into(&mut dto);
        dto
    }
}

use dali2rust_api::http::input_device_state::{
    instance_type_name, FeedbackDto, InputDeviceDto, InputDeviceHttpState, InputDeviceSummaryDto,
    InstanceDto, InstanceRuntimeDto, ReadValueDto,
};

pub(crate) struct InputDeviceHttpStateBridge {
    store: Arc<RegistryStore>,
}

impl InputDeviceHttpStateBridge {
    pub(crate) fn new(store: Arc<RegistryStore>) -> Self {
        Self { store }
    }
}

const FORCED_EVENT_SCHEME: u8 = 2;

impl InputDeviceHttpState for InputDeviceHttpStateBridge {
    fn list(&self, adapter_id: u8) -> Vec<InputDeviceSummaryDto> {
        self.store
            .input_device_summaries(adapter_id)
            .into_iter()
            .map(|s| InputDeviceSummaryDto {
                adapter_id: s.adapter_id,
                short_address: s.short_address,
                name: s.name,
                present: s.present,
                instance_count: s.instance_count,
                first_instance_type: s.first_instance_type,
                ha_expose: s.ha_expose,
                last_seen_ms: s.last_seen_ms,
                last_event_at_ms: s.last_event_at_ms,
            })
            .collect()
    }

    fn detail(&self, adapter_id: u8, short_address: u8) -> Option<InputDeviceDto> {
        let detail = self.store.input_device_detail(adapter_id, short_address)?;
        let s = detail.summary;
        Some(InputDeviceDto {
            summary: InputDeviceSummaryDto {
                adapter_id: s.adapter_id,
                short_address: s.short_address,
                name: s.name,
                present: s.present,
                instance_count: s.instance_count,
                first_instance_type: s.first_instance_type,
                ha_expose: s.ha_expose,
                last_seen_ms: s.last_seen_ms,
                last_event_at_ms: s.last_event_at_ms,
            },
            notes: detail.notes,
            device_capabilities: s.device_capabilities,
            device_status: s.device_status,
            version_number: s.version_number,
            now_ms: 0,
            nvm_settling_until_ms: detail.nvm_settling_until_ms,
            instances: detail.instances.iter().map(instance_dto).collect(),
        })
    }

    fn revision(&self) -> u32 {
        self.store.input_devices_revision()
    }
}

fn feedback_dto(view: &dali2rust_registry_runtime::InstanceView) -> FeedbackDto {
    FeedbackDto {
        probed: view.feedback_probe.is_some(),
        present: matches!(view.feedback_probe, Some(1 | 2)),
        opcode_map: match view.feedback_probe {
            Some(1) => Some("diia_corrected"),
            Some(2) => Some("ed1"),
            _ => None,
        },
        capability: view.feedback_capability.value,
        colour_capability: view.feedback_colour_capability.value,
        timing: view.feedback_timing.value,
        active_brightness: view.feedback_active_brightness.value,
        active_colour: view.feedback_active_colour.value,
        inactive_brightness: view.feedback_inactive_brightness.value,
        inactive_colour: view.feedback_inactive_colour.value,
    }
}

fn instance_dto(view: &dali2rust_registry_runtime::InstanceView) -> InstanceDto {
    InstanceDto {
        instance_number: view.instance_number,
        instance_type: view.instance_type,
        instance_type_name: view.instance_type.and_then(instance_type_name),
        instance_status: view.instance_status,
        resolution: view.resolution,
        event_scheme: ReadValueDto::new(view.event_scheme.value, view.event_scheme.read_at_ms),
        event_scheme_confirmed: view.event_scheme.value == Some(FORCED_EVENT_SCHEME),
        event_filter: ReadValueDto::new(
            view.event_filter.value.map(|bytes| bytes.to_vec()),
            view.event_filter.read_at_ms,
        ),
        event_priority: ReadValueDto::new(
            view.event_priority.value,
            view.event_priority.read_at_ms,
        ),
        instance_groups: view
            .instance_groups
            .iter()
            .map(|g| ReadValueDto::new(g.value, g.read_at_ms))
            .collect(),
        timers: view
            .timers
            .iter()
            .map(|t| ReadValueDto::new(t.value, t.read_at_ms))
            .collect(),
        manual_config_active: view.manual_config_active,
        feedback: feedback_dto(view),
        runtime: InstanceRuntimeDto {
            last_event_info: view.last_event_info,
            last_event_at_ms: view.last_event_at_ms,
            event_count: view.event_count,
            input_value: view.input_value,
        },
    }
}

use dali2rust_rules_model::{DeviceRef, GroupRef, InputDeviceRef, LampRef, NameResolver};

pub struct RegistryNameResolver {
    store: Arc<RegistryStore>,
    primary_adapter: u8,
    adapter_count: u8,
}

impl RegistryNameResolver {
    #[must_use]
    pub fn new(store: Arc<RegistryStore>, primary_adapter: u8, adapter_count: u8) -> Self {
        Self {
            store,
            primary_adapter,
            adapter_count,
        }
    }
}

impl NameResolver for RegistryNameResolver {
    fn primary_adapter(&self) -> u8 {
        self.primary_adapter
    }

    fn adapter_exists(&self, adapter_id: u8) -> bool {
        adapter_id < self.adapter_count || self.store.adapter_id_exists(adapter_id)
    }

    fn resolve_lamp(&self, name: &str) -> Option<LampRef> {
        self.store
            .virtual_lamp_by_name(name)
            .map(|(adapter_id, id)| LampRef {
                adapter_id,
                id: u16::from(id),
            })
    }

    fn resolve_group(&self, name: &str) -> Option<GroupRef> {
        self.store
            .group_by_name(name)
            .map(|(adapter_id, id)| GroupRef {
                adapter_id,
                id: u16::from(id),
            })
    }

    fn resolve_device(&self, name: &str) -> Option<DeviceRef> {
        self.store
            .physical_device_by_name(name)
            .map(|(adapter_id, short_address)| DeviceRef {
                adapter_id,
                short_address,
            })
    }

    fn resolve_input_device(&self, name: &str) -> Option<InputDeviceRef> {
        self.store
            .input_device_by_name(name)
            .map(|(adapter_id, device_short_address)| InputDeviceRef {
                adapter_id,
                device_short_address,
            })
    }

    fn resolve_scene(&self, name: &str) -> Option<u8> {
        self.store.scene_by_name(name)
    }
}

pub(crate) struct RulesWorldBridge {
    store: Arc<dali2rust_registry_runtime::RegistryStore>,
    wall: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    hcl_state: Arc<dyn dali2rust_api::http::hcl_state::HclScheduleHttpState>,
    overrides: dali2rust_hcl_runtime::SharedOverrideLedger,
    started: std::time::Instant,
}

impl RulesWorldBridge {
    pub(crate) fn new(
        store: Arc<dali2rust_registry_runtime::RegistryStore>,
        wall: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
        hcl_state: Arc<dyn dali2rust_api::http::hcl_state::HclScheduleHttpState>,
        overrides: dali2rust_hcl_runtime::SharedOverrideLedger,
    ) -> Self {
        Self {
            store,
            wall,
            hcl_state,
            overrides,
            started: std::time::Instant::now(),
        }
    }

    fn schedule_targets(
        &self,
        dto: &dali2rust_api::http::hcl_state::HclScheduleDto,
    ) -> Vec<dali2rust_rules_model::LightTarget> {
        let mut targets = Vec::new();
        for t in &dto.targets {
            match t.scope.as_str() {
                "broadcast" => targets.push(dali2rust_rules_model::LightTarget::Broadcast {
                    adapter_id: t.adapter_id,
                }),
                _ => {
                    for group in t.group_ids.clone().unwrap_or_default() {
                        targets.push(dali2rust_rules_model::LightTarget::Group(
                            dali2rust_rules_model::GroupRef {
                                adapter_id: t.adapter_id,
                                id: u16::from(group),
                            },
                        ));
                    }
                }
            }
        }
        targets
    }
}

impl dali2rust_rules_runtime::RulesWorldPort for RulesWorldBridge {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn unix_ms(&self) -> u64 {
        dali2rust_platform::clock::UnixTimeMs::unix_millis(&dali2rust_bsp::unix_clock::StdUnixTimeMs)
    }

    fn wall(&self) -> Option<dali2rust_rules_runtime::runtime::engine::WallTime> {
        let local = self.wall.local()?;
        Some(dali2rust_rules_runtime::runtime::engine::WallTime {
            minutes_of_day: local.minutes_since_midnight,
            weekday: local.weekday,
        })
    }

    fn sun(&self) -> Option<dali2rust_rules_runtime::runtime::engine::SunTimes> {
        let local = self.wall.local()?;
        let location = self
            .hcl_state
            .list_hcl_schedule_dtos()
            .into_iter()
            .find_map(|s| s.location)?;
        let micro = |deg: f64| i32::try_from((deg * 1_000_000.0) as i64).ok();
        let events = dali2rust_hcl_runtime::solar_events(
            dali2rust_hcl_runtime::Location::from_microdeg(
                micro(location.latitude_deg),
                micro(location.longitude_deg),
            )?,
            local.year_day,
            local.utc_offset_minutes,
        );
        let clamp = |m: i32| u16::try_from(m.clamp(0, 1439)).unwrap_or(0);
        Some(dali2rust_rules_runtime::runtime::engine::SunTimes {
            sunrise_min: clamp(events.sunrise_minutes?),
            sunset_min: clamp(events.sunset_minutes?),
        })
    }

    fn controller_active(&self) -> bool {
        self.store.application_controller_active()
    }

    fn lamps(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::LampState> {
        self.store
            .rules_lamp_rows()
            .into_iter()
            .map(|(adapter_id, id, is_on, level, cct_kelvin, last_level)| {
                dali2rust_rules_runtime::runtime::engine::LampState {
                    adapter_id,
                    id,
                    is_on,
                    level,
                    cct_kelvin,
                    last_level,
                }
            })
            .collect()
    }

    fn groups(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::GroupState> {
        self.store
            .rules_group_rows()
            .into_iter()
            .map(|(adapter_id, id, any_on, member_count)| {
                dali2rust_rules_runtime::runtime::engine::GroupState {
                    adapter_id,
                    id,
                    any_on,
                    all_off: !any_on,
                    member_count,
                }
            })
            .collect()
    }

    fn devices(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::DeviceState> {
        let now = dali2rust_bsp::unix_clock::unix_wall_clock_millis();
        self.store
            .rules_device_rows(now)
            .into_iter()
            .map(|(adapter_id, short_address, online)| {
                dali2rust_rules_runtime::runtime::engine::DeviceState {
                    adapter_id,
                    short_address,
                    online,
                }
            })
            .collect()
    }

    fn inputs(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::InputState> {
        self.store
            .rules_input_rows()
            .into_iter()
            .map(
                |(adapter_id, short_address, instance_number, occupied, light)| {
                    dali2rust_rules_runtime::runtime::engine::InputState {
                        adapter_id,
                        short_address,
                        instance_number,
                        occupied,
                        light,
                        position: None,
                        last_event_age_ms: None,
                    }
                },
            )
            .collect()
    }

    fn hcl(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::HclTargetState> {
        let mut rows = Vec::new();
        for dto in self.hcl_state.list_hcl_schedule_dtos() {
            let suspended: Vec<dali2rust_hcl_runtime::SuspendedTarget> = self
                .overrides
                .lock()
                .map(|g| g.suspended_targets(&dto.schedule_id))
                .unwrap_or_default();
            for target in self.schedule_targets(&dto) {
                let overridden = suspended
                    .iter()
                    .any(|s| suspended_matches(&s.target, &target));
                rows.push(dali2rust_rules_runtime::runtime::engine::HclTargetState {
                    target,
                    enabled: dto.enabled,
                    overridden,
                });
            }
        }
        rows
    }

    fn input_instance_groups(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> [Option<u8>; 3] {
        self.store
            .input_device_detail(adapter_id, short_address)
            .and_then(|detail| {
                detail
                    .instances
                    .iter()
                    .find(|i| i.instance_number == instance_number)
                    .map(|i| {
                        [
                            i.instance_groups[0].value.flatten(),
                            i.instance_groups[1].value.flatten(),
                            i.instance_groups[2].value.flatten(),
                        ]
                    })
            })
            .unwrap_or([None; 3])
    }

    fn hcl_schedules_for(&self, target: &dali2rust_rules_model::LightTarget) -> Vec<String> {
        self.hcl_state
            .list_hcl_schedule_dtos()
            .into_iter()
            .filter(|dto| self.schedule_targets(dto).iter().any(|t| t == target))
            .map(|dto| dto.schedule_id)
            .collect()
    }
}

fn suspended_matches(
    suspended: &dali2rust_hcl_runtime::TargetKey,
    target: &dali2rust_rules_model::LightTarget,
) -> bool {
    match target {
        dali2rust_rules_model::LightTarget::Group(group) => {
            suspended.adapter_id == group.adapter_id && suspended.group_id == group.id as u8
        }
        dali2rust_rules_model::LightTarget::Broadcast { adapter_id } => {
            suspended.adapter_id == *adapter_id && suspended.group_id == 0
        }
        dali2rust_rules_model::LightTarget::Lamp(_) => false,
    }
}

pub struct FirmwareBridge {
    state: Arc<dali2rust_ota_runtime::OtaState>,
    port: Option<Arc<dyn dali2rust_platform::firmware::FirmwareUpdatePort>>,
}

impl FirmwareBridge {
    pub fn new(
        state: Arc<dali2rust_ota_runtime::OtaState>,
        port: Option<Arc<dyn dali2rust_platform::firmware::FirmwareUpdatePort>>,
    ) -> Self {
        Self { state, port }
    }
}

impl dali2rust_api::http::firmware_state::FirmwareHttpState for FirmwareBridge {
    fn firmware_dto(&self) -> dali2rust_api::http::firmware_state::FirmwareStateDto {
        use dali2rust_api::http::firmware_state::{FirmwareStateDto, FirmwareUpdateDto};
        let slot = match &self.port {
            Some(port) => port.slot(),
            None => dali2rust_platform::firmware::FirmwareSlot::new("", false, false),
        };
        let snapshot = self.state.snapshot(slot);
        FirmwareStateDto {
            running_slot: slot.label_str().to_string(),
            ota_capable: slot.ota_capable,
            pending_verify: slot.pending_verify,
            update: FirmwareUpdateDto {
                state: snapshot.phase.as_str(),
                url: snapshot.url.as_str().to_string(),
                downloaded_bytes: snapshot.downloaded_bytes,
                total_bytes: snapshot.total_bytes,
                percent: snapshot.percent(),
                error: snapshot.error.map(|e| e.as_str()),
            },
        }
    }
}
