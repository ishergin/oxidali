use std::sync::{Arc, Mutex};

use dali2rust_api::confirmation_bridge::{spawn_confirmation_bridge, PendingConfirmationSlots};
use dali2rust_bus::{BusId, BusPublisher, BusSubscriberRx};
use dali2rust_display_runtime::{spawn_display_worker, DisplayView, HardwareDisplay};
use dali2rust_domain::registry::{InputInstanceTypeReadPort, ProjectorReadPort, RegistryReadPort};
use dali2rust_fanout_runtime::{
    spawn_projector_worker, spawn_sniffer_translator_worker, ProjectorCounters,
    SnifferTranslatorCounters,
};
use dali2rust_operations_runtime::{
    spawn_apply_orchestrator_worker, spawn_operation_tracker_worker, ApplyOrchestratorCounters,
    OperationTrackerCounters, OperationTrackerHttpRead, OperationTrackerInner,
};
use dali2rust_platform::dali::{DaliTransport, SnifferTap};
use dali2rust_platform::slice_store::SliceStore;
use dali2rust_registry_runtime::{spawn_registry_worker, RegistryStore, RegistryWorkerCounters};

use super::http_bridges::TransferSeams;

const SNIFFER_CHANNEL_CAPACITY: usize = 32;

pub(crate) const SNIFFER_WINDOW_CAPACITY: usize = 256;

const SNIFFER_REGISTRY_ADAPTER_ID: u8 = 0;

use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
};

pub(crate) struct SpawnedWorkers {
    pub dali: std::thread::JoinHandle<()>,
    pub registry: std::thread::JoinHandle<()>,
    pub operation_tracker_worker: std::thread::JoinHandle<()>,
    pub operation_tracker: Arc<Mutex<OperationTrackerInner>>,
    pub apply_orchestrator: std::thread::JoinHandle<()>,
    pub confirmation_bridge: std::thread::JoinHandle<()>,
    pub display: std::thread::JoinHandle<()>,
    pub projector: std::thread::JoinHandle<()>,
    pub sniffer_translator: std::thread::JoinHandle<()>,
    pub hcl_scheduler: std::thread::JoinHandle<()>,
    pub hcl_overrides: dali2rust_hcl_runtime::SharedOverrideLedger,
    pub poller: std::thread::JoinHandle<()>,
    pub rules: std::thread::JoinHandle<()>,
    pub arbitration_supervisor: std::thread::JoinHandle<()>,
    pub arbitration: std::thread::JoinHandle<()>,
    pub replication: std::thread::JoinHandle<()>,
    pub arbitration_transitions: dali2rust_redundancy_runtime::SharedTransitionLog,
    pub mqtt: Option<std::thread::JoinHandle<()>>,
    pub ws_worker: Option<std::thread::JoinHandle<()>>,
    pub ota: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone)]
pub(crate) struct RuntimeCounterHandles {
    pub dali_worker: Arc<DaliWorkerCounters>,
    pub operation_tracker: Arc<OperationTrackerCounters>,
    pub apply_orchestrator: Arc<ApplyOrchestratorCounters>,
    pub projector: Arc<ProjectorCounters>,
    pub registry: Arc<RegistryWorkerCounters>,
    pub sniffer_translator: Arc<SnifferTranslatorCounters>,
    pub phy_sniffer: Arc<dali2rust_platform::dali::PhySnifferCounters>,
    pub dali_wire: Arc<dali2rust_platform::dali::DaliWireCounters>,
    pub hcl_scheduler: Arc<dali2rust_hcl_runtime::HclSchedulerCounters>,
    pub poller: Arc<dali2rust_poller_runtime::PollerCounters>,
    pub websocket: Arc<dali2rust_ws_runtime::WsCounters>,
    pub mqtt: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    pub rules: Arc<dali2rust_rules_runtime::RulesWorkerCounters>,
    pub rules_engine: Arc<dali2rust_rules_runtime::RulesEngineCells>,
    pub arbitration_supervisor:
        Arc<dali2rust_redundancy_runtime::ArbitrationSupervisorCounters>,
    pub arbitration_reflex: Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
    pub arbitration_worker: Arc<dali2rust_redundancy_runtime::ArbitrationWorkerCounters>,
    pub replication: Arc<dali2rust_redundancy_runtime::ReplicationCounters>,
}

pub(crate) struct PipelineChannels {
    pub dali_cmd: BusSubscriberRx,
    pub registry: BusSubscriberRx,
}

pub(crate) struct ServiceChannels {
    pub operation_cmd: BusSubscriberRx,
    pub operation_conf: BusSubscriberRx,
    pub apply_cmd: BusSubscriberRx,
    pub confirmations: BusSubscriberRx,
    pub display_ev: BusSubscriberRx,
    pub operation_ev: BusSubscriberRx,
    pub apply_ev: BusSubscriberRx,
    pub projector_ev: BusSubscriberRx,
    pub periodic: PeriodicChannels,
    pub ota_cmd: Option<BusSubscriberRx>,
    pub supervisor_ev: Option<BusSubscriberRx>,
    pub mqtt_ev: Option<BusSubscriberRx>,
}

pub(crate) struct PeriodicChannels {
    pub hcl_rx: BusSubscriberRx,
    pub hcl_conf: BusSubscriberRx,
    pub poller_ev: BusSubscriberRx,
    pub poller_conf: BusSubscriberRx,
    pub rules_cmd: BusSubscriberRx,
    pub arbitration_ev: BusSubscriberRx,
    pub replication_ev: BusSubscriberRx,
}

pub(crate) struct WorkerChannels {
    pub pipeline: PipelineChannels,
    pub services: ServiceChannels,
    pub ws_ev: WsInboxPending,
}

pub(crate) struct WsInbox {
    pub rx: BusSubscriberRx,
    pub probe: dali2rust_bus::EventInboxProbe,
}

pub(crate) struct WsInboxPending {
    pub rx: BusSubscriberRx,
    pub index: usize,
}

