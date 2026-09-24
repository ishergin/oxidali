mod app_router;
pub mod time_settings;
mod bus_host;
pub use bus_host::NAMED_EVENT_SUBSCRIBERS;
mod display_source;
mod bus_stack;
mod http_bridges;
mod registry_init;
mod workers;

use std::sync::{Arc, Mutex};

use dali2rust_api::http::handlers::static_assets::StaticAsset;
use dali2rust_api::http::router::Router;
use dali2rust_bus::{BusConfig, BusId, BusPublisher};
use dali2rust_dali_runtime::DaliRuntimeConfig;
pub use dali2rust_display_runtime::{DisplayView, HardwareDisplay};
use dali2rust_platform::dali::DaliTransport;
use dali2rust_platform::slice_store::SliceStore;
pub use dali2rust_registry_runtime::RegistryStore;
pub use dali2rust_ws_runtime::{
    spawn_ws_worker, RegisterRejected, WsCounters, WsHub, WsHubConfig, WsSink, WsSinkError,
    WsWorkerPorts, MAX_WS_CLIENTS,
};

use crate::dali::transport::mock::MockDaliTransport;

pub use bus_stack::BusStackRuntime;
use app_router::{
    build_app_router, hcl_override_read_port, operation_read_port, HttpBusDispatch, ReadModelPorts,
};
use bus_host::spawn_bus_subscribers;
use registry_init::init_registry_slice;
use workers::{
    spawn_bus_workers, BusWiring, HclSchedulerDeps, PollerWorkerDeps, RuntimeCounterHandles,
    WorkerChannels, WorkerSpawnInputs,
};

pub struct StackOptions {
    pub runtime_config: DaliRuntimeConfig,
    pub bus_config: BusConfig,
    pub bus_id: BusId,
    pub adapter_count: u8,
    pub persistence_slices: Option<Arc<dyn SliceStore>>,
    pub heap_stats: Option<Arc<dyn dali2rust_platform::heap::HeapStatsPort>>,
    pub network_link: Option<Arc<dyn dali2rust_platform::net::NetworkLink>>,
    pub wall_clock: Option<Arc<dali2rust_bsp::wall_clock::SystemWallClock>>,
    pub web_assets: &'static [StaticAsset],
    pub hcl_config: dali2rust_hcl_runtime::HclConfig,
    pub controller_hardware_id: Option<[u8; 6]>,
    pub mqtt_client: Option<dali2rust_platform::mqtt::MqttClientBundle>,
    pub firmware_update: Option<crate::ota::FirmwareUpdatePorts>,
}

impl Default for StackOptions {
    fn default() -> Self {
        Self {
            runtime_config: DaliRuntimeConfig::default(),
            bus_config: BusConfig::default(),
            bus_id: BusId::default(),
            adapter_count: 1,
            persistence_slices: None,
            heap_stats: None,
            network_link: None,
            wall_clock: None,
            web_assets: &[],
            hcl_config: dali2rust_hcl_runtime::HclConfig::default(),
            controller_hardware_id: None,
            mqtt_client: None,
            firmware_update: None,
        }
    }
}

struct PostBootOptions {
    runtime_config: DaliRuntimeConfig,
    gauges: http_bridges::PlatformGauges,
    web_assets: &'static [StaticAsset],
    firmware_update: Option<crate::ota::FirmwareUpdatePorts>,
}

fn boot_from_options(
    version: &'static str,
    options: StackOptions,
) -> (Box<BootedStack>, PostBootOptions) {
    let StackOptions {
        runtime_config,
        bus_config,
        bus_id,
        adapter_count,
        persistence_slices,
        heap_stats,
        network_link,
        wall_clock,
        web_assets,
        hcl_config,
        controller_hardware_id,
        mqtt_client,
        firmware_update,
    } = options;
    registry_init::log_internal_free("before workers");
    let booted = boot_stack(BootArgs {
        version,
        bus_config,
        bus_id,
        adapter_count,
        persistence_slices,
        wall_clock,
        hcl_config,
        controller_hardware_id,
        mqtt_client,
    });
    (
        booted,
        PostBootOptions {
            runtime_config,
            gauges: http_bridges::PlatformGauges {
                heap: heap_stats,
                link: network_link,
            },
            web_assets,
            firmware_update,
        },
    )
}

