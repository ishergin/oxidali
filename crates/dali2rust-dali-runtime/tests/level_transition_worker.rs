use std::sync::Arc;
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{
    BusCommandPayload, BusEventPayload, DaliDimCommand, DaliLevelTransitionAppliedEvent,
    DaliRecallLevelPresetCommand, DaliStepCommand, DaliStopFadeCommand, DaliTargetScope, DeliveryStatus,
    DimDirection, ErrorCode, LevelPreset, LevelTransition, Origin, RuntimeSource, StepKind,
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
const STOP_FADE_APPLIED: &str = "DaliStopFadeAppliedEvent";

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
                reg.subscribe_events(16, &[APPLIED, STOP_FADE_APPLIED]),
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

fn publish(stack: &Stack, correlation_id: u64, origin: Origin, command: impl Into<BusCommandPayload>) {
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
        stack.events.recv_timeout(Duration::from_secs(1)).expect("the transition must publish its applied fact")
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
    let mut command = DaliRecallLevelPresetCommand::for_short(0, 3, LevelPreset::MaxLevel);
    command.hold_hcl = false;
    publish(&stack, 61, Origin::Mqtt, command);

    let fact = applied(&stack);
    assert_eq!(fact.scope, DaliTargetScope::Short);
    assert_eq!((fact.virtual_lamp_id, fact.short_address, fact.group_id), (None, Some(3), None));
    assert_eq!(fact.transition, LevelTransition::RecallMaxLevel);
    assert_eq!(fact.source, RuntimeSource::Mqtt);
    assert!(!fact.hold_hcl, "the fact carries the command's hold");
    assert!(
        stack.confirmations.recv_timeout(Duration::from_millis(200)).is_err(),
        "one device's recall is answered by the registry once it has committed it"
    );

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
    let error = conf.confirmation.error.as_ref().expect("product error");
    assert_eq!((error.code, error.message.as_str()), (ErrorCode::VlUnbound, "vl_unbound"));
    assert!(stack.events.recv_timeout(Duration::from_millis(100)).is_err(), "a refused recall states no transition");
    assert!(stack.transport.lock().expect("mock lock").sent_frames().is_empty());
}

fn dim(scope: DaliTargetScope, target: u8, direction: DimDirection) -> DaliDimCommand {
    DaliDimCommand {
        registry_adapter_id: 0,
        scope,
        virtual_lamp_id: if scope == DaliTargetScope::VirtualLamp { target } else { 0 },
        short_address: if scope == DaliTargetScope::Short { target } else { 0 },
        group_id: if scope == DaliTargetScope::Group { target } else { 0 },
        direction,
        hold_hcl: true,
    }
}

#[test]
fn a_short_address_dim_sends_one_up_frame_and_leaves_the_answer_to_the_registry() {
    let expected = frame_of(DaliAddress::Short(3), StandardCommand::Up);
    let stack = spawn_stack(&[expected]);
    let mut command = dim(DaliTargetScope::Short, 3, DimDirection::Up);
    command.hold_hcl = false;
    publish(&stack, 71, Origin::Rules, command);

    let fact = applied(&stack);
    assert_eq!((fact.scope, fact.short_address, fact.group_id), (DaliTargetScope::Short, Some(3), None));
    assert_eq!(fact.transition, LevelTransition::Up);
    assert_eq!(fact.source, RuntimeSource::Rules);
    assert!(!fact.hold_hcl, "the fact carries the command's hold");
    assert!(
        stack.confirmations.recv_timeout(Duration::from_millis(200)).is_err(),
        "one device's dim is answered by the registry once it has committed it"
    );
    let mock = stack.transport.lock().expect("mock lock");
    assert_eq!(mock.sent_frames(), vec![0x0701]);
    assert_eq!(mock.script_error(), None);
}

#[test]
fn a_group_or_broadcast_dim_is_one_frame_on_its_address_and_the_worker_answers_it() {
    for (corr, scope, target, direction, frame, transition) in [
        (72u64, DaliTargetScope::Group, 5u8, DimDirection::Down, 0x8B02u16, LevelTransition::Down),
        (73, DaliTargetScope::Broadcast, 0, DimDirection::Up, 0xFF01, LevelTransition::Up),
    ] {
        let stack = spawn_stack(&[frame]);
        publish(&stack, corr, Origin::Api, dim(scope, target, direction));

        assert_eq!(confirmation(&stack).status, DeliveryStatus::Ok, "{scope:?}");
        let fact = applied(&stack);
        assert_eq!((fact.scope, fact.transition, fact.short_address), (scope, transition, None));
        assert_eq!(fact.group_id, (scope == DaliTargetScope::Group).then_some(target));
        assert!(fact.hold_hcl);
        assert_eq!(stack.transport.lock().expect("mock lock").sent_frames(), vec![frame], "{scope:?}");
    }
}

#[test]
fn a_dim_of_an_unbound_virtual_lamp_is_refused_before_the_wire() {
    let stack = spawn_stack(&[]);
    publish(&stack, 74, Origin::Rules, dim(DaliTargetScope::VirtualLamp, 2, DimDirection::Down));

    let conf = confirmation(&stack);
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    let error = conf.confirmation.error.as_ref().expect("product error");
    assert_eq!(error.code, ErrorCode::VlUnbound);
    assert!(stack.events.recv_timeout(Duration::from_millis(100)).is_err(), "a refused dim states no transition");
    assert!(stack.transport.lock().expect("mock lock").sent_frames().is_empty());
}

#[test]
fn each_step_on_a_group_is_its_own_opcode_and_closes_with_its_transition() {
    for (corr, step, frame, transition) in [
        (81u64, StepKind::Up, 0x8B03u16, LevelTransition::StepUp),
        (82, StepKind::Down, 0x8B04, LevelTransition::StepDown),
        (83, StepKind::OnAndUp, 0x8B08, LevelTransition::OnAndStepUp),
        (84, StepKind::DownAndOff, 0x8B07, LevelTransition::StepDownAndOff),
    ] {
        let stack = spawn_stack(&[frame]);
        publish(&stack, corr, Origin::Cluster, DaliStepCommand::for_group(0, 5, step));

        assert_eq!(confirmation(&stack).status, DeliveryStatus::Ok, "{step:?}");
        let fact = applied(&stack);
        assert_eq!((fact.scope, fact.group_id, fact.transition), (DaliTargetScope::Group, Some(5), transition));
        assert_eq!(fact.source, RuntimeSource::Cluster, "{step:?}");
        assert_eq!(stack.transport.lock().expect("mock lock").sent_frames(), vec![frame], "{step:?}");
    }
}

#[test]
fn two_steps_published_together_are_two_frames_and_two_facts() {
    let frame = frame_of(DaliAddress::Broadcast, StandardCommand::StepUp);
    let stack = spawn_stack(&[frame, frame]);
    publish(&stack, 85, Origin::Api, DaliStepCommand::broadcast(0, StepKind::Up));
    publish(&stack, 86, Origin::Api, DaliStepCommand::broadcast(0, StepKind::Up));

    for _ in 0..2 {
        assert_eq!(confirmation(&stack).status, DeliveryStatus::Ok);
        assert_eq!(applied(&stack).transition, LevelTransition::StepUp);
    }
    assert_eq!(stack.transport.lock().expect("mock lock").sent_frames(), vec![0xFF03, 0xFF03]);
}

#[test]
fn a_stopped_fade_on_a_group_states_its_fact_before_the_answer() {
    let stack = spawn_stack(&[0x8AFF]);
    publish(&stack, 87, Origin::Api, DaliStopFadeCommand::for_group(0, 5));

    let BusFrame::Event(ev) = stack.events.recv_timeout(Duration::from_secs(1)).expect("the stopped fade's fact")
    else {
        panic!("expected an event frame");
    };
    let BusEventPayload::DaliStopFadeAppliedEvent(fact) = &ev.payload else {
        panic!("expected the stopped fade's fact, got {:?}", ev.payload.variant_name());
    };
    assert_eq!((fact.scope, fact.group_id, fact.short_address), (DaliTargetScope::Group, Some(5), None));
    assert_eq!(fact.source, RuntimeSource::Api);
    assert_eq!(confirmation(&stack).status, DeliveryStatus::Ok);
    assert_eq!(stack.transport.lock().expect("mock lock").sent_frames(), vec![0x8AFF]);
}