impl From<(BusSubscriberRx, usize)> for WsInboxPending {
    fn from((rx, index): (BusSubscriberRx, usize)) -> Self {
        Self { rx, index }
    }
}

impl WsInboxPending {
    pub fn attach(self, publisher: &BusPublisher) -> WsInbox {
        WsInbox {
            rx: self.rx,
            probe: publisher.event_inbox_probe(self.index),
        }
    }
}

pub(crate) struct BusWiring {
    pub bus_id: BusId,
    pub publisher: BusPublisher,
    pub slots: Arc<PendingConfirmationSlots>,
}

#[derive(Clone)]
pub(crate) struct RegistryDeps {
    pub read_port: Arc<dyn RegistryReadPort>,
    pub store: Arc<RegistryStore>,
    pub counters: Arc<RegistryWorkerCounters>,
    pub adapter_count: u8,
    pub persistence_slices: Option<Arc<dyn SliceStore>>,
}

struct PipelineWorkers {
    dali: std::thread::JoinHandle<()>,
    registry: std::thread::JoinHandle<()>,
    dali_counters: Arc<DaliWorkerCounters>,
    registry_counters: Arc<RegistryWorkerCounters>,
}

#[derive(Clone)]
pub(crate) struct HclSchedulerDeps {
    pub read_port: Arc<dyn dali2rust_domain::registry::HclSchedulerReadPort>,
    pub clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    pub correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    pub config: dali2rust_hcl_runtime::HclConfig,
    pub hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
    pub role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
}

pub(crate) struct DisplaySeams {
    pub wire: Arc<dali2rust_platform::dali::DaliWireCounters>,
    pub phy_sniffer: Arc<dali2rust_platform::dali::PhySnifferCounters>,
    pub ws: Arc<dali2rust_ws_runtime::WsCounters>,
    pub mqtt: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    pub clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
}

#[derive(Clone)]
pub(crate) struct PollerWorkerDeps {
    pub read_port: Arc<dyn dali2rust_domain::registry::PollerReadPort>,
    pub correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    pub hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
    pub role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
}

pub(crate) struct RulesWorkerDeps {
    pub store: Arc<dali2rust_rules_runtime::RulesStore>,
    pub compiler: Arc<dyn dali2rust_rules_model::RuleCompiler>,
    pub resolver: Arc<dyn dali2rust_rules_model::NameResolver>,
    pub slices: Option<Arc<dyn SliceStore>>,
    pub registry: Arc<RegistryStore>,
    pub wall: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    pub hcl_state: Arc<dyn dali2rust_api::http::hcl_state::HclScheduleHttpState>,
}

#[allow(clippy::too_many_arguments, reason = "one call site; each argument is a distinct dependency")]
pub(crate) struct RedundancyBeats {
    pub(crate) registry: Arc<dali2rust_platform::liveness::LivenessBeat>,
    pub(crate) hcl: Arc<dali2rust_platform::liveness::LivenessBeat>,
    pub(crate) rules: Arc<dali2rust_platform::liveness::LivenessBeat>,
}

impl RedundancyBeats {
    pub(crate) fn new() -> Self {
        let stale = dali2rust_redundancy_runtime::WORKER_STALE_AFTER_MS;
        Self {
            registry: Arc::new(dali2rust_platform::liveness::LivenessBeat::new(
                "registry", stale,
            )),
            hcl: Arc::new(dali2rust_platform::liveness::LivenessBeat::new("hcl", stale)),
            rules: Arc::new(dali2rust_platform::liveness::LivenessBeat::new(
                "rules", stale,
            )),
        }
    }

    pub(crate) fn watch(&self) -> dali2rust_platform::liveness::LivenessWatch {
        let mut watch = dali2rust_platform::liveness::LivenessWatch::new();
        watch.register(Arc::clone(&self.registry));
        watch.register(Arc::clone(&self.hcl));
        watch.register(Arc::clone(&self.rules));
        watch
    }
}

fn spawn_pipeline_workers<T: DaliTransport + Send + 'static>(
    transport: Arc<Mutex<T>>,
    runtime_config: DaliRuntimeConfig,
    registry: RegistryDeps,
    channels: PipelineChannels,
    publisher: &BusPublisher,
    bus_id: BusId,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
    sniffer_tap: Arc<SnifferTap>,
    wire_counters: Arc<dali2rust_platform::dali::DaliWireCounters>,
    beats: &RedundancyBeats,
    correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
) -> PipelineWorkers {
    let clock = Box::new(StdClock::new());
    let mut controller = DaliController::with_retry_policy(transport, clock, runtime_config.retry);
    controller.set_sniffer_tap(sniffer_tap, SNIFFER_REGISTRY_ADAPTER_ID);
    controller.set_wire_counters(Arc::clone(&wire_counters));
    let dali_counters = Arc::new(DaliWorkerCounters::default());
    let dali = spawn_dali_worker(
        channels.dali_cmd,
        controller,
        runtime_config,
        registry.read_port,
        publisher.clone(),
        bus_id,
        Arc::clone(&dali_counters),
        interactive,
        correlation,
    );
    let registry_counters_handle = Arc::clone(&registry.counters);
    let registry = spawn_registry_worker(
        channels.registry,
        publisher.clone(),
        bus_id,
        registry.adapter_count,
        Arc::clone(&registry.store),
        Arc::clone(&registry.counters),
        registry.persistence_slices,
        Arc::clone(&beats.registry),
    );
    PipelineWorkers {
        dali,
        registry,
        dali_counters,
        registry_counters: registry_counters_handle,
    }
}