pub fn build_router_with_bus_and_transport<T: DaliTransport + Send + 'static>(
    version: &'static str,
    transport: Arc<Mutex<T>>,
    hardware_display: HardwareDisplay,
    options: StackOptions,
) -> (Router, Arc<WsHub>, Box<BusStackRuntime>) {
    let (mut booted, rest) = boot_from_options(version, options);
    let clock: Arc<dyn dali2rust_platform::clock::Clock> =
        Arc::new(dali2rust_dali_runtime::StdClock::new());
    let home_assistant = controller_ha_summary(&booted);
    let (workers, read_models) =
        spawn_fleet(transport, hardware_display, &mut booted, &rest, &clock);
    let rules = rules_http_deps(&booted);
    let router = assemble_router(
        RouterParts {
            version,
            adapter_count: booted.adapter_count,
            web_assets: rest.web_assets,
            dispatch: booted.dispatch,
            registry_http: booted.registry.http,
            wall_clock: booted.scheduling.wall_clock,
            persist_timezone: booted.scheduling.persist_timezone,
            read_models,
            clock,
            home_assistant,
            rules,
        },
        &workers,
    );
    (
        router,
        booted.ws.hub,
        Box::new(BusStackRuntime::new(
            booted.bus.host,
            booted.bus.publisher,
            workers,
        )),
    )
}

fn spawn_fleet<T: DaliTransport + Send + 'static>(
    transport: Arc<Mutex<T>>,
    hardware_display: HardwareDisplay,
    booted: &mut BootedStack,
    rest: &PostBootOptions,
    clock: &Arc<dyn dali2rust_platform::clock::Clock>,
) -> (workers::SpawnedWorkers, ReadModelPorts) {
    let WorkerChannels {
        pipeline,
        services,
        ws_ev,
    } = booted.bus.channels.take().expect("worker inboxes taken twice");
    let ws_ev = ws_ev.attach(&booted.bus.publisher);
    let inputs = worker_inputs(rest, booted, hardware_display, pipeline, services);
    let (mut workers, counter_handles) = spawn_bus_workers(transport, inputs);
    registry_init::log_internal_free("after workers");
    let read_models = boot_read_models(
        booted,
        rest,
        counter_handles,
        clock,
        Arc::clone(&workers.arbitration_transitions),
    );
    workers.ws_worker = Some(spawn_websocket_worker(&booted.ws, ws_ev, &read_models));
    (workers, read_models)
}

fn boot_read_models(
    booted: &mut BootedStack,
    rest: &PostBootOptions,
    counter_handles: RuntimeCounterHandles,
    clock: &Arc<dyn dali2rust_platform::clock::Clock>,
    transitions: dali2rust_redundancy_runtime::SharedTransitionLog,
) -> ReadModelPorts {
    read_model_ports(ReadModelArgs {
        clock: Arc::clone(clock),
        publisher: booted.bus.publisher.clone(),
        slots: Arc::clone(&booted.diagnostics_slots),
        counters: counter_handles,
        store: Arc::clone(&booted.diagnostics_store),
        gauges: rest.gauges.clone(),
        redundancy_settings: Arc::clone(&booted.diagnostics_store)
            as Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
        dali_settings: Arc::clone(&booted.diagnostics_store)
            as Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
        transitions,
        ota: booted.ota.clone(),
        firmware_port: rest.firmware_update.as_ref().map(|p| Arc::clone(&p.port)),
    })
}

fn controller_ha_summary(
    booted: &BootedStack,
) -> Arc<dyn dali2rust_api::http::handlers::controller::ControllerHaSummary> {
    Arc::new(http_bridges::ControllerHaBridge::new(
        Arc::clone(&booted.registry.http.home_assistant_settings_port),
        Arc::clone(&booted.mqtt_counters),
    ))
}

