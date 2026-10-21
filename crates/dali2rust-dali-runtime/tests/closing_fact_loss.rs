use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx,
    PublishResult,
};
use dali2rust_contracts::msg::{
    BusCommandPayload, ConfirmationEnvelope, DaliProgramGroupMembershipCommand,
    DaliProgramSceneCommand, DaliProgramTarget, DaliRecallLastActiveLevelCommand,
    DaliRecallSceneCommand, DaliSetTargetStateCommand, DaliTargetScope, DeliveryStatus,
    ErrorCode, GroupMembershipAction, HclTargetScope, LightSetpoint,
    OperationRegistryResetCommand, Origin, PowerState, SceneProgramAction,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::{hold_the_events_ingress_full, try_wait_until};

const REQUEST_CORRELATION: u64 = 5_000;
const OCCUPYING_CORRELATION: u64 = 5_001;
const SHORT: u8 = 9;
const GROUP: u8 = 3;
const SCENE: u8 = 4;
const LEVEL: u8 = 42;
const GEAR_ANSWER: u8 = 0xFF;
const EVENTS_INGRESS: usize = 4;
const WORKER_INBOX: usize = 16;
const CONFIRMATION_TAP: usize = 1;
const STEP_DEADLINE: Duration = Duration::from_secs(10);

type Finished = fn(&DaliWorkerCounters) -> &AtomicU32;

struct Stack {
    _host: BusHost,
    publisher: BusPublisher,
    confirmations: BusSubscriberRx,
    transport: Arc<Mutex<MockDaliTransport>>,
    counters: Arc<DaliWorkerCounters>,
    _worker: JoinHandle<()>,
}

fn spawn_stack() -> Stack {
    let (host, publisher, (worker_commands, confirmations)) = BusHost::spawn(
        BusConfig {
            events_ingress: EVENTS_INGRESS,
            ..BusConfig::default()
        },
        |reg| {
            (
                reg.subscribe_commands(WORKER_INBOX, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_confirmations(CONFIRMATION_TAP),
            )
        },
    );
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    transport.lock().expect("mock").set_persistent_response(GEAR_ANSWER);
    let counters = Arc::new(DaliWorkerCounters::default());
    let worker = spawn_dali_worker(
        worker_commands,
        DaliController::new(Arc::clone(&transport), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::new(RegistryStore::with_adapter_count(1)),
        publisher.clone(),
        BusId::default(),
        Arc::clone(&counters),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    );
    Stack { _host: host, publisher, confirmations, transport, counters, _worker: worker }
}

fn publish_command(publisher: &BusPublisher, correlation_id: u64, payload: BusCommandPayload) {
    let frame = BusFrame::command(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BusId::default().0,
        Some(Origin::Api),
        payload,
    ));
    assert_eq!(publisher.try_publish(BusChannel::Commands, frame), PublishResult::Queued);
}

fn next_confirmation(confirmations: &BusSubscriberRx) -> Option<Arc<ConfirmationEnvelope>> {
    let deadline = Instant::now() + STEP_DEADLINE;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())?;
        if let Ok(BusFrame::Confirmation(confirmation)) = confirmations.recv_timeout(remaining) {
            return Some(confirmation);
        }
    }
}

fn empty_the_occupied_tap(confirmations: &BusSubscriberRx) {
    let occupant = confirmations
        .try_recv()
        .expect("the unhandled command's rejection never reached the confirmation tap");
    let BusFrame::Confirmation(occupant) = occupant else {
        panic!("the confirmation tap held a non-confirmation frame");
    };
    assert_eq!(
        (occupant.meta.correlation_id, occupant.status),
        (OCCUPYING_CORRELATION, DeliveryStatus::DeliveryRejected),
        "the tap was occupied by something other than the unhandled command's rejection"
    );
}

fn hold_the_request_before_its_closing_publish(
    stack: &Stack,
    payload: BusCommandPayload,
) -> dali2rust_test_support::HeldEventsIngress {
    let wire_held = stack.transport.lock().expect("mock");
    publish_command(&stack.publisher, REQUEST_CORRELATION, payload);
    publish_command(
        &stack.publisher,
        OCCUPYING_CORRELATION,
        OperationRegistryResetCommand {}.into(),
    );
    let ingress = hold_the_events_ingress_full(&stack.publisher);
    empty_the_occupied_tap(&stack.confirmations);
    drop(wire_held);
    ingress
}