fn spawn_fanout_workers(
    projector_ev: BusSubscriberRx,
    publisher: &BusPublisher,
    bus_id: BusId,
    store: &Arc<RegistryStore>,
    observed_rx: std::sync::mpsc::Receiver<dali2rust_platform::dali::ObservedRawFrame>,
    sniffer_tap: Arc<SnifferTap>,
) -> FanoutWorkers {
    let projector_counters = Arc::new(ProjectorCounters::default());
    let projector = spawn_projector_worker(
        projector_ev,
        publisher.clone(),
        bus_id,
        Arc::clone(store) as Arc<dyn ProjectorReadPort>,
        Arc::clone(&projector_counters),
    );
    let sniffer_translator_counters = Arc::new(SnifferTranslatorCounters::default());
    let sniffer_translator = spawn_sniffer_translator_worker(
        observed_rx,
        publisher.clone(),
        bus_id,
        SNIFFER_REGISTRY_ADAPTER_ID,
        Arc::clone(&sniffer_translator_counters),
        Some(sniffer_tap),
        Arc::clone(store) as Arc<dyn InputInstanceTypeReadPort>,
    );
    FanoutWorkers {
        projector,
        sniffer_translator,
        projector_counters,
        sniffer_translator_counters,
    }
}

struct FanoutWorkers {
    projector: std::thread::JoinHandle<()>,
    sniffer_translator: std::thread::JoinHandle<()>,
    projector_counters: Arc<ProjectorCounters>,
    sniffer_translator_counters: Arc<SnifferTranslatorCounters>,
}

struct OperationsWorkers {
    tracker: std::thread::JoinHandle<()>,
    tracker_inner: Arc<Mutex<OperationTrackerInner>>,
    tracker_counters: Arc<OperationTrackerCounters>,
    apply_orchestrator: std::thread::JoinHandle<()>,
    orchestrator_counters: Arc<ApplyOrchestratorCounters>,
}

fn spawn_operations_workers(
    operation_cmd: BusSubscriberRx,
    operation_ev: BusSubscriberRx,
    operation_conf: BusSubscriberRx,
    apply_cmd: BusSubscriberRx,
    apply_ev: BusSubscriberRx,
    publisher: &BusPublisher,
    bus_id: BusId,
    orchestrator_store: Arc<RegistryStore>,
) -> OperationsWorkers {
    let tracker_inner = Arc::new(Mutex::new(OperationTrackerInner::new()));
    let tracker_counters = Arc::new(OperationTrackerCounters::default());
    let tracker = spawn_operation_tracker_worker(
        operation_cmd,
        operation_ev,
        operation_conf,
        publisher.clone(),
        bus_id,
        Arc::clone(&tracker_inner),
        Arc::clone(&tracker_counters),
    );
    let orchestrator_counters = Arc::new(ApplyOrchestratorCounters::default());
    let apply_orchestrator = spawn_apply_orchestrator_worker(
        apply_cmd,
        apply_ev,
        publisher.clone(),
        bus_id,
        orchestrator_store,
        Arc::new(OperationTrackerHttpRead(Arc::clone(&tracker_inner))),
        Arc::clone(&orchestrator_counters),
    );
    OperationsWorkers {
        tracker,
        tracker_inner,
        tracker_counters,
        apply_orchestrator,
        orchestrator_counters,
    }
}

struct HclWorker {
    handle: std::thread::JoinHandle<()>,
    counters: Arc<dali2rust_hcl_runtime::HclSchedulerCounters>,
    overrides: dali2rust_hcl_runtime::SharedOverrideLedger,
}

fn spawn_hcl_worker(
    rx: BusSubscriberRx,
    conf_rx: BusSubscriberRx,
    publisher: &BusPublisher,
    bus_id: BusId,
    hcl: HclSchedulerDeps,
    beats: &RedundancyBeats,
) -> HclWorker {
    let counters = Arc::new(dali2rust_hcl_runtime::HclSchedulerCounters::default());
    let overrides: dali2rust_hcl_runtime::SharedOverrideLedger = Arc::default();
    let handle = dali2rust_hcl_runtime::spawn_hcl_scheduler_worker(
        rx,
        conf_rx,
        publisher.clone(),
        bus_id,
        hcl.read_port,
        hcl.clock,
        hcl.correlation,
        hcl.config,
        Arc::clone(&counters),
        Arc::clone(&overrides),
        Arc::clone(&beats.hcl),
        Arc::clone(&hcl.role),
        hcl.hold,
    );
    HclWorker {
        handle,
        counters,
        overrides,
    }
}

pub(crate) struct MqttWorkerDeps {
    pub publisher: BusPublisher,
    pub correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    pub client: Box<dyn dali2rust_platform::mqtt::MqttClient>,
    pub incoming: std::sync::mpsc::Receiver<dali2rust_platform::mqtt::MqttIncoming>,
    pub link: Arc<dali2rust_platform::mqtt::MqttLink>,
    pub read_port: Arc<dyn dali2rust_domain::registry::HaPublishReadPort>,
    pub settings: Arc<dyn dali2rust_domain::registry::HomeAssistantSettingsReadPort>,
    pub settings_watch: Arc<dyn dali2rust_domain::registry::HomeAssistantSettingsApplyWatchPort>,
    pub role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    pub secret: Arc<dyn dali2rust_domain::registry::HomeAssistantSecretReadPort>,
    pub version: &'static str,
    pub adapter_count: u8,
}

fn spawn_mqtt_worker_thread(
    ev_rx: BusSubscriberRx,
    bus_id: BusId,
    deps: MqttWorkerDeps,
    counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
) -> std::thread::JoinHandle<()> {
    dali2rust_mqtt_runtime::spawn_mqtt_worker(
        ev_rx,
        deps.client,
        deps.incoming,
        deps.link,
        dali2rust_mqtt_runtime::MqttWorkerPorts {
            publisher: deps.publisher,
            bus_id,
            correlation: deps.correlation,
            read_port: deps.read_port,
            settings: deps.settings,
            settings_watch: deps.settings_watch,
            secret: deps.secret,
            counters,
            version: deps.version,
            adapter_count: deps.adapter_count,
            role: deps.role,
        },
    )
}