fn worker_inputs(
    rest: &PostBootOptions,
    booted: &BootedStack,
    hardware_display: HardwareDisplay,
    pipeline_channels: workers::PipelineChannels,
    service_channels: workers::ServiceChannels,
) -> WorkerSpawnInputs {
    WorkerSpawnInputs {
        runtime_config: rest.runtime_config,
        wiring: bus_wiring(booted.bus_id, &booted.bus.publisher, &booted.dispatch.slots),
        registry: booted.registry.deps.clone(),
        pipeline_channels,
        service_channels,
        hardware_display,
        hcl: booted.scheduling.hcl.clone(),
        poller: booted.scheduling.poller.clone(),
        interactive: Arc::clone(&booted.interactive),
        sniffer_tap: Arc::clone(&booted.ws.tap),
        ws_counters: Arc::clone(booted.ws.hub.counters()),
        mqtt: booted.mqtt.lock().expect("mqtt wiring lock").take(),
        mqtt_counters: Arc::clone(&booted.mqtt_counters),
        rules: rules_worker_deps(booted),
        correlation: Arc::clone(&booted.scheduling.poller.correlation),
        ota: ota_worker_deps(rest, booted),
    }
}

fn rules_worker_deps(booted: &BootedStack) -> workers::RulesWorkerDeps {
    workers::RulesWorkerDeps {
        store: Arc::clone(&booted.rules_store),
        compiler: Arc::new(dali2rust_rules_lang::RulesLangV1),
        resolver: Arc::new(http_bridges::RegistryNameResolver::new(
            Arc::clone(&booted.diagnostics_store),
            0,
            booted.adapter_count,
        )),
        slices: booted.registry.deps.persistence_slices.clone(),
        registry: Arc::clone(&booted.diagnostics_store),
        wall: Arc::clone(&booted.scheduling.wall_clock),
        hcl_state: Arc::clone(&booted.registry.http.hcl_state),
    }
}

struct RouterParts {
    version: &'static str,
    adapter_count: u8,
    web_assets: &'static [StaticAsset],
    dispatch: HttpBusDispatch,
    registry_http: http_bridges::RegistryHttpPorts,
    wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    persist_timezone: Arc<dyn Fn(&str) + Send + Sync>,
    read_models: ReadModelPorts,
    clock: Arc<dyn dali2rust_platform::clock::Clock>,
    home_assistant: Arc<dyn dali2rust_api::http::handlers::controller::ControllerHaSummary>,
    rules: app_router::RulesHttpDeps,
}

fn assemble_router(parts: RouterParts, workers: &workers::SpawnedWorkers) -> Router {
    build_app_router(
        parts.version,
        parts.adapter_count,
        parts.dispatch,
        parts.registry_http,
        operation_read_port(Arc::clone(&workers.operation_tracker)),
        hcl_override_read_port(Arc::clone(&workers.hcl_overrides)),
        parts.read_models,
        parts.clock,
        parts.wall_clock,
        parts.persist_timezone,
        parts.web_assets,
        parts.home_assistant,
        parts.rules,
    )
}

fn rules_http_deps(booted: &BootedStack) -> app_router::RulesHttpDeps {
    app_router::RulesHttpDeps {
        state: Arc::new(http_bridges::RulesHttpBridge::new(Arc::clone(
            &booted.rules_store,
        ))),
        compiler: Arc::new(dali2rust_rules_lang::RulesLangV1),
        resolver: Arc::new(http_bridges::RegistryNameResolver::new(
            Arc::clone(&booted.diagnostics_store),
            0,
            booted.adapter_count,
        )),
    }
}

struct BootedStack {
    bus_id: BusId,
    adapter_count: u8,
    bus: bus_host::BusSubscribers,
    dispatch: HttpBusDispatch,
    registry: registry_init::RegistrySlice,
    scheduling: SchedulingDeps,
    diagnostics_store: Arc<RegistryStore>,
    rules_store: Arc<dali2rust_rules_runtime::RulesStore>,
    diagnostics_slots: Arc<dali2rust_api::confirmation_bridge::PendingConfirmationSlots>,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
    ws: WsWiring,
    mqtt: Mutex<Option<workers::MqttWorkerDeps>>,
    mqtt_counters: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    ota: OtaShared,
}

#[derive(Clone)]
struct OtaShared {
    state: Arc<dali2rust_ota_runtime::OtaState>,
    hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
}

