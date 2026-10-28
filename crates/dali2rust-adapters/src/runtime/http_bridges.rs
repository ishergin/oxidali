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
    StatsBusDto, StatsControllerDto, StatsDaliBackwardDto, StatsDaliConsoleDto, StatsDaliDto,
    StatsDaliReadbackDto, StatsDaliTaskTimingDto, StatsHclDto, StatsHttpState, StatsInputDto,
    StatsMqttDto,
    StatsNetworkDto, StatsOperationsDto, StatsReportDto, StatsRuleCountDto, StatsRulesDto,
    StatsWebSocketDto,
};
use dali2rust_api::http::{
    physical_device_state::PhysicalDeviceHttpState, redundancy_state as rs, AdapterHttpState,
    AdapterHttpStateBridge, AdapterSettingsApplyWatch, AdapterSettingsApplyWatchBridge,
    GroupHttpState, GroupHttpStateBridge, GroupMetadataApplyWatch, GroupMetadataApplyWatchBridge,
    HclScheduleHttpState, HclScheduleHttpStateBridge, PhysicalDeviceHttpStateBridge,
    PhysicalDevicePatchWatch, PhysicalDevicePatchWatchBridge, SceneHttpState, SceneHttpStateBridge,
    SceneMetadataApplyWatch, SceneMetadataApplyWatchBridge, VirtualLampBindingApplyWatch,
    VirtualLampBindingApplyWatchBridge, VirtualLampHttpState, VirtualLampHttpStateBridge,
    VirtualLampPatchWatch, VirtualLampPatchWatchBridge,
};
use dali2rust_api::http::handlers::config_transfer::{ImportRefusal, SliceManifestEntry};
use dali2rust_bus::BusPublisher;
use dali2rust_domain::registry::{AdapterReadPort, RegistryReadPort};
use dali2rust_operations_runtime::{ApplyOrchestratorCounters, OperationTrackerCounters};
use dali2rust_platform::clock::Uptime;
use dali2rust_platform::heap::{HeapStats, HeapStatsPort};
use dali2rust_platform::net::{LinkStats, NetworkLink};
use dali2rust_mqtt_runtime::MqttCounters;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_registry_runtime::{
    ForeignSliceOwner, ImportStageRefusal, InputDeviceSummary, StagedSlice,
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

pub(crate) struct RuleTopicsBridge {
    store: std::sync::Arc<dali2rust_rules_runtime::RulesStore>,
}

impl RuleTopicsBridge {
    pub(crate) fn new(store: std::sync::Arc<dali2rust_rules_runtime::RulesStore>) -> Self {
        Self { store }
    }
}

impl dali2rust_mqtt_runtime::RuleTopicsReadPort for RuleTopicsBridge {
    fn rule_topics_generation(&self) -> u32 {
        self.store.generation()
    }

    fn rule_topics(&self) -> Vec<String> {
        self.store.mqtt_topics()
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
    ))
}

#[derive(Clone)]
pub(crate) struct TransferSeams {
    pub slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
    pub adapter_count: u8,
}

pub(crate) struct ForeignSliceBridge {
    slices: Arc<dyn SliceStore>,
    wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
}

impl ForeignSliceBridge {
    pub(crate) fn new(
        slices: Arc<dyn SliceStore>,
        wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    ) -> Self {
        Self { slices, wall_clock }
    }
}

impl ForeignSliceOwner for ForeignSliceBridge {
    fn validate(&self, key: SliceKey, bytes: &[u8]) -> Result<(), String> {
        match key {
            SliceKey::ControllerSettings => crate::runtime::time_settings::validate_settings_slice(bytes),
            SliceKey::Rules { bank } => dali2rust_rules_runtime::runtime::persistence::validate_bank(bank, bytes)
                .map_err(str::to_string),
            other => Err(format!("no owner reads {}", other.label())),
        }
    }