fn assert_a_lost_closing_fact_fails_the_request(
    payload: BusCommandPayload,
    finished: Option<Finished>,
) {
    let stack = spawn_stack();
    let ingress = hold_the_request_before_its_closing_publish(&stack, payload);
    assert!(
        try_wait_until(
            || stack.counters.event_publish_failed.load(Ordering::Relaxed) > 0,
            STEP_DEADLINE
        ),
        "inconclusive: the events ingress was held full, yet no required publish gave up"
    );
    if let Some(finished) = finished {
        assert!(
            try_wait_until(
                || finished(&stack.counters).load(Ordering::Relaxed) > 0,
                STEP_DEADLINE
            ),
            "the request never finished while the events ingress was held full"
        );
    }
    ingress.release();
    let confirmation = next_confirmation(&stack.confirmations)
        .expect("no confirmation within the deadline after the router was released");
    let error = confirmation.confirmation.error.as_ref().map(|error| error.code);
    assert_eq!(
        (confirmation.meta.correlation_id, confirmation.status, error),
        (REQUEST_CORRELATION, DeliveryStatus::ExecutionFailed, Some(ErrorCode::ExecutionFailed)),
        "the request's closing fact never reached the bus, yet it answered {confirmation:?}"
    );
    assert_eq!(
        stack.counters.execution_failed.load(Ordering::Relaxed),
        1,
        "the request answered ExecutionFailed once, so execution_failed counts it once"
    );
}

fn on_level() -> LightSetpoint {
    LightSetpoint {
        power: PowerState::On,
        level: Some(LEVEL),
        ..Default::default()
    }
}

fn target_state(scope: DaliTargetScope) -> BusCommandPayload {
    DaliSetTargetStateCommand {
        scope,
        group_id: GROUP,
        ..DaliSetTargetStateCommand::for_short(0, SHORT, &on_level())
    }
    .into()
}

#[test]
fn a_lost_applied_fact_fails_a_short_address_request() {
    assert_a_lost_closing_fact_fails_the_request(
        target_state(DaliTargetScope::Short),
        None,
    );
}

#[test]
fn a_lost_applied_fact_fails_a_broadcast_request() {
    assert_a_lost_closing_fact_fails_the_request(
        target_state(DaliTargetScope::Broadcast),
        None,
    );
}

#[test]
fn a_lost_applied_fact_fails_a_group_request() {
    assert_a_lost_closing_fact_fails_the_request(
        target_state(DaliTargetScope::Group),
        None,
    );
}

#[test]
fn a_lost_recalled_fact_fails_a_scene_recall() {
    assert_a_lost_closing_fact_fails_the_request(
        DaliRecallSceneCommand::for_group(0, GROUP, SCENE).into(),
        Some(|counters| &counters.semantic_recall_scene_handled),
    );
}

#[test]
fn a_lost_applied_fact_fails_a_last_active_recall() {
    assert_a_lost_closing_fact_fails_the_request(
        DaliRecallLastActiveLevelCommand {
            registry_adapter_id: 0,
            scope: HclTargetScope::Group,
            group_id: Some(GROUP),
        }
        .into(),
        Some(|counters| &counters.semantic_recall_last_active_level_handled),
    );
}

#[test]
fn a_lost_programmed_fact_fails_a_scene_programming() {
    assert_a_lost_closing_fact_fails_the_request(
        DaliProgramSceneCommand {
            registry_adapter_id: 0,
            target: DaliProgramTarget::Short { short_address: SHORT },
            scene_id: SCENE,
            action: SceneProgramAction::Clear,
            target_state: None,
        }
        .into(),
        Some(|counters| &counters.semantic_program_scene_handled),
    );
}

#[test]
fn a_lost_programmed_fact_fails_a_group_membership_programming() {
    assert_a_lost_closing_fact_fails_the_request(
        DaliProgramGroupMembershipCommand {
            registry_adapter_id: 0,
            target: DaliProgramTarget::Short { short_address: SHORT },
            group_id: GROUP,
            action: GroupMembershipAction::Add,
        }
        .into(),
        Some(|counters| &counters.semantic_program_group_membership_handled),
    );
}