impl Default for OtaShared {
    fn default() -> Self {
        Self {
            state: Arc::new(dali2rust_ota_runtime::OtaState::default()),
            hold: Arc::new(dali2rust_platform::firmware::MaintenanceHold::new()),
        }
    }
}

fn ota_worker_deps(
    rest: &PostBootOptions,
    booted: &BootedStack,
) -> Option<workers::OtaWorkerDeps> {
    Some(workers::OtaWorkerDeps {
        ports: rest.firmware_update.clone()?,
        state: Arc::clone(&booted.ota.state),
        hold: Arc::clone(&booted.ota.hold),
    })
}

struct WsWiring {
    hub: Arc<WsHub>,
    tap: Arc<dali2rust_platform::dali::SnifferTap>,
    sniffer_rx: Mutex<Option<std::sync::mpsc::Receiver<dali2rust_platform::dali::SnifferRecord>>>,
}

fn spawn_websocket_worker(
    ws: &WsWiring,
    ws_ev: workers::WsInbox,
    read_models: &ReadModelPorts,
) -> std::thread::JoinHandle<()> {
    spawn_ws_worker(
        ws_ev.rx,
        ws_ev.probe,
        ws.sniffer_rx
            .lock()
            .expect("sniffer receiver")
            .take()
            .expect("sniffer receiver taken twice"),
        Arc::clone(&ws.tap),
        Arc::clone(&ws.hub),
        WsWorkerPorts {
            stats: Arc::clone(&read_models.stats),
            diagnostics: Arc::clone(&read_models.diagnostics),
        },
        Arc::new(dali2rust_platform::dali::wall_clock_millis),
    )
}

fn wire_websocket() -> WsWiring {
    let (tap, sniffer_rx) =
        dali2rust_platform::dali::SnifferTap::new(workers::SNIFFER_WINDOW_CAPACITY);
    let log_ring = dali2rust_bsp::log_ring::global();
    dali2rust_bsp::log_ring::install_capture_source();
    let hub = WsHub::new(
        WsHubConfig::default(),
        Arc::new(WsCounters::default()),
        Arc::clone(&tap),
        log_ring,
    );
    WsWiring {
        hub,
        tap,
        sniffer_rx: Mutex::new(Some(sniffer_rx)),
    }
}

struct BootArgs {
    version: &'static str,
    bus_config: BusConfig,
    bus_id: BusId,
    adapter_count: u8,
    persistence_slices: Option<Arc<dyn SliceStore>>,
    wall_clock: Option<Arc<dali2rust_bsp::wall_clock::SystemWallClock>>,
    hcl_config: dali2rust_hcl_runtime::HclConfig,
    controller_hardware_id: Option<[u8; 6]>,
    mqtt_client: Option<dali2rust_platform::mqtt::MqttClientBundle>,
}

fn boot_mqtt(
    client: Option<dali2rust_platform::mqtt::MqttClientBundle>,
    adapter_count: u8,
    version: &'static str,
    registry: &registry_init::RegistrySlice,
    bus: &bus_host::BusSubscribers,
    dispatch: &HttpBusDispatch,
) -> Option<workers::MqttWorkerDeps> {
    wire_mqtt(
        &Arc::clone(&registry.deps.store),
        client,
        &bus.publisher.clone(),
        &Arc::clone(&dispatch.correlation),
        Arc::clone(&registry.deps.counters),
        adapter_count,
        version,
    )
}

fn boot_stack(args: BootArgs) -> Box<BootedStack> {
    registry_init::log_internal_free("composition start");
    let interactive = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let bus = spawn_bus_subscribers(args.bus_config, args.bus_id, Arc::clone(&interactive));
    let dispatch = HttpBusDispatch::for_bus(args.bus_config, args.bus_id, bus.publisher.clone());
    let (clock_wiring, registry) = boot_clock_and_registry(&args, &bus);
    let diagnostics_store = Arc::clone(&registry.deps.store);
    let diagnostics_slots = Arc::clone(&dispatch.slots);
    let mqtt = boot_mqtt(
        args.mqtt_client,
        args.adapter_count,
        args.version,
        &registry,
        &bus,
        &dispatch,
    );
    let ota = OtaShared::default();
    let scheduling = resolve_scheduling_deps(
        &registry.deps.store,
        &dispatch.correlation,
        clock_wiring,
        args.hcl_config,
        &ota.hold,
    );
    Box::new(BootedStack {
        bus_id: args.bus_id,
        adapter_count: args.adapter_count,
        bus,
        dispatch,
        registry,
        scheduling,
        diagnostics_store,
        diagnostics_slots,
        rules_store: Arc::new(dali2rust_rules_runtime::RulesStore::new()),
        interactive,
        ws: wire_websocket(),
        mqtt: Mutex::new(mqtt),
        mqtt_counters: Arc::new(dali2rust_mqtt_runtime::MqttCounters::default()),
        ota,
    })
}