fn spawn_poller_worker_thread(
    ev_rx: BusSubscriberRx,
    conf_rx: BusSubscriberRx,
    publisher: &BusPublisher,
    bus_id: BusId,
    adapter_count: u8,
    poller: PollerWorkerDeps,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
) -> (
    std::thread::JoinHandle<()>,
    Arc<dali2rust_poller_runtime::PollerCounters>,
) {
    let counters = Arc::new(dali2rust_poller_runtime::PollerCounters::default());
    let handle = dali2rust_poller_runtime::spawn_poller_worker(
        ev_rx,
        conf_rx,
        publisher.clone(),
        poller.read_port,
        poller.correlation,
        bus_id,
        adapter_count,
        Arc::clone(&counters),
        interactive,
        poller.role,
        poller.hold,
    );
    (handle, counters)
}

struct PeriodicWorkers {
    hcl: HclWorker,
    poller: std::thread::JoinHandle<()>,
    poller_counters: Arc<dali2rust_poller_runtime::PollerCounters>,
    rules: std::thread::JoinHandle<()>,
    rules_counters: Arc<dali2rust_rules_runtime::RulesWorkerCounters>,
    rules_engine_cells: Arc<dali2rust_rules_runtime::RulesEngineCells>,
    arbitration: ArbitrationWorker,
    replication: ReplicationWorker,
}

pub(crate) struct ReplicationWorker {
    pub handle: std::thread::JoinHandle<()>,
    pub counters: Arc<dali2rust_redundancy_runtime::ReplicationCounters>,
}

pub(crate) struct ReplicationDepsIn {
    pub fetch: Arc<dyn dali2rust_platform::http_fetch::HttpFetch>,
    pub sink: Arc<dyn dali2rust_redundancy_runtime::ReplicationSink>,
    pub redundancy: Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
    pub dali: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
}

fn spawn_replication(
    ev_rx: BusSubscriberRx,
    publisher: &BusPublisher,
    bus_id: BusId,
    deps: ReplicationDepsIn,
) -> ReplicationWorker {
    let counters = Arc::new(dali2rust_redundancy_runtime::ReplicationCounters::default());
    let handle = dali2rust_redundancy_runtime::spawn_replication_worker(
        ev_rx,
        dali2rust_redundancy_runtime::ReplicationDeps {
            publisher: publisher.clone(),
            bus_id,
            fetch: deps.fetch,
            sink: deps.sink,
            redundancy: deps.redundancy,
            dali: deps.dali,
            counters: Arc::clone(&counters),
        },
    );
    ReplicationWorker { handle, counters }
}

pub(crate) struct RedundancyDepsIn {
    pub arbitration: ArbitrationDeps,
    pub replication: ReplicationDepsIn,
}

fn redundancy_deps(store: &Arc<RegistryStore>, transfer: &TransferSeams) -> RedundancyDepsIn {
    RedundancyDepsIn {
        arbitration: arbitration_deps(store),
        replication: replication_deps(store, transfer),
    }
}

fn replication_deps(store: &Arc<RegistryStore>, transfer: &TransferSeams) -> ReplicationDepsIn {
    ReplicationDepsIn {
        fetch: Arc::new(dali2rust_platform::http_fetch::TcpHttpFetch),
        sink: Arc::new(crate::runtime::http_bridges::ReplicationSinkBridge::new(
            Arc::clone(store),
            transfer.slices.clone(),
            transfer.adapter_count,
            Arc::clone(&transfer.wall_clock),
        )),
        redundancy: Arc::clone(store)
            as Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
        dali: Arc::clone(store) as Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    }
}

fn arbitration_deps(store: &Arc<RegistryStore>) -> ArbitrationDeps {
    ArbitrationDeps {
        registry_adapter_id: SNIFFER_REGISTRY_ADAPTER_ID,
        redundancy: Arc::clone(store)
            as Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
        dali: Arc::clone(store) as Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
        adapters: Arc::clone(store) as Arc<dyn dali2rust_domain::registry::AdapterEnabledReadPort>,
    }
}

pub(crate) struct ArbitrationWorker {
    pub handle: std::thread::JoinHandle<()>,
    pub counters: Arc<dali2rust_redundancy_runtime::ArbitrationWorkerCounters>,
    pub transitions: dali2rust_redundancy_runtime::SharedTransitionLog,
}

pub(crate) struct ArbitrationDeps {
    pub registry_adapter_id: u8,
    pub redundancy: Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
    pub dali: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    pub adapters: Arc<dyn dali2rust_domain::registry::AdapterEnabledReadPort>,
}

fn spawn_arbitration(
    ev_rx: BusSubscriberRx,
    publisher: &BusPublisher,
    bus_id: BusId,
    deps: ArbitrationDeps,
) -> ArbitrationWorker {
    let counters = Arc::new(dali2rust_redundancy_runtime::ArbitrationWorkerCounters::default());
    let transitions: dali2rust_redundancy_runtime::SharedTransitionLog = Arc::default();
    let handle = dali2rust_redundancy_runtime::spawn_arbitration_worker(
        ev_rx,
        dali2rust_redundancy_runtime::ArbitrationWorkerDeps {
            publisher: publisher.clone(),
            bus_id,
            registry_adapter_id: deps.registry_adapter_id,
            redundancy: deps.redundancy,
            dali: deps.dali,
            adapters: deps.adapters,
            counters: Arc::clone(&counters),
            transitions: Arc::clone(&transitions),
        },
    );
    ArbitrationWorker {
        handle,
        counters,
        transitions,
    }
}

struct RulesWorker {
    rules: std::thread::JoinHandle<()>,
    rules_counters: Arc<dali2rust_rules_runtime::RulesWorkerCounters>,
    rules_engine_cells: Arc<dali2rust_rules_runtime::RulesEngineCells>,
}

