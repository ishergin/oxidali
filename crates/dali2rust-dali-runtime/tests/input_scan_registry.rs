use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, CorrelationIdAllocator};
use dali2rust_contracts::msg::{Dali103ScanCommand, Origin};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::dev103::{
    Device103Address, Device103Command, Instance103Command, InstanceAddress,
};
use dali2rust_platform::dali::WireActivity;
use dali2rust_platform::liveness::LivenessBeat;
use dali2rust_registry_runtime::{
    spawn_registry_worker, RegistryStore, RegistryWorkerCounters, REGISTRY_EVENTS_HANDLED_EVENTS,
    REGISTRY_WORKER_HANDLED_COMMANDS,
};
use dali2rust_test_support::{publish_queued, wait_until};

const ADAPTERS: u8 = 1;
const PANEL: u8 = 5;
const INSTANCE_COUNT: u8 = 10;
const SILENT_INSTANCE: u8 = 1;
const LAST_INSTANCE: u8 = INSTANCE_COUNT - 1;
const SCAN_CORRELATION: u64 = 120;
const WAIT: Duration = Duration::from_secs(20);
const LIVENESS_BUDGET_MS: u32 = 60_000;

fn instance_type(instance_number: u8) -> u8 {
    instance_number % 4 + 1
}

fn script_panel(transport: &MockDaliTransport) {
    let device = Device103Address::Short(PANEL);
    transport.script_frame24_answer(
        Device103Command::QueryNumberOfInstances.frame(device).as_bytes(),
        INSTANCE_COUNT,
    );
    for number in (0..INSTANCE_COUNT).filter(|n| *n != SILENT_INSTANCE) {
        transport.script_frame24_answer(
            Instance103Command::QueryInstanceType
                .frame(device, InstanceAddress::Number(number))
                .as_bytes(),
            instance_type(number),
        );
    }
}

struct Stack {
    store: Arc<RegistryStore>,
    registry_counters: Arc<RegistryWorkerCounters>,
    dali_counters: Arc<DaliWorkerCounters>,
    _host: BusHost,
}

fn scan_the_panel() -> Stack {
    let (host, publisher, (dali_rx, registry_rx)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS),
            reg.subscribe_commands_and_events(
                256,
                REGISTRY_WORKER_HANDLED_COMMANDS,
                REGISTRY_EVENTS_HANDLED_EVENTS,
            ),
        )
    });
    let store = Arc::new(RegistryStore::with_adapter_count(ADAPTERS));
    let registry_counters = Arc::new(RegistryWorkerCounters::default());
    spawn_registry_worker(
        registry_rx,
        publisher.clone(),
        BusId::default(),
        ADAPTERS,
        Arc::clone(&store),
        Arc::clone(&registry_counters),
        None,
        Arc::new(LivenessBeat::new("test", LIVENESS_BUDGET_MS)),
    );
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    script_panel(&transport.lock().expect("mock lock"));
    let dali_counters = Arc::new(DaliWorkerCounters::default());
    spawn_dali_worker(
        dali_rx,
        DaliController::new(transport, Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::clone(&store) as _,
        publisher.clone(),
        BusId::default(),
        Arc::clone(&dali_counters),
        Arc::new(WireActivity::new()),
        Arc::new(CorrelationIdAllocator::new()),
    );
    let scan = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        SCAN_CORRELATION,
        BusId::default().0,
        Some(Origin::Api),
        Dali103ScanCommand { registry_adapter_id: 0 },
    );
    publish_queued(&publisher, BusChannel::Commands, BusFrame::command(scan));
    Stack { store, registry_counters, dali_counters, _host: host }
}

#[test]
fn every_instance_type_lands_on_its_own_number_issue120() {
    let stack = scan_the_panel();
    let answered = u32::from(INSTANCE_COUNT - 1);
    wait_until(
        || {
            stack.dali_counters.input_scans_completed.load(Ordering::Relaxed) >= 1
                && stack
                    .registry_counters
                    .events
                    .input_instance_readbacks_applied
                    .load(Ordering::Relaxed)
                    >= answered
        },
        WAIT,
    );
    let type_of = |number| stack.store.input_instance_type(0, PANEL, number);
    assert_eq!(type_of(0), Some(instance_type(0)));
    assert_eq!(
        type_of(SILENT_INSTANCE),
        None,
        "an instance that did not answer its type query has no type"
    );
    assert_eq!(
        type_of(SILENT_INSTANCE + 1),
        Some(instance_type(SILENT_INSTANCE + 1)),
        "the silent instance must not shift its neighbour's type onto itself"
    );
    assert_eq!(
        type_of(LAST_INSTANCE),
        Some(instance_type(LAST_INSTANCE)),
        "a type past the eighth instance reaches the registry"
    );
}