    fn imported(&self, key: SliceKey) {
        if key == SliceKey::ControllerSettings {
            crate::runtime::time_settings::hydrate_timezone(self.wall_clock.as_ref(), Some(&self.slices));
        }
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
    uptime: Uptime,
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    counters: RuntimeCounterHandles,
    store: Arc<RegistryStore>,
}

impl DiagnosticsBridge {
    pub(crate) fn new(
        uptime: Uptime,
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        counters: RuntimeCounterHandles,
        store: Arc<RegistryStore>,
    ) -> Self {
        Self {
            uptime,
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
        input_events_ambiguous_scheme, input_lifecycle, fact_publish_retried,
        app_control_pairs,
    }

    rules_worker_dto(c: dali2rust_rules_runtime::RulesWorkerCounters) -> RulesWorkerDto {
        commits_applied, commits_rejected, enable_toggles, hydrate_failed,
        persist_failed, ignored_commands, effects_published, effects_ingress_rejected,
        effects_skipped_dark, log_lines, activations_published,
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
        hydrate_default_total, hydrate_error_total, unread_slices,
    }

    hcl_scheduler_dto(c: dali2rust_hcl_runtime::HclSchedulerCounters) -> HclSchedulerDto {
        ticks, ticks_time_unsynced, commands_published, commands_dropped_cap,
        command_timeouts, command_failures, ingress_rejections,
        overrides_started, overrides_cleared, overrides_reset, ignored_commands,
        ignored_events,
    }

    poller_dto(c: dali2rust_poller_runtime::PollerCounters) -> PollerDto {
        cycles_total, reads_published, reads_completed, reads_failed, reads_preempted,
        reads_absent, outstanding_expired, skipped_inbox_full, duty_deferred,
        interactive_deferred, targets_excluded, window_deferred, device_cooldowns,
        health_probes_published, health_probes_invalid, health_probes_clear,
        health_probes_one_failure, health_probes_several_failures, health_probes_expired,
        health_probes_power_failure, sweeps, sweep_reads,
        ignored_events,
    }

    phy_sniffer_dto(c: dali2rust_platform::dali::PhySnifferCounters) -> PhySnifferDto {
        frames, backward8, forward16, forward24, decode_failed, unsupported_len, dropped,
        answer_table_torn, answer_decode_overrun,
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

pub(crate) struct ControllerSummaryBridge {
    settings: Arc<dyn dali2rust_domain::registry::HomeAssistantSettingsReadPort>,
    counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    link: Option<Arc<dyn NetworkLink>>,
}

impl ControllerSummaryBridge {
    pub(crate) fn new(
        settings: Arc<dyn dali2rust_domain::registry::HomeAssistantSettingsReadPort>,
        counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
        link: Option<Arc<dyn NetworkLink>>,
    ) -> Self {
        Self { settings, counters, link }
    }
}

impl dali2rust_api::http::handlers::controller::ControllerSummarySource for ControllerSummaryBridge {
    fn facts(&self) -> dali2rust_api::http::handlers::controller::ControllerFacts {
        let view = self.settings.home_assistant_settings_view();
        let link = self.link.as_deref();
        dali2rust_api::http::handlers::controller::ControllerFacts {
            broker_url: view.broker_url_view(),
            ha_enabled: view.enabled,
            installation_id: view.controller_id,
            ha_connected: self.counters.is_connected(),
            ha_own_topics_refused: self.counters.own_topics_refused(),
            mac: link.and_then(|l| l.hardware_address()),
            hostname: link.and_then(|l| l.hostname()),
            ipv4: link.and_then(|l| l.status().ipv4),
        }
    }
}

fn stats_mqtt_dto(c: &MqttCounters) -> StatsMqttDto {
    StatsMqttDto {
        connected: c.is_connected(),
        publishes_total: MqttCounters::load(&c.publishes_total),
        publish_failures_total: MqttCounters::load(&c.publish_failures_total),
        rule_messages_total: MqttCounters::load(&c.rule_messages_total),
        rule_messages_coalesced_total: MqttCounters::load(&c.rule_messages_coalesced_total),
        rule_messages_lost_total: MqttCounters::load(&c.rule_messages_lost_total),
        subscriptions_refused_total: MqttCounters::load(&c.subscriptions_refused_total),
        own_topics_refused_total: MqttCounters::load(&c.own_topics_refused_total),
    }
}

fn stats_hcl_dto(counters: &dali2rust_hcl_runtime::HclSchedulerCounters) -> StatsHclDto {
    StatsHclDto { ticks_cut_total: counters.ticks_cut.load(Relaxed) }
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
        effects_unbound: counters.rules.effects_unbound.load(Relaxed),
        hcl_switches_coalesced: counters.registry.command.hcl_switches_coalesced.load(Relaxed),
        continuations_scheduled: c.continuations_scheduled.load(Relaxed),
        continuations_fired: c.continuations_fired.load(Relaxed),
        continuations_dropped: c.continuations_dropped.load(Relaxed),
        continuations_pending: c.continuations_pending.load(Relaxed),
        timers_active: c.timers_active.load(Relaxed),
        ticks_time_unsynced: c.ticks_time_unsynced.load(Relaxed),
        rules_loaded: c.rules_loaded.load(Relaxed),
        vars_in_use: c.vars_in_use.load(Relaxed),
        latency_p50_ms: c.latency_p50_ms.load(Relaxed),
        latency_p95_ms: c.latency_p95_ms.load(Relaxed),
        latency_max_ms: c.latency_max_ms.load(Relaxed),
        stats: rule_stat_rows(c),
    }
}

fn rule_stat_rows(cells: &dali2rust_rules_runtime::RulesEngineCells) -> Vec<StatsRuleCountDto> {
    let rows = cells.stat_counts().into_iter();
    rows.map(|row| StatsRuleCountDto { name: row.name, count: row.count }).collect()
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
    uptime: Uptime,
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    counters: RuntimeCounterHandles,
    heap: Option<Arc<dyn HeapStatsPort>>,
    link: Option<Arc<dyn NetworkLink>>,
}

impl StatsBridge {
    pub(crate) fn new(
        uptime: Uptime,
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        counters: RuntimeCounterHandles,
        platform: PlatformGauges,
    ) -> Self {
        Self {
            uptime,
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
    StatsDaliDto {
        commands_executed_total: c.commands_handled.load(Relaxed),
        errors_total: c.errors_total(),
        target_state_superseded_total: c.target_state_superseded.load(Relaxed),
        wire_load_permille: h.dali_wire.load_permille.load(Relaxed),
        wire_load_own_permille: h.dali_wire.load_own_permille.load(Relaxed),
        collision_restarts_total: h.dali_wire.collision_restarts.load(Relaxed),
        foreign_frames_total: h.phy_sniffer.foreign_frames_total(),
        foreign_verbs_projected_total: h.projector.transitions_expanded.load(Relaxed),
        foreign_dimming_unprojected_total: h.sniffer_translator.dimming_unprojectable.load(Relaxed),
        foreign_scene_writes_total: h.sniffer_translator.scene_writes_observed.load(Relaxed),
        foreign_unaddressed_ignored_total: h.sniffer_translator.unaddressed_ignored.load(Relaxed),
        backward: stats_backward_dto(&h.phy_sniffer),
        console: stats_console_dto(),
        isr_ticks_deficit_raw_total: h.phy_sniffer.isr_ticks_deficit_raw.load(Relaxed),
        isr_ticks_surplus_raw_total: h.phy_sniffer.isr_ticks_surplus_raw.load(Relaxed),
        isr_ticks_lost_total: h.phy_sniffer.isr_ticks_lost.load(Relaxed),
        isr_ticks_extra_total: h.phy_sniffer.isr_ticks_extra.load(Relaxed),
        isr_late_ticks_total: h.phy_sniffer.isr_late_ticks.load(Relaxed),
        isr_max_gap_us: h.phy_sniffer.isr_max_gap_us.load(Relaxed),
        task_timing: stats_task_timing_dto(h),
        readback: stats_readback_dto(&h.dali_wire),
    }
}

fn stats_backward_dto(s: &dali2rust_platform::dali::PhySnifferCounters) -> StatsDaliBackwardDto {
    StatsDaliBackwardDto {
        backward_undecodable_total: s.backward_undecodable.load(Relaxed),
        backward_frame_size_total: s.backward_frame_size.load(Relaxed),
        backward_incomplete_total: s.backward_incomplete.load(Relaxed),
        backward_early_rejected_total: s.backward_early_rejected.load(Relaxed),
        backward_late_rejected_total: s.backward_late_rejected.load(Relaxed),
        backward_multi_answer_total: s.backward_multi_answer.load(Relaxed),
    }
}

fn stats_console_dto() -> StatsDaliConsoleDto {
    let console = crate::log::console_counters();
    StatsDaliConsoleDto {
        console_log_dropped_total: console.dropped.load(Relaxed),
        console_log_busy_total: console.busy.load(Relaxed),
        console_log_truncated_total: console.truncated.load(Relaxed),
        console_log_unavailable_total: console.unavailable.load(Relaxed),
        console_uart_errors_total: console.uart_errors.load(Relaxed),
    }
}

fn stats_readback_dto(w: &dali2rust_platform::dali::DaliWireCounters) -> StatsDaliReadbackDto {
    StatsDaliReadbackDto {
        readback_groups_corrected_total: w.readback_groups_corrected.load(Relaxed),
        readback_colour_features_corrected_total: w.readback_colour_features_corrected.load(Relaxed),
        readback_extended_fade_corrected_total: w.readback_extended_fade_corrected.load(Relaxed),
        program_repairs_total: w.program_repairs.load(Relaxed),
    }
}

fn stats_task_timing_dto(h: &RuntimeCounterHandles) -> StatsDaliTaskTimingDto {
    let s = &h.phy_sniffer;
    let flush = dali2rust_platform::dali::persist_flush_stats();
    let gate = dali2rust_platform::dali::persist_gate_stats();
    StatsDaliTaskTimingDto {
        answer_staged_total: s.answer_staged.load(Relaxed),
        answer_table_behind_total: s.answer_table_behind.load(Relaxed),
        answer_table_publish_max_ticks: s.answer_table_publish_max_ticks.load(Relaxed),
        sniff_poll_late_total: s.sniff_poll_late.load(Relaxed),
        sniff_poll_gap_max_us: s.sniff_poll_gap_max_us.load(Relaxed),
        persist_flush_total: flush.flushes,
        persist_flush_slow_total: flush.slow,
        persist_flush_ms_total: flush.ms_total,
        persist_flush_max_ms: flush.max_ms,
        persist_gate_waits_total: gate.waits,
        persist_gate_timeouts_total: gate.timeouts,
    }
}

fn stats_operations_dto(c: &OperationTrackerCounters) -> StatsOperationsDto {
    let totals = c.totals();
    StatsOperationsDto {
        running: totals.in_flight(),
        succeeded_total: totals.succeeded,
        failed_total: totals.failed,
        timed_out_total: totals.timed_out,
        cancelled_total: totals.cancelled,
    }
}

impl StatsHttpState for StatsBridge {
    fn stats_dto(&self) -> StatsReportDto {
        let mut dto = StatsReportDto::default();
        self.stats_dto_into(&mut dto);
        dto
    }

    fn stats_dto_into(&self, out: &mut StatsReportDto) {
        let uptime_ms = self.uptime.elapsed_ms();
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
        out.hcl = stats_hcl_dto(&self.counters.hcl_scheduler);
        out.rules = stats_rules_dto(&self.counters);
        out.network = self.link.as_ref().map(|l| stats_network_dto(l.stats()));
    }
}

fn manifest_entries(
    store: &RegistryStore,
    slices: Option<&Arc<dyn SliceStore>>,
    adapter_count: u8,
) -> Vec<SliceManifestEntry> {
    let Some(slices) = slices else {
        return Vec::new();
    };
    store
        .slice_manifest(slices.as_ref(), adapter_count)
        .into_iter()
        .map(|row| SliceManifestEntry { name: row.name, bytes: row.bytes, crc32: row.crc32 })
        .collect()
}

pub(crate) struct ConfigTransferBridge {
    store: Arc<RegistryStore>,
    slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
    adapter_count: u8,
}

impl ConfigTransferBridge {
    pub(crate) fn new(
        store: Arc<RegistryStore>,
        slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
        adapter_count: u8,
    ) -> Self {
        Self {
            store,
            slices,
            adapter_count,
        }
    }

    fn key(&self, name: &str) -> Option<dali2rust_platform::slice_store::SliceKey> {
        dali2rust_registry_runtime::slice_key_from_name(name, self.adapter_count)
    }
}

impl dali2rust_api::http::handlers::config_transfer::ConfigTransferPort for ConfigTransferBridge {
    fn slice_manifest(&self) -> Vec<SliceManifestEntry> {
        manifest_entries(&self.store, self.slices.as_ref(), self.adapter_count)
    }

    fn export_slice(&self, name: &str) -> Option<Vec<u8>> {
        let slices = self.slices.as_ref()?;
        let key = self.key(name)?;
        self.store.export_slice(slices.as_ref(), key)
    }

    fn stage_import(
        &self,
        workflow: u64,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), ImportRefusal> {
        if self.slices.is_none() {
            return Err(ImportRefusal::PersistenceDisabled);
        }
        let key = self.key(name).ok_or(ImportRefusal::UnknownSlice)?;
        let staged = vec![StagedSlice { key, bytes: bytes.to_vec() }];
        self.store
            .stage_import(workflow, staged)
            .map_err(|ImportStageRefusal::Busy| ImportRefusal::ImportBusy)
    }

    fn discard_import(&self, workflow: u64) {
        self.store.discard_import(workflow);
    }
}

pub(crate) struct ReplicationSinkBridge {
    store: Arc<RegistryStore>,
    slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
    adapter_count: u8,
}

impl ReplicationSinkBridge {
    pub(crate) fn new(
        store: Arc<RegistryStore>,
        slices: Option<Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
        adapter_count: u8,
    ) -> Self {
        Self {
            store,
            slices,
            adapter_count,
        }
    }

    fn key(&self, name: &str) -> Option<SliceKey> {
        dali2rust_registry_runtime::slice_key_from_name(name, self.adapter_count)
    }

    fn accepted_key(&self, name: &str) -> Option<SliceKey> {
        self.slices.as_ref()?;
        self.key(name)
    }

    fn staged(&self, slice: dali2rust_redundancy_runtime::PulledSlice) -> Option<StagedSlice> {
        let key = self.accepted_key(&slice.name)?;
        Some(StagedSlice { key, bytes: slice.bytes })
    }
}

impl dali2rust_redundancy_runtime::ReplicationSink for ReplicationSinkBridge {
    fn local_digests(&self) -> Vec<dali2rust_redundancy_runtime::SliceDigest> {
        manifest_entries(&self.store, self.slices.as_ref(), self.adapter_count)
            .into_iter()
            .map(dali2rust_redundancy_runtime::SliceDigest::from)
            .collect()
    }

    fn accepts(&self, name: &str) -> bool {
        self.accepted_key(name).is_some()
    }

    fn family(&self, name: &str) -> String {
        self.key(name).map_or_else(|| name.to_string(), |key| key.family().label())
    }

    fn stage(&self, pulled: Vec<dali2rust_redundancy_runtime::PulledSlice>) -> bool {
        let staged: Option<Vec<StagedSlice>> =
            pulled.into_iter().map(|slice| self.staged(slice)).collect();
        staged.is_some_and(|staged| {
            self.store
                .stage_import(dali2rust_contracts::CORRELATION_NONE, staged)
                .is_ok()
        })
    }

    fn discard_staged(&self) {
        self.store.discard_import(dali2rust_contracts::CORRELATION_NONE);
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
    counters: RuntimeCounterHandles,
    transitions: dali2rust_redundancy_runtime::SharedTransitionLog,
}

fn arbitration_dto(
    supervisor: &dali2rust_redundancy_runtime::ArbitrationSupervisorCounters,
    answers: dali2rust_platform::arbitration::ArbitrationAnswerCounters,
    worker: &dali2rust_dali_runtime::DaliWorkerCounters,
) -> dali2rust_api::http::redundancy_state::RedundancyArbitrationDto {
    dali2rust_api::http::redundancy_state::RedundancyArbitrationDto {
        defended: supervisor.defended.load(Relaxed),
        worker_stale: supervisor.stood_down_worker_stale.load(Relaxed),
        answered: answers.answered,
        suppressed: answers.suppressed_lease,
        cell_busy: answers.cell_busy,
        aborted: answers.aborted,
        window_closed: answers.window_closed,
        late: answers.out_of_window,
        probe_failed: worker.arbitration_probe_failed.load(Relaxed),
        handover_incomplete: worker.handover_stand_down_failed.load(Relaxed),
    }
}

fn role_name(standby: bool) -> &'static str {
    if standby {
        dali2rust_api::http::redundancy_settings_state::ROLE_STANDBY
    } else {
        dali2rust_api::http::redundancy_settings_state::ROLE_PRIMARY
    }
}

impl RedundancyBridge {
    pub(crate) fn new(
        settings: Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
        dali: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
        counters: RuntimeCounterHandles,
        transitions: dali2rust_redundancy_runtime::SharedTransitionLog,
    ) -> Self {
        Self {
            settings,
            dali,
            counters,
            transitions,
        }
    }

    fn arbitration_row(&self) -> dali2rust_api::http::redundancy_state::RedundancyArbitrationDto {
        arbitration_dto(
            &self.counters.arbitration_supervisor,
            self.counters.answer_inputs.counters(),
            &self.counters.dali_worker,
        )
    }

    fn probe_row(&self) -> dali2rust_api::http::redundancy_state::RedundancyProbeDto {
        let worker = &self.counters.arbitration_worker;
        dali2rust_api::http::redundancy_state::RedundancyProbeDto {
            published: worker.probes_published.load(Relaxed),
            ingress_rejected: worker.probes_ingress_rejected.load(Relaxed),
            owned: worker.probes_owned.load(Relaxed),
            unowned: worker.probes_unowned.load(Relaxed),
        }
    }

    fn replication_row(&self) -> dali2rust_api::http::redundancy_state::RedundancyReplicationDto {
        let c = &self.counters.replication;
        dali2rust_api::http::redundancy_state::RedundancyReplicationDto {
            passes: c.passes.load(Relaxed),
            peer_unreachable: c.peer_unreachable.load(Relaxed),
            pulled: c.slices_pulled.load(Relaxed),
            rejected: c.slices_rejected.load(Relaxed),
            reload_publish_failed: c.reload_publish_failed.load(Relaxed),
            wakes_sent: c.wakes_sent.load(Relaxed),
            wakes_failed: c.wakes_failed.load(Relaxed),
            wakes_received: c.wakes_received.load(Relaxed),
        }
    }
}

impl dali2rust_api::http::redundancy_state::RedundancyHttpState for RedundancyBridge {
    fn redundancy_state_dto(&self) -> dali2rust_api::http::redundancy_state::RedundancyStateDto {
        let view = self.settings.redundancy_settings_view();
        let now = dali2rust_platform::liveness::monotonic_ms();
        let inputs = &self.counters.answer_inputs;
        let worker = &self.counters.arbitration_worker;
        rs::RedundancyStateDto {
            now_ms: now,
            answering: inputs.is_answering(now),
            lease_remaining_ms: inputs.lease_remaining_ms(now),
            enabled: view.enabled,
            role: role_name(view.standby_role),
            active: self.dali.dali_settings_view().application_active,
            arbitration: self.arbitration_row(),
            probes: self.probe_row(),
            takeovers: worker.takeovers.load(Relaxed),
            stand_downs: worker.stand_downs.load(Relaxed),
            role_publish_failed: worker.role_publish_failed.load(Relaxed),
            ignored_events: worker.ignored_events.load(Relaxed),
            replication: self.replication_row(),
            transitions: self.transition_rows(),
        }
    }
}

impl RedundancyBridge {
    fn transition_rows(
        &self,
    ) -> Vec<dali2rust_api::http::redundancy_state::RedundancyTransitionDto> {
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
                took_ms: r.took_ms(),
                last_peer_answer_ms: r.last_peer_answer_ms,
                missed_probes: r.missed_probes,
            })
            .collect()
    }
}

impl DiagnosticsHttpState for DiagnosticsBridge {
    fn diagnostics_dto_into(&self, out: &mut DiagnosticsDto) {
        out.uptime_ms = self.uptime.elapsed_ms();
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
    }

    fn diagnostics_dto(&self) -> DiagnosticsDto {
        let mut dto = DiagnosticsDto::default();
        self.diagnostics_dto_into(&mut dto);
        dto
    }
}

use dali2rust_api::http::input_device_state::{
    instance_type_name, opcode_map_name, FeedbackDto, InputDeviceDto, InputDeviceHttpState,
    InputDeviceSummaryDto, InstanceDto, InstanceRuntimeDto, ReadValueDto,
};

pub(crate) struct InputDeviceHttpStateBridge {
    store: Arc<RegistryStore>,
}

impl InputDeviceHttpStateBridge {
    pub(crate) fn new(store: Arc<RegistryStore>) -> Self {
        Self { store }
    }
}

impl InputDeviceHttpState for InputDeviceHttpStateBridge {
    fn adapter_count(&self) -> u8 {
        AdapterReadPort::adapter_count(self.store.as_ref())
    }

    fn list(&self, adapter_id: u8) -> Vec<InputDeviceSummaryDto> {
        self.store
            .input_device_summaries(adapter_id)
            .into_iter()
            .map(summary_dto)
            .collect()
    }

    fn detail(&self, adapter_id: u8, short_address: u8) -> Option<InputDeviceDto> {
        let detail = self.store.input_device_detail(adapter_id, short_address)?;
        let s = detail.summary;
        let (device_capabilities, device_status, version_number) =
            (s.device_capabilities, s.device_status, s.version_number);
        Some(InputDeviceDto {
            summary: summary_dto(s),
            notes: detail.notes,
            device_capabilities,
            device_status,
            version_number,
            now_ms: 0,
            nvm_settling_until_ms: detail.nvm_settling_until_ms,
            instances: detail.instances.iter().map(instance_dto).collect(),
        })
    }

    fn revision(&self) -> u32 {
        self.store.input_devices_revision()
    }
}

fn summary_dto(s: InputDeviceSummary) -> InputDeviceSummaryDto {
    InputDeviceSummaryDto {
        adapter_id: s.adapter_id,
        short_address: s.short_address,
        name: s.name,
        present: s.present,
        instance_count: s.instance_count,
        first_instance_type: s.first_instance_type,
        ha_expose: s.ha_expose,
        last_seen_ms: s.last_seen_ms,
        last_event_at_ms: s.last_event_at_ms,
    }
}

fn feedback_dto(view: &dali2rust_registry_runtime::InstanceView) -> FeedbackDto {
    let map = view.feedback_map();
    FeedbackDto {
        probed: view.feedback_probe.is_some(),
        present: map.is_some(),
        opcode_map: map.map(opcode_map_name),
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
        event_scheme_confirmed: view.event_scheme_confirmed(),
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

use dali2rust_domain::registry::{GROUP_COUNT, VIRTUAL_LAMP_COUNT};
use dali2rust_rules_model::limits::{MAX_GROUP_ID, MAX_LAMP_ID};
use dali2rust_rules_model::{DeviceRef, GroupRef, InputDeviceRef, LampRef, NameResolver, SceneRef};
use dali2rust_api::http::firmware_state::{FirmwareStateDto, FirmwareUpdateDto};

const _: () = assert!(MAX_LAMP_ID as u16 + 1 == VIRTUAL_LAMP_COUNT as u16);
const _: () = assert!(MAX_GROUP_ID as u16 + 1 == GROUP_COUNT as u16);

pub struct RegistryNameResolver {
    store: Arc<RegistryStore>,
    primary_adapter: u8,
}

impl RegistryNameResolver {
    #[must_use]
    pub fn new(store: Arc<RegistryStore>, primary_adapter: u8) -> Self {
        Self { store, primary_adapter }
    }
}

impl NameResolver for RegistryNameResolver {
    fn primary_adapter(&self) -> u8 {
        self.primary_adapter
    }

    fn adapter_exists(&self, adapter_id: u8) -> bool {
        self.store.adapter_id_exists(adapter_id)
    }

    fn resolve_lamp(&self, name: &str) -> Vec<LampRef> {
        self.store
            .virtual_lamps_named(name)
            .into_iter()
            .map(|(adapter_id, id)| LampRef { adapter_id, id: u16::from(id) })
            .collect()
    }

    fn resolve_group(&self, name: &str) -> Vec<GroupRef> {
        self.store
            .groups_named(name)
            .into_iter()
            .map(|(adapter_id, id)| GroupRef { adapter_id, id: u16::from(id) })
            .collect()
    }

    fn resolve_device(&self, name: &str) -> Vec<DeviceRef> {
        self.store
            .physical_devices_named(name)
            .into_iter()
            .map(|(adapter_id, short_address)| DeviceRef { adapter_id, short_address })
            .collect()
    }

    fn resolve_input_device(&self, name: &str) -> Vec<InputDeviceRef> {
        self.store
            .input_devices_named(name)
            .into_iter()
            .map(|(adapter_id, device_short_address)| InputDeviceRef {
                adapter_id,
                device_short_address,
            })
            .collect()
    }

    fn resolve_scene(&self, name: &str) -> Vec<SceneRef> {
        self.store
            .scenes_named(name)
            .into_iter()
            .map(|(adapter_id, id)| SceneRef { adapter_id, id })
            .collect()
    }
}

pub(crate) struct RulesWorldBridge {
    store: Arc<dali2rust_registry_runtime::RegistryStore>,
    wall: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    hcl: dali2rust_hcl_runtime::HclRulesView,
}

impl RulesWorldBridge {
    pub(crate) fn new(
        store: Arc<dali2rust_registry_runtime::RegistryStore>,
        wall: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
        hcl: dali2rust_hcl_runtime::HclRulesView,
    ) -> Self {
        Self { store, wall, hcl }
    }
}

fn rules_hcl_target(key: dali2rust_hcl_runtime::TargetKey) -> dali2rust_rules_model::LightTarget {
    match key.scope {
        dali2rust_contracts::msg::HclTargetScope::Broadcast => {
            dali2rust_rules_model::LightTarget::Broadcast { adapter_id: key.adapter_id }
        }
        dali2rust_contracts::msg::HclTargetScope::Group => {
            dali2rust_rules_model::LightTarget::Group(dali2rust_rules_model::GroupRef {
                adapter_id: key.adapter_id,
                id: u16::from(key.group_id),
            })
        }
    }
}

impl dali2rust_rules_runtime::RulesWorldPort for RulesWorldBridge {
    fn now_ms(&self) -> u64 {
        dali2rust_bsp::monotonic_clock::monotonic_millis()
    }

    fn unix_ms(&self) -> u64 {
        dali2rust_platform::clock::UnixTimeMs::unix_millis(&dali2rust_bsp::unix_clock::StdUnixTimeMs)
    }

    fn wall(&self) -> Option<dali2rust_rules_runtime::runtime::engine::WallTime> {
        self.wall.local().map(Into::into)
    }

    fn sun(&self) -> Option<dali2rust_rules_runtime::runtime::engine::SunTimes> {
        let sun = self.hcl.sun(self.wall.local()?)?;
        Some(dali2rust_rules_runtime::runtime::engine::SunTimes {
            sunrise_min: sun.sunrise,
            sunset_min: sun.sunset,
        })
    }

    fn controller_active(&self) -> bool {
        self.store.application_controller_active()
    }

    fn lamps(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::LampState> {
        self.store
            .rules_lamp_rows()
            .into_iter()
            .map(|row| dali2rust_rules_runtime::runtime::engine::LampState {
                adapter_id: row.adapter_id,
                id: row.id,
                is_on: row.is_on,
                level: row.level,
                cct_kelvin: row.cct_kelvin,
                last_level: row.last_level,
                bound: row.bound,
            })
            .collect()
    }

    fn groups(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::GroupState> {
        self.store
            .rules_group_rows()
            .into_iter()
            .map(|row| dali2rust_rules_runtime::runtime::engine::GroupState {
                adapter_id: row.adapter_id,
                id: row.id,
                any_on: row.any_on,
                all_off: row.all_off(),
                member_count: row.member_count,
            })
            .collect()
    }

    fn devices(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::DeviceState> {
        let now = dali2rust_bsp::unix_clock::unix_wall_clock_millis();
        self.store
            .rules_device_rows(now)
            .into_iter()
            .map(|row| dali2rust_rules_runtime::runtime::engine::DeviceState {
                adapter_id: row.adapter_id,
                short_address: row.short_address,
                online: row.online,
            })
            .collect()
    }

    fn inputs(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::InputState> {
        let now = dali2rust_bsp::monotonic_clock::observation_stamp_ms();
        self.store
            .rules_input_rows(now)
            .into_iter()
            .map(|row| dali2rust_rules_runtime::runtime::engine::InputState {
                adapter_id: row.adapter_id,
                short_address: row.short_address,
                instance_number: row.instance_number,
                instance_type: row.instance_type,
                occupied: row.occupied,
                light: row.light,
                position: row.position,
                last_event_age_ms: row.last_event_age_ms,
            })
            .collect()
    }

    fn hcl(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::HclTargetState> {
        self.hcl
            .target_statuses()
            .into_iter()
            .map(|row| dali2rust_rules_runtime::runtime::engine::HclTargetState {
                target: rules_hcl_target(row.target),
                enabled: row.enabled,
                overridden: row.overridden,
            })
            .collect()
    }

    fn hcl_schedules(&self) -> Vec<String> {
        self.store.hcl_schedule_ids()
    }

    fn input_instance_groups(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> [Option<u8>; 3] {
        self.store.input_instance_groups(adapter_id, short_address, instance_number)
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

#[cfg(test)]
mod tests {
    use core::sync::atomic::Ordering::Relaxed;
    use std::sync::{Arc, Mutex};

    use super::{
        arbitration_dto, stats_readback_dto, ConfigTransferBridge, ReplicationSinkBridge,
    };
    use dali2rust_api::http::handlers::config_transfer::ConfigTransferPort;
    use dali2rust_bsp::slice_store_files::InMemorySliceStore;
    use dali2rust_platform::slice_store::{SliceKey, SliceStore};
    use dali2rust_redundancy_runtime::{PulledSlice, ReplicationSink};
    use dali2rust_registry_runtime::{InputDeviceSummary, RegistryStore, StagedSlice};
    use dali2rust_test_support::write_slice;
    use dali2rust_contracts::msg::HclTargetScope;
    use dali2rust_rules_model::{GroupRef, LightTarget};
    use crate::dali::transport::mock::MockDaliTransport;
    use dali2rust_contracts::msg::GroupMembershipAction;
    use dali2rust_dali_runtime::runtime::controller::DaliController;
    use dali2rust_dali_runtime::runtime::executor::program_group_membership;
    use dali2rust_dali_runtime::StdClock;
    use dali2rust_domain::dali::commands::{DaliCommand, StandardCommand};
    use dali2rust_domain::dali::types::DaliAddress;
    use dali2rust_platform::arbitration::ArbitrationAnswerCounters;
    use dali2rust_platform::dali::DaliWireCounters;

    const SHORT: u8 = 4;
    const GROUP: u8 = 3;
    const ADAPTERS: u8 = 1;
    const KNOWN_SLICE: &str = "poller_settings";
    const OTHER_KNOWN_SLICE: &str = "hcl_schedules";
    const SLICE_BYTES: [u8; 3] = [1, 2, 3];
    const SLICE_BYTES_CRC32: u32 = 0x55BC_801D;
    const UNKNOWN_SLICE: &str = "no_such_slice";
    const OTHER_WORKFLOW: u64 = 77;

    fn frame(command: StandardCommand) -> u16 {
        let address = DaliAddress::short(SHORT).expect("short address");
        DaliCommand::Standard { address, command }.to_forward_frame().raw()
    }

    fn script_group_add(mock: &MockDaliTransport, readback: u8) {
        let add = frame(StandardCommand::AddToGroup { group: GROUP });
        mock.expect_forward_frame(add);
        mock.expect_forward_frame(add);
        mock.expect_forward_frame_with_backward(frame(StandardCommand::QueryGroups0To7), Some(readback));
        mock.expect_forward_frame_with_backward(frame(StandardCommand::QueryGroups8To15), Some(0));
    }

    #[test]
    fn a_repair_the_group_programme_spends_reaches_program_repairs_total_and_no_other_field() {
        let mock = MockDaliTransport::new();
        script_group_add(&mock, 0);
        script_group_add(&mock, 1 << GROUP);
        let transport = Arc::new(Mutex::new(mock));
        let mut controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
        let wire = Arc::new(DaliWireCounters::default());
        controller.set_wire_counters(Arc::clone(&wire));

        program_group_membership(&mut controller, SHORT, GROUP, GroupMembershipAction::Add)
            .expect("the second drive converges");

        let row = stats_readback_dto(&wire);
        let got = [
            row.readback_groups_corrected_total,
            row.readback_colour_features_corrected_total,
            row.readback_extended_fade_corrected_total,
            row.program_repairs_total,
        ];
        assert_eq!(got, [0, 0, 0, 1]);
        assert_eq!(transport.lock().expect("mock lock").scripted_exchanges_remaining(), 0);
    }

    #[test]
    fn every_arbitration_field_reads_its_own_producer() {
        let supervisor = dali2rust_redundancy_runtime::ArbitrationSupervisorCounters::default();
        supervisor.defended.store(1, Relaxed);
        supervisor.stood_down_worker_stale.store(2, Relaxed);
        let answers = ArbitrationAnswerCounters {
            answered: 3,
            suppressed_lease: 4,
            cell_busy: 5,
            aborted: 6,
            window_closed: 7,
            out_of_window: 8,
        };
        let worker = dali2rust_dali_runtime::DaliWorkerCounters::default();
        worker.arbitration_probe_failed.store(9, Relaxed);
        worker.handover_stand_down_failed.store(10, Relaxed);

        let row = arbitration_dto(&supervisor, answers, &worker);
        let got = [
            row.defended,
            row.worker_stale,
            row.answered,
            row.suppressed,
            row.cell_busy,
            row.aborted,
            row.window_closed,
            row.late,
            row.probe_failed,
            row.handover_incomplete,
        ];
        assert_eq!(got, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    fn sink_over(slices: Option<Arc<dyn SliceStore>>) -> (ReplicationSinkBridge, Arc<RegistryStore>) {
        let store = Arc::new(RegistryStore::with_adapter_count(ADAPTERS));
        (ReplicationSinkBridge::new(Arc::clone(&store), slices, ADAPTERS), store)
    }

    fn pulled(name: &str) -> PulledSlice {
        PulledSlice { name: name.to_string(), bytes: SLICE_BYTES.to_vec() }
    }

    fn store_is_free_for_a_stage(store: &RegistryStore) -> bool {
        let probe = StagedSlice { key: SliceKey::PollerSettings, bytes: Vec::new() };
        let free = store.stage_import(OTHER_WORKFLOW, vec![probe]).is_ok();
        store.discard_import(OTHER_WORKFLOW);
        free
    }

    #[test]
    fn a_pulled_set_holding_a_name_accepts_would_refuse_stages_nothing() {
        let slices: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        let (sink, store) = sink_over(Some(slices));
        assert!(sink.accepts(KNOWN_SLICE));
        assert!(!sink.accepts(UNKNOWN_SLICE));

        assert!(!sink.stage(vec![pulled(KNOWN_SLICE), pulled(UNKNOWN_SLICE)]));

        assert!(store_is_free_for_a_stage(&store), "the known slice was not staged beside the unknown one");
    }

    #[test]
    fn what_accepts_approves_is_staged_and_holds_the_stage_until_discarded() {
        let slices: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        let (sink, store) = sink_over(Some(slices));

        assert!(sink.stage(vec![pulled(KNOWN_SLICE), pulled(OTHER_KNOWN_SLICE)]));
        assert!(!store_is_free_for_a_stage(&store));
        sink.discard_staged();
        assert!(store_is_free_for_a_stage(&store));
    }

    #[test]
    fn a_sink_without_slice_storage_accepts_and_stages_nothing() {
        let (sink, store) = sink_over(None);
        assert!(!sink.accepts(KNOWN_SLICE));
        assert!(sink.local_digests().is_empty());

        assert!(!sink.stage(vec![pulled(KNOWN_SLICE)]));

        assert!(store_is_free_for_a_stage(&store));
    }

    #[test]
    fn an_input_device_summary_keeps_each_field_in_its_own_place() {
        let dto = super::summary_dto(InputDeviceSummary {
            adapter_id: 1,
            device_capabilities: Some(2),
            device_status: Some(3),
            version_number: Some(4),
            short_address: 5,
            name: Some("hall".to_string()),
            present: true,
            instance_count: 6,
            first_instance_type: Some(7),
            ha_expose: false,
            last_seen_ms: Some(8),
            last_event_at_ms: Some(9),
        });
        assert_eq!((dto.adapter_id, dto.short_address, dto.instance_count), (1, 5, 6));
        assert_eq!((dto.present, dto.ha_expose, dto.first_instance_type), (true, false, Some(7)));
        assert_eq!((dto.last_seen_ms, dto.last_event_at_ms), (Some(8), Some(9)));
        assert_eq!(dto.name.as_deref(), Some("hall"));
    }

    #[test]
    fn the_replication_digests_are_the_manifest_the_transfer_port_serves() {
        let backing = Arc::new(InMemorySliceStore::new());
        write_slice(&*backing, SliceKey::PollerSettings, &SLICE_BYTES);
        let slices: Arc<dyn SliceStore> = backing;
        let (sink, store) = sink_over(Some(Arc::clone(&slices)));
        let transfer = ConfigTransferBridge::new(store, Some(slices), ADAPTERS);

        let digests = sink.local_digests();
        let served = transfer.slice_manifest();
        let digest_of = |name: &str| digests.iter().find(|d| d.name == name).map(|d| (d.bytes, d.crc32));
        let served_of = |name: &str| served.iter().find(|row| row.name == name).map(|row| (row.bytes, row.crc32));

        let written = Some((Some(SLICE_BYTES.len()), SLICE_BYTES_CRC32));
        assert_eq!(served_of(KNOWN_SLICE), written, "the port serves the written length and its CRC-32");
        assert_eq!(digest_of(KNOWN_SLICE), written, "the digest names the same written slice");
        let never_written = Some((None, 0));
        assert_eq!(served_of(OTHER_KNOWN_SLICE), never_written, "a slice never written has no bytes");
        assert_eq!(digest_of(OTHER_KNOWN_SLICE), never_written, "and no digest of bytes");
        assert_eq!((served_of(UNKNOWN_SLICE), digest_of(UNKNOWN_SLICE)), (None, None));
    }

    #[test]
    fn a_held_broadcast_and_group_zero_reach_the_rules_world_as_different_targets() {
        let key = |scope, group_id| dali2rust_hcl_runtime::TargetKey { adapter_id: 1, scope, group_id };
        assert_eq!(
            super::rules_hcl_target(key(HclTargetScope::Broadcast, 0)),
            LightTarget::Broadcast { adapter_id: 1 }
        );
        assert_eq!(
            super::rules_hcl_target(key(HclTargetScope::Group, 0)),
            LightTarget::Group(GroupRef { adapter_id: 1, id: 0 })
        );
        assert_eq!(
            super::rules_hcl_target(key(HclTargetScope::Group, 9)),
            LightTarget::Group(GroupRef { adapter_id: 1, id: 9 })
        );
    }
}