fn spawn_rules(
    rules_cmd: BusSubscriberRx,
    publisher: &BusPublisher,
    bus_id: BusId,
    rules_deps: RulesWorkerDeps,
    hcl: &HclWorker,
    beats: &RedundancyBeats,
) -> RulesWorker {
    let rules_counters = Arc::new(dali2rust_rules_runtime::RulesWorkerCounters::default());
    let rules_engine_cells = Arc::new(dali2rust_rules_runtime::RulesEngineCells::default());
    let world = Arc::new(crate::runtime::http_bridges::RulesWorldBridge::new(
        Arc::clone(&rules_deps.registry),
        Arc::clone(&rules_deps.wall),
        Arc::clone(&rules_deps.hcl_state),
        Arc::clone(&hcl.overrides),
    ));
    let rules = dali2rust_rules_runtime::spawn_rules_worker(
        rules_cmd,
        publisher.clone(),
        bus_id,
        dali2rust_rules_runtime::RulesWorkerSeams {
            store: rules_deps.store,
            compiler: rules_deps.compiler,
            resolver: rules_deps.resolver,
            slices: rules_deps.slices,
            world,
        },
        Arc::clone(&rules_counters),
        Arc::clone(&rules_engine_cells),
        Arc::clone(&beats.rules),
    );
    RulesWorker {
        rules,
        rules_counters,
        rules_engine_cells,
    }
}

#[allow(clippy::too_many_arguments, reason = "one call site; every arg is a distinct inbox or dep bundle")]
fn spawn_periodic_workers(
    channels: PeriodicChannels,
    publisher: &BusPublisher,
    bus_id: BusId,
    adapter_count: u8,
    inputs: PeriodicInputs<'_>,
    redundancy: RedundancyDepsIn,
) -> PeriodicWorkers {
    let PeriodicInputs {
        hcl,
        poller,
        rules: rules_deps,
        interactive,
        beats,
    } = inputs;
    let hcl = spawn_hcl_worker(channels.hcl_rx, channels.hcl_conf, publisher, bus_id, hcl, beats);
    let (poller, poller_counters) = spawn_poller_worker_thread(
        channels.poller_ev,
        channels.poller_conf,
        publisher,
        bus_id,
        adapter_count,
        poller,
        interactive,
    );
    let RulesWorker {
        rules,
        rules_counters,
        rules_engine_cells,
    } = spawn_rules(channels.rules_cmd, publisher, bus_id, rules_deps, &hcl, beats);
    let arbitration =
        spawn_arbitration(channels.arbitration_ev, publisher, bus_id, redundancy.arbitration);
    let replication =
        spawn_replication(channels.replication_ev, publisher, bus_id, redundancy.replication);
    PeriodicWorkers {
        arbitration,
        replication,
        hcl,
        poller,
        poller_counters,
        rules,
        rules_counters,
        rules_engine_cells,
    }
}

struct BridgeAndDisplay {
    confirmation_bridge: std::thread::JoinHandle<()>,
    display: std::thread::JoinHandle<()>,
}

fn spawn_bridge_and_display(
    confirmations: BusSubscriberRx,
    display_ev: BusSubscriberRx,
    slots: Arc<PendingConfirmationSlots>,
    hardware_display: HardwareDisplay,
    source: Arc<dyn dali2rust_display_runtime::source::DisplaySource>,
) -> BridgeAndDisplay {
    let confirmation_bridge = spawn_confirmation_bridge(confirmations, slots);
    let display = spawn_display_worker(
        display_ev,
        source,
        Arc::new(DisplayView::default()),
        hardware_display,
    );
    BridgeAndDisplay {
        confirmation_bridge,
        display,
    }
}

#[allow(clippy::too_many_arguments, reason = "one call site; each group is already a struct")]
fn spawn_service_workers(
    channels: ServiceChannels,
    publisher: BusPublisher,
    bus_id: BusId,
    adapter_count: u8,
    orchestrator_store: Arc<RegistryStore>,
    slots: Arc<PendingConfirmationSlots>,
    hardware_display: HardwareDisplay,
    observed_rx: std::sync::mpsc::Receiver<dali2rust_platform::dali::ObservedRawFrame>,
    periodic_inputs: PeriodicInputs<'_>,
    sniffer_tap: Arc<SnifferTap>,
    display_seams: DisplaySeams,
    transfer: TransferSeams,
) -> (OperationsWorkers, FanoutWorkers, PeriodicWorkers, BridgeAndDisplay) {
    let operations = spawn_operations_workers(
        channels.operation_cmd,
        channels.operation_ev,
        channels.operation_conf,
        channels.apply_cmd,
        channels.apply_ev,
        &publisher,
        bus_id,
        Arc::clone(&orchestrator_store),
    );
    let fanout = spawn_fanout_workers(
        channels.projector_ev,
        &publisher,
        bus_id,
        &orchestrator_store,
        observed_rx,
        sniffer_tap,
    );
    let periodic = spawn_periodic_workers(
        channels.periodic,
        &publisher,
        bus_id,
        adapter_count,
        periodic_inputs,
        redundancy_deps(&orchestrator_store, &transfer),
    );
    let bridge = spawn_bridge_and_display(
        channels.confirmations,
        channels.display_ev,
        slots,
        hardware_display,
        display_source(&orchestrator_store, &periodic, display_seams),
    );
    (operations, fanout, periodic, bridge)
}

fn display_source(
    registry: &Arc<RegistryStore>,
    periodic: &PeriodicWorkers,
    seams: DisplaySeams,
) -> Arc<dyn dali2rust_display_runtime::source::DisplaySource> {
    Arc::new(super::display_source::ControllerDisplaySource {
        registry: Arc::clone(registry),
        wire: seams.wire,
        phy_sniffer: seams.phy_sniffer,
        poller: Arc::clone(&periodic.poller_counters),
        hcl_overrides: Arc::clone(&periodic.hcl.overrides),
        mqtt: seams.mqtt,
        ws: seams.ws,
        clock: seams.clock,
        adapter_id: SNIFFER_REGISTRY_ADAPTER_ID,
    })
}

pub(crate) struct WorkerSpawnInputs {
    pub runtime_config: DaliRuntimeConfig,
    pub wiring: BusWiring,
    pub registry: RegistryDeps,
    pub pipeline_channels: PipelineChannels,
    pub service_channels: ServiceChannels,
    pub hardware_display: HardwareDisplay,
    pub hcl: HclSchedulerDeps,
    pub poller: PollerWorkerDeps,
    pub interactive: Arc<dali2rust_platform::dali::WireActivity>,
    pub sniffer_tap: Arc<SnifferTap>,
    pub ws_counters: Arc<dali2rust_ws_runtime::WsCounters>,
    pub mqtt: Option<MqttWorkerDeps>,
    pub mqtt_counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    pub rules: RulesWorkerDeps,
    pub correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    pub ota: Option<OtaWorkerDeps>,
}

