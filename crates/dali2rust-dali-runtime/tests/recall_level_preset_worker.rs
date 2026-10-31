use std::sync::Arc;
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{
    BusEventPayload, DaliLevelTransitionAppliedEvent, DaliRecallLevelPresetCommand, DaliTargetScope,
    DeliveryStatus, LevelPreset, LevelTransition, Origin, RuntimeSource,
};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_registry_runtime::RegistryStore;

const APPLIED: &str = "DaliLevelTransitionAppliedEvent";

fn frame_of(address: DaliAddress, command: StandardCommand) -> u16 {
    DaliCommand::Standard { address, command }.to_forward_frame().raw()
}

struct Stack {
    publisher: dali2rust_bus::BusPublisher,
    _host: BusHost,
    confirmations: std::sync::mpsc::Receiver<BusFrame>,
    events: std::sync::mpsc::Receiver<BusFrame>,
    transport: Arc<std::sync::Mutex<MockDaliTransport>>,
    _worker: std::thread::JoinHandle<()>,
}

fn spawn_stack(scripted: &[u16]) -> Stack {
    let (host, publisher, (worker_cmd, confirmations, events)) =
        BusHost::spawn(BusConfig::default(), |reg| {
            (
                reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_confirmations(16),
                reg.subscribe_events(16, &[APPLIED]),
            )
        });
    let transport = Arc::new(std::sync::Mutex::new(MockDaliTransport::new()));
    for frame in scripted {
        transport.lock().expect("mock lock").expect_forward_frame(*frame);
    }
    let controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
    let worker = spawn_dali_worker(
        worker_cmd,
        controller,
        DaliRuntimeConfig::default(),
        Arc::new(RegistryStore::with_adapter_count(1)),
        publisher.clone(),
        BusId::default(),
        Arc::new(DaliWorkerCounters::default()),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    );
    Stack { publisher, _host: host, confirmations, events, transport, _worker: worker }
}

fn publish(stack: &Stack, correlation_id: u64, origin: Origin, command: DaliRecallLevelPresetCommand) {
    let frame = BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        Some(origin),
        command,
    ));
    assert_eq!(stack.publisher.try_publish(BusChannel::Commands, frame), PublishResult::Queued);
}

fn confirmation(stack: &Stack) -> Arc<dali2rust_contracts::msg::ConfirmationEnvelope> {
    let BusFrame::Confirmation(conf) =
        stack.confirmations.recv_timeout(Duration::from_secs(1)).expect("confirmation")
    else {
        panic!("expected a confirmation frame");
    };
    conf
}

fn applied(stack: &Stack) -> DaliLevelTransitionAppliedEvent {
    let BusFrame::Event(ev) =
        stack.events.recv_timeout(Duration::from_secs(1)).expect("the recall must publish its applied fact")
    else {
        panic!("expected an event frame");
    };
    let BusEventPayload::DaliLevelTransitionAppliedEvent(applied) = &ev.payload else {
        panic!("subscribed to one kind");
    };
    applied.clone()
}

#[test]
fn a_short_address_recall_sends_its_opcode_and_closes_with_the_transition_and_its_source() {
    let expected = frame_of(DaliAddress::Short(3), StandardCommand::RecallMaxLevel);
    let stack = spawn_stack(&[expected]);
    publish(&stack, 61, Origin::Mqtt, DaliRecallLevelPresetCommand::for_short(0, 3, LevelPreset::MaxLevel));

    assert_eq!(confirmation(&stack).status, DeliveryStatus::Ok);
    let fact = applied(&stack);
    assert_eq!(fact.scope, DaliTargetScope::Short);
    assert_eq!((fact.virtual_lamp_id, fact.short_address, fact.group_id), (None, Some(3), None));
    assert_eq!(fact.transition, LevelTransition::RecallMaxLevel);
    assert_eq!(fact.source, RuntimeSource::Mqtt);

    let mock = stack.transport.lock().expect("mock lock");
    assert_eq!(mock.sent_frames(), vec![expected]);
    assert_eq!(expected, 0x0705);
    assert_eq!(mock.script_error(), None);
}

#[test]
fn a_group_recall_of_the_minimum_addresses_the_group() {
    let expected = frame_of(DaliAddress::Group(5), StandardCommand::RecallMinLevel);
    let stack = spawn_stack(&[expected]);
    publish(&stack, 62, Origin::Rules, DaliRecallLevelPresetCommand::for_group(0, 5, LevelPreset::MinLevel));

    assert_eq!(confirmation(&stack).status, DeliveryStatus::Ok);
    let fact = applied(&stack);
    assert_eq!((fact.scope, fact.group_id, fact.short_address), (DaliTargetScope::Group, Some(5), None));
    assert_eq!(fact.transition, LevelTransition::RecallMinLevel);
    assert_eq!(fact.source, RuntimeSource::Rules);
    assert_eq!(stack.transport.lock().expect("mock lock").sent_frames(), vec![0x8B06]);
}

#[test]
fn an_unbound_virtual_lamp_is_refused_before_the_wire_and_states_nothing() {
    let stack = spawn_stack(&[]);
    publish(&stack, 63, Origin::Api, DaliRecallLevelPresetCommand::for_virtual_lamp(0, 2, LevelPreset::MaxLevel));

    let conf = confirmation(&stack);
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    assert_eq!(conf.confirmation.error.as_ref().expect("product error").message.as_str(), "vl_unbound");
    assert!(stack.events.recv_timeout(Duration::from_millis(100)).is_err(), "a refused recall states no transition");
    assert!(stack.transport.lock().expect("mock lock").sent_frames().is_empty());
}