fn wire_mqtt(
    store: &Arc<RegistryStore>,
    bundle: Option<dali2rust_platform::mqtt::MqttClientBundle>,
    publisher: &BusPublisher,
    correlation: &Arc<dali2rust_bus::CorrelationIdAllocator>,
    registry_counters: Arc<dali2rust_registry_runtime::RegistryWorkerCounters>,
    adapter_count: u8,
    version: &'static str,
) -> Option<workers::MqttWorkerDeps> {
    let bundle = bundle?;
    let settings_watch = Arc::new(dali2rust_registry_runtime::RegistryApplyWatch::new(
        registry_counters,
    ));
    let link = bundle.client.link();
    Some(workers::MqttWorkerDeps {
        publisher: publisher.clone(),
        correlation: Arc::clone(correlation),
        client: bundle.client,
        incoming: bundle.incoming,
        link,
        read_port: store.clone(),
        settings: store.clone(),
        settings_watch,
        secret: store.clone(),
        role: store.clone(),
        version,
        adapter_count,
    })
}

fn boot_clock_and_registry(
    args: &BootArgs,
    bus: &crate::runtime::bus_host::BusSubscribers,
) -> (ClockWiring, registry_init::RegistrySlice) {
    let clock_wiring = wire_wall_clock(args.wall_clock.clone(), &args.persistence_slices);
    let registry = init_registry_slice(
        args.adapter_count,
        args.persistence_slices.clone(),
        &bus.publisher,
        args.controller_hardware_id,
        Arc::clone(&clock_wiring.0),
    );
    (clock_wiring, registry)
}

struct SchedulingDeps {
    hcl: HclSchedulerDeps,
    poller: PollerWorkerDeps,
    wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    persist_timezone: Arc<dyn Fn(&str) + Send + Sync>,
}

fn resolve_scheduling_deps(
    store: &Arc<RegistryStore>,
    correlation: &Arc<dali2rust_bus::CorrelationIdAllocator>,
    clock_wiring: ClockWiring,
    hcl_config: dali2rust_hcl_runtime::HclConfig,
    hold: &Arc<dali2rust_platform::firmware::MaintenanceHold>,
) -> SchedulingDeps {
    let (wall_clock, persist_timezone) = clock_wiring;
    let hcl = hcl_deps(store, &wall_clock, correlation, hcl_config, hold);
    let poller = poller_deps(store, correlation, hold);
    SchedulingDeps {
        hcl,
        poller,
        wall_clock,
        persist_timezone,
    }
}

type ClockWiring = (
    Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    Arc<dyn Fn(&str) + Send + Sync>,
);

fn wire_wall_clock(
    provided: Option<Arc<dali2rust_bsp::wall_clock::SystemWallClock>>,
    persistence_slices: &Option<Arc<dyn SliceStore>>,
) -> ClockWiring {
    let clock: Arc<dyn dali2rust_platform::wall_clock::WallClock> = provided
        .map(|clock| clock as Arc<dyn dali2rust_platform::wall_clock::WallClock>)
        .unwrap_or_else(|| Arc::new(dali2rust_bsp::wall_clock::SystemWallClock::new()));
    time_settings::hydrate_timezone(clock.as_ref(), persistence_slices.as_ref());
    let persist = time_settings::timezone_writer(persistence_slices.clone());
    (clock, persist)
}