pub(crate) struct OtaWorkerDeps {
    pub ports: crate::ota::FirmwareUpdatePorts,
    pub state: Arc<dali2rust_ota_runtime::OtaState>,
    pub hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
}

pub(crate) fn spawn_bus_workers<T: DaliTransport + Send + 'static>(
    transport: Arc<Mutex<T>>,
    inputs: WorkerSpawnInputs,
) -> (SpawnedWorkers, RuntimeCounterHandles) {
    let wire = wire_seams(&transport);
    let beats = RedundancyBeats::new();
    let (pipeline_inputs, mut services) = inputs.split();
    let pipeline = spawn_pipeline_workers(
        transport,
        pipeline_inputs.runtime_config,
        pipeline_inputs.registry,
        pipeline_inputs.channels,
        &services.publisher,
        services.bus_id,
        Arc::clone(&services.interactive),
        Arc::clone(&services.sniffer_tap),
        Arc::clone(&wire.dali),
        &beats,
        pipeline_inputs.correlation,
    );
    let mqtt = spawn_mqtt_if_configured(
        services.channels.mqtt_ev.take(),
        services.mqtt.take(),
        services.bus_id,
        &services.mqtt_counters,
    );
    let ota = spawn_ota_if_configured(
        services.channels.ota_cmd.take(),
        services.ota.take(),
        &services.publisher,
        services.bus_id,
    );
    let (mut spawned, counters) = finish_spawn(services, wire, pipeline, mqtt, beats);
    spawned.ota = ota;
    (spawned, counters)
}

struct PipelineInputs {
    runtime_config: DaliRuntimeConfig,
    registry: RegistryDeps,
    channels: PipelineChannels,
    correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
}

struct ServiceInputs {
    bus_id: BusId,
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    adapter_count: u8,
    registry_store: Arc<RegistryStore>,
    channels: ServiceChannels,
    hardware_display: HardwareDisplay,
    hcl: HclSchedulerDeps,
    poller: PollerWorkerDeps,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
    sniffer_tap: Arc<SnifferTap>,
    mqtt: Option<MqttWorkerDeps>,
    mqtt_counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    ws_counters: Arc<dali2rust_ws_runtime::WsCounters>,
    rules: RulesWorkerDeps,
    ota: Option<OtaWorkerDeps>,
    transfer: TransferSeams,
}

impl WorkerSpawnInputs {
    fn split(self) -> (PipelineInputs, ServiceInputs) {
        let registry_store = Arc::clone(&self.registry.store);
        let adapter_count = self.registry.adapter_count;
        let transfer = TransferSeams {
            slices: self.registry.persistence_slices.clone(),
            adapter_count,
            wall_clock: Arc::clone(&self.hcl.clock),
        };
        (
            PipelineInputs {
                runtime_config: self.runtime_config,
                registry: self.registry,
                channels: self.pipeline_channels,
                correlation: self.correlation,
            },
            ServiceInputs {
                bus_id: self.wiring.bus_id,
                publisher: self.wiring.publisher,
                slots: self.wiring.slots,
                adapter_count,
                registry_store,
                channels: self.service_channels,
                hardware_display: self.hardware_display,
                hcl: self.hcl,
                poller: self.poller,
                interactive: self.interactive,
                sniffer_tap: self.sniffer_tap,
                mqtt: self.mqtt,
                mqtt_counters: self.mqtt_counters,
                ws_counters: self.ws_counters,
                rules: self.rules,
                ota: self.ota,
                transfer,
            },
        )
    }
}

struct PeriodicInputs<'a> {
    hcl: HclSchedulerDeps,
    poller: PollerWorkerDeps,
    rules: RulesWorkerDeps,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
    beats: &'a RedundancyBeats,
}

struct ArbitrationBringUp {
    supervisor: std::thread::JoinHandle<()>,
    counters: Arc<dali2rust_redundancy_runtime::ArbitrationSupervisorCounters>,
    reflex: Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
}

fn bring_up_arbitration(
    ev_rx: BusSubscriberRx,
    reflex: Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
    settings: Arc<RegistryStore>,
    beats: &RedundancyBeats,
) -> ArbitrationBringUp {
    let counters =
        Arc::new(dali2rust_redundancy_runtime::ArbitrationSupervisorCounters::default());
    let supervisor = dali2rust_redundancy_runtime::spawn_arbitration_supervisor(
        ev_rx,
        dali2rust_redundancy_runtime::SupervisorInputs {
            reflex: Arc::clone(&reflex),
            watch: beats.watch(),
            settings: settings as Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
            counters: Arc::clone(&counters),
            config: dali2rust_redundancy_runtime::ArbitrationSupervisorConfig::default(),
        },
    );
    ArbitrationBringUp {
        supervisor,
        counters,
        reflex,
    }
}

fn start_supervisor(
    inputs: &mut ServiceInputs,
    wire: &WireSeams,
    beats: &RedundancyBeats,
) -> ArbitrationBringUp {
    bring_up_arbitration(
        inputs
            .channels
            .supervisor_ev
            .take()
            .expect("supervisor inbox subscribed once"),
        Arc::clone(&wire.arbitration),
        Arc::clone(&inputs.registry_store),
        beats,
    )
}

fn display_seams_from(inputs: &ServiceInputs, wire: &WireSeams) -> DisplaySeams {
    DisplaySeams {
        wire: Arc::clone(&wire.dali),
        phy_sniffer: Arc::clone(&wire.phy_sniffer),
        ws: Arc::clone(&inputs.ws_counters),
        mqtt: Arc::clone(&inputs.mqtt_counters),
        clock: Arc::clone(&inputs.hcl.clock),
    }
}

