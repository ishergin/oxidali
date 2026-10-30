use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, CorrelationIdAllocator};
use dali2rust_contracts::msg::{
    Dali103FeedbackConfigureCommand, Dali103ScanCommand, FeedbackPatchField, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::dev103::{
    feedback_capability, Device103Address, Device103Command, Feedback332Command,
    FeedbackOpcodeMap, Instance103Command, InstanceAddress,
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
const FEEDBACK_CORRELATION: u64 = 121;
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
    publisher: dali2rust_bus::BusPublisher,
    transport: Arc<Mutex<MockDaliTransport>>,
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
        DaliController::new(Arc::clone(&transport), Box::new(StdClock::new())),
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
    Stack { store, registry_counters, dali_counters, publisher, transport, _host: host }
}

#[test]
fn every_instance_type_lands_on_its_own_number() {
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

fn feedback_probe(stack: &Stack, instance_number: u8) -> Option<u8> {
    let detail = stack.store.input_device_detail(0, PANEL)?;
    let instance = detail.instances.iter().find(|i| i.instance_number == instance_number)?;
    instance.feedback_probe
}

#[test]
fn a_feedback_write_that_finds_the_2017_map_leaves_it_for_the_next_write() {
    let stack = scan_the_panel();
    wait_until(|| feedback_probe(&stack, 0) == Some(FeedbackOpcodeMap::ABSENT_CODE), WAIT);
    let ed1_capability = Feedback332Command::QueryCapability
        .frame(Device103Address::Short(PANEL), InstanceAddress::FeatureNumber(0), FeedbackOpcodeMap::Ed1)
        .as_bytes();
    stack
        .transport
        .lock()
        .expect("mock lock")
        .script_frame24_answer(ed1_capability, feedback_capability::VISIBLE);
    let mut write = Dali103FeedbackConfigureCommand {
        registry_adapter_id: 0,
        short_address: PANEL,
        instance_number: 0,
        patch_mask: 0,
        timing: 0,
        active_brightness: 0,
        active_colour: 0,
        inactive_brightness: 0,
        inactive_colour: 0,
        opcode_map: FeedbackOpcodeMap::ABSENT_CODE,
    };
    write.patch(FeedbackPatchField::ActiveBrightness, 1);
    let envelope = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        FEEDBACK_CORRELATION,
        BusId::default().0,
        Some(Origin::Api),
        write,
    );
    publish_queued(&stack.publisher, BusChannel::Commands, BusFrame::command(envelope));
    wait_until(|| feedback_probe(&stack, 0) == Some(FeedbackOpcodeMap::Ed1.code()), WAIT);
}