fn hcl_deps(
    store: &Arc<RegistryStore>,
    wall_clock: &Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    correlation: &Arc<dali2rust_bus::CorrelationIdAllocator>,
    config: dali2rust_hcl_runtime::HclConfig,
    hold: &Arc<dali2rust_platform::firmware::MaintenanceHold>,
) -> HclSchedulerDeps {
    HclSchedulerDeps {
        read_port: Arc::clone(store) as Arc<dyn dali2rust_domain::registry::HclSchedulerReadPort>,
        clock: Arc::clone(wall_clock),
        correlation: Arc::clone(correlation),
        config,
        role: Arc::clone(store) as Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
        hold: Arc::clone(hold),
    }
}

fn poller_deps(
    store: &Arc<RegistryStore>,
    correlation: &Arc<dali2rust_bus::CorrelationIdAllocator>,
    hold: &Arc<dali2rust_platform::firmware::MaintenanceHold>,
) -> PollerWorkerDeps {
    PollerWorkerDeps {
        read_port: Arc::clone(store) as Arc<dyn dali2rust_domain::registry::PollerReadPort>,
        correlation: Arc::clone(correlation),
        role: Arc::clone(store) as Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
        hold: Arc::clone(hold),
    }
}

fn bus_wiring(
    bus_id: BusId,
    publisher: &dali2rust_bus::BusPublisher,
    slots: &Arc<dali2rust_api::confirmation_bridge::PendingConfirmationSlots>,
) -> BusWiring {
    BusWiring {
        bus_id,
        publisher: publisher.clone(),
        slots: Arc::clone(slots),
    }
}

struct ReadModelArgs {
    clock: Arc<dyn dali2rust_platform::clock::Clock>,
    publisher: dali2rust_bus::BusPublisher,
    slots: Arc<dali2rust_api::confirmation_bridge::PendingConfirmationSlots>,
    counters: RuntimeCounterHandles,
    store: Arc<RegistryStore>,
    gauges: http_bridges::PlatformGauges,
    redundancy_settings: Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
    dali_settings: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    transitions: dali2rust_redundancy_runtime::SharedTransitionLog,
    ota: OtaShared,
    firmware_port: Option<Arc<dyn dali2rust_platform::firmware::FirmwareUpdatePort>>,
}

fn read_model_ports(args: ReadModelArgs) -> ReadModelPorts {
    ReadModelPorts {
        diagnostics: Arc::new(http_bridges::DiagnosticsBridge::new(
            args.publisher.clone(),
            Arc::clone(&args.slots),
            args.counters.clone(),
            args.store,
        )),
        redundancy: Arc::new(http_bridges::RedundancyBridge::new(
            Arc::clone(&args.redundancy_settings),
            Arc::clone(&args.dali_settings),
            Arc::clone(&args.counters.arbitration_reflex),
            Arc::clone(&args.counters.arbitration_worker),
            args.transitions,
            Arc::clone(&args.counters.replication),
        )),
        stats: Arc::new(http_bridges::StatsBridge::new(
            args.clock,
            args.publisher,
            args.slots,
            args.counters,
            args.gauges,
        )),
        firmware: Arc::new(http_bridges::FirmwareBridge::new(
            args.ota.state,
            args.firmware_port,
        )),
    }
}

pub fn build_http_test_stack(
    version: &'static str,
    mock: Arc<Mutex<MockDaliTransport>>,
    config: BusConfig,
    runtime_config: DaliRuntimeConfig,
    persistence_slices: Option<Arc<dyn SliceStore>>,
    web_assets: &'static [StaticAsset],
    mqtt_client: Option<dali2rust_platform::mqtt::MqttClientBundle>,
) -> (Router, Arc<WsHub>, Box<BusStackRuntime>) {
    build_router_with_bus_and_transport(
        version,
        mock,
        HardwareDisplay::none(),
        StackOptions {
            bus_config: config,
            runtime_config,
            hcl_config: dali2rust_hcl_runtime::HclConfig {
                tick_period_ms: 250,
                command_timeout_ms: 500,
            },
            adapter_count: 2,
            persistence_slices,
            web_assets,
            mqtt_client,
            firmware_update: Some(crate::ota::FirmwareUpdatePorts::host()),
            ..StackOptions::default()
        },
    )
}