fn finish_spawn(
    mut inputs: ServiceInputs,
    wire: WireSeams,
    pipeline: PipelineWorkers,
    mqtt: Option<std::thread::JoinHandle<()>>,
    beats: RedundancyBeats,
) -> (SpawnedWorkers, RuntimeCounterHandles) {
    let display_seams = display_seams_from(&inputs, &wire);
    let arbitration = start_supervisor(&mut inputs, &wire, &beats);
    let services = spawn_service_workers(
        inputs.channels,
        inputs.publisher,
        inputs.bus_id,
        inputs.adapter_count,
        inputs.registry_store,
        inputs.slots,
        inputs.hardware_display,
        wire.observed_rx,
        PeriodicInputs {
            hcl: inputs.hcl,
            poller: inputs.poller,
            rules: inputs.rules,
            interactive: inputs.interactive,
            beats: &beats,
        },
        inputs.sniffer_tap,
        display_seams,
        inputs.transfer,
    );
    collect_spawned(
        pipeline,
        services,
        (wire.phy_sniffer, wire.dali),
        arbitration,
        inputs.ws_counters,
        mqtt,
        inputs.mqtt_counters,
    )
}

fn spawn_ota_if_configured(
    rx: Option<BusSubscriberRx>,
    deps: Option<OtaWorkerDeps>,
    publisher: &BusPublisher,
    bus_id: BusId,
) -> Option<std::thread::JoinHandle<()>> {
    let (rx, deps) = (rx?, deps?);
    crate::runtime::registry_init::log_internal_free("before ota worker");
    let handle = Some(dali2rust_ota_runtime::spawn_ota_worker(
        rx,
        publisher.clone(),
        bus_id,
        dali2rust_ota_runtime::OtaWorkerSeams {
            port: deps.ports.port,
            source: deps.ports.source,
            state: deps.state,
            hold: deps.hold,
        },
    ));
    crate::runtime::registry_init::log_internal_free("after ota worker");
    handle
}

fn spawn_mqtt_if_configured(
    ev_rx: Option<BusSubscriberRx>,
    deps: Option<MqttWorkerDeps>,
    bus_id: BusId,
    counters: &Arc<dali2rust_mqtt_runtime::MqttCounters>,
) -> Option<std::thread::JoinHandle<()>> {
    ev_rx
        .zip(deps)
        .map(|(ev_rx, deps)| spawn_mqtt_worker_thread(ev_rx, bus_id, deps, Arc::clone(counters)))
}

struct WireSeams {
    arbitration: Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
    observed_rx: std::sync::mpsc::Receiver<dali2rust_platform::dali::ObservedRawFrame>,
    phy_sniffer: Arc<dali2rust_platform::dali::PhySnifferCounters>,
    dali: Arc<dali2rust_platform::dali::DaliWireCounters>,
}

fn wire_seams<T: DaliTransport + Send + 'static>(transport: &Arc<Mutex<T>>) -> WireSeams {
    let (observed_rx, phy_sniffer) = wire_sniffer_seam(transport);
    let dali = Arc::new(dali2rust_platform::dali::DaliWireCounters::default());
    let arbitration = Arc::new(dali2rust_platform::arbitration::ArbitrationReflex::new());
    {
        let mut t = transport.lock().expect("transport lock");
        t.set_wire_counters(Arc::clone(&dali));
        t.set_arbitration_reflex(Arc::clone(&arbitration));
    }
    WireSeams {
        observed_rx,
        phy_sniffer,
        dali,
        arbitration,
    }
}

fn wire_sniffer_seam<T: DaliTransport + Send + 'static>(
    transport: &Arc<Mutex<T>>,
) -> (
    std::sync::mpsc::Receiver<dali2rust_platform::dali::ObservedRawFrame>,
    Arc<dali2rust_platform::dali::PhySnifferCounters>,
) {
    let (observed_tx, observed_rx) = std::sync::mpsc::sync_channel(SNIFFER_CHANNEL_CAPACITY);
    let phy_sniffer = Arc::new(dali2rust_platform::dali::PhySnifferCounters::default());
    {
        let mut t = transport.lock().expect("transport lock");
        t.set_observed_frame_sender(observed_tx);
        t.set_sniffer_counters(Arc::clone(&phy_sniffer));
    }
    (observed_rx, phy_sniffer)
}

fn collect_spawned(
    pipeline: PipelineWorkers,
    services: (OperationsWorkers, FanoutWorkers, PeriodicWorkers, BridgeAndDisplay),
    wire: (
        Arc<dali2rust_platform::dali::PhySnifferCounters>,
        Arc<dali2rust_platform::dali::DaliWireCounters>,
    ),
    arbitration: ArbitrationBringUp,
    websocket: Arc<dali2rust_ws_runtime::WsCounters>,
    mqtt: Option<std::thread::JoinHandle<()>>,
    mqtt_counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
) -> (SpawnedWorkers, RuntimeCounterHandles) {
    let (operations, fanout, periodic, bridge) = services;
    let PeriodicWorkers {
        hcl,
        poller,
        poller_counters,
        rules,
        rules_counters,
        rules_engine_cells,
        arbitration: arb_worker,
        replication: repl_worker,
    } = periodic;
    let periodic = periodic_counters(
        hcl.counters,
        poller_counters,
        rules_counters,
        rules_engine_cells,
        (arb_worker.counters, repl_worker.counters),
    );
    let counters = gather_counters(
        (pipeline.dali_counters, pipeline.registry_counters),
        (operations.tracker_counters, operations.orchestrator_counters),
        (fanout.projector_counters, fanout.sniffer_translator_counters),
        periodic,
        wire,
        (arbitration.counters, arbitration.reflex),
        (websocket, mqtt_counters),
    );
    let handles = (
        hcl.handle, hcl.overrides, poller, rules, arb_worker.handle, arb_worker.transitions,
    );
    let workers = gather_handles(
        (pipeline.dali, pipeline.registry),
        (operations.tracker, operations.tracker_inner, operations.apply_orchestrator),
        (fanout.projector, fanout.sniffer_translator),
        handles,
        bridge,
        (arbitration.supervisor, mqtt, repl_worker.handle),
    );
    (workers, counters)
}

fn gather_handles(
    pipeline: (std::thread::JoinHandle<()>, std::thread::JoinHandle<()>),
    operations: (
        std::thread::JoinHandle<()>,
        Arc<Mutex<OperationTrackerInner>>,
        std::thread::JoinHandle<()>,
    ),
    fanout: (std::thread::JoinHandle<()>, std::thread::JoinHandle<()>),
    periodic: (
        std::thread::JoinHandle<()>,
        dali2rust_hcl_runtime::SharedOverrideLedger,
        std::thread::JoinHandle<()>,
        std::thread::JoinHandle<()>,
        std::thread::JoinHandle<()>,
        dali2rust_redundancy_runtime::SharedTransitionLog,
    ),
    bridge: BridgeAndDisplay,
    solo: (
        std::thread::JoinHandle<()>,
        Option<std::thread::JoinHandle<()>>,
        std::thread::JoinHandle<()>,
    ),
) -> SpawnedWorkers {
    let (hcl_scheduler, hcl_overrides, poller, rules, arbitration, arbitration_transitions) =
        periodic;
    SpawnedWorkers {
        dali: pipeline.0,
        registry: pipeline.1,
        operation_tracker_worker: operations.0,
        operation_tracker: operations.1,
        apply_orchestrator: operations.2,
        confirmation_bridge: bridge.confirmation_bridge,
        display: bridge.display,
        projector: fanout.0,
        sniffer_translator: fanout.1,
        hcl_scheduler,
        hcl_overrides,
        poller,
        rules,
        arbitration,
        arbitration_transitions,
        arbitration_supervisor: solo.0,
        mqtt: solo.1,
        replication: solo.2,
        ws_worker: None,
        ota: None,
    }
}

#[allow(clippy::too_many_arguments, reason = "one call site; each is a distinct block")]
fn periodic_counters(
    hcl: Arc<dali2rust_hcl_runtime::HclSchedulerCounters>,
    poller: Arc<dali2rust_poller_runtime::PollerCounters>,
    rules: Arc<dali2rust_rules_runtime::RulesWorkerCounters>,
    rules_engine: Arc<dali2rust_rules_runtime::RulesEngineCells>,
    redundancy: (
        Arc<dali2rust_redundancy_runtime::ArbitrationWorkerCounters>,
        Arc<dali2rust_redundancy_runtime::ReplicationCounters>,
    ),
) -> PeriodicCounters {
    PeriodicCounters {
        hcl,
        poller,
        rules,
        rules_engine,
        arbitration: redundancy.0,
        replication: redundancy.1,
    }
}

fn gather_counters(
    pipeline: (Arc<DaliWorkerCounters>, Arc<RegistryWorkerCounters>),
    operations: (Arc<OperationTrackerCounters>, Arc<ApplyOrchestratorCounters>),
    fanout: (Arc<ProjectorCounters>, Arc<SnifferTranslatorCounters>),
    periodic: PeriodicCounters,
    wire: (
        Arc<dali2rust_platform::dali::PhySnifferCounters>,
        Arc<dali2rust_platform::dali::DaliWireCounters>,
    ),
    arbitration: (
        Arc<dali2rust_redundancy_runtime::ArbitrationSupervisorCounters>,
        Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
    ),
    bridges: (
        Arc<dali2rust_ws_runtime::WsCounters>,
        Arc<dali2rust_mqtt_runtime::MqttCounters>,
    ),
) -> RuntimeCounterHandles {
    counter_handles(CounterSources {
        pipeline,
        operations,
        fanout,
        periodic,
        wire,
        arbitration,
        websocket: bridges.0,
        mqtt: bridges.1,
    })
}

struct CounterSources {
    pipeline: (Arc<DaliWorkerCounters>, Arc<RegistryWorkerCounters>),
    operations: (Arc<OperationTrackerCounters>, Arc<ApplyOrchestratorCounters>),
    fanout: (Arc<ProjectorCounters>, Arc<SnifferTranslatorCounters>),
    periodic: PeriodicCounters,
    wire: (
        Arc<dali2rust_platform::dali::PhySnifferCounters>,
        Arc<dali2rust_platform::dali::DaliWireCounters>,
    ),
    arbitration: (
        Arc<dali2rust_redundancy_runtime::ArbitrationSupervisorCounters>,
        Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
    ),
    websocket: Arc<dali2rust_ws_runtime::WsCounters>,
    mqtt: Arc<dali2rust_mqtt_runtime::MqttCounters>,
}

struct PeriodicCounters {
    hcl: Arc<dali2rust_hcl_runtime::HclSchedulerCounters>,
    poller: Arc<dali2rust_poller_runtime::PollerCounters>,
    rules: Arc<dali2rust_rules_runtime::RulesWorkerCounters>,
    rules_engine: Arc<dali2rust_rules_runtime::RulesEngineCells>,
    arbitration: Arc<dali2rust_redundancy_runtime::ArbitrationWorkerCounters>,
    replication: Arc<dali2rust_redundancy_runtime::ReplicationCounters>,
}

fn counter_handles(src: CounterSources) -> RuntimeCounterHandles {
    RuntimeCounterHandles {
        arbitration_supervisor: src.arbitration.0,
        arbitration_reflex: src.arbitration.1,
        arbitration_worker: src.periodic.arbitration,
        replication: src.periodic.replication,
        dali_worker: src.pipeline.0,
        registry: src.pipeline.1,
        operation_tracker: src.operations.0,
        apply_orchestrator: src.operations.1,
        projector: src.fanout.0,
        sniffer_translator: src.fanout.1,
        phy_sniffer: src.wire.0,
        dali_wire: src.wire.1,
        hcl_scheduler: src.periodic.hcl,
        poller: src.periodic.poller,
        websocket: src.websocket,
        mqtt: src.mqtt,
        rules: src.periodic.rules,
        rules_engine: src.periodic.rules_engine,
    }
}
