use std::sync::Arc;
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{
    DaliRecallLastActiveLevelCommand, DeliveryStatus, ErrorCode, HclTargetScope,
};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_registry_runtime::RegistryStore;

fn last_active_frame(address: DaliAddress) -> u16 {
    DaliCommand::Standard {
        address,
        command: StandardCommand::GoToLastActiveLevel,
    }
    .to_forward_frame()
    .raw()
}

fn recall_frame(correlation_id: u64, scope: HclTargetScope, group_id: Option<u8>) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        None,
        DaliRecallLastActiveLevelCommand {
            registry_adapter_id: 0,
            scope,
            group_id,
        },
    ))
}

type Stack = (
    dali2rust_bus::BusPublisher,
    dali2rust_bus::BusHost,
    std::sync::mpsc::Receiver<BusFrame>,
    std::sync::mpsc::Receiver<BusFrame>,
    Arc<std::sync::Mutex<MockDaliTransport>>,
    std::thread::JoinHandle<()>,
);

fn spawn_stack_with_events(scripted: &[u16]) -> Stack {
    let (host, publisher, (worker_cmd, conf_obs, ev_obs)) =
        BusHost::spawn(BusConfig::default(), |reg| {
            (
                reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_confirmations(16),
                reg.subscribe_events(16, &["DaliTargetStateAppliedEvent"]),
            )
        });
    let transport = Arc::new(std::sync::Mutex::new(MockDaliTransport::new()));
    {
        let mock = transport.lock().expect("mock lock");
        for frame in scripted {
            mock.expect_forward_frame(*frame);
        }
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
    (publisher, host, conf_obs, ev_obs, transport, worker)
}

fn spawn_stack(
    scripted: &[u16],
) -> (
    dali2rust_bus::BusPublisher,
    dali2rust_bus::BusHost,
    std::sync::mpsc::Receiver<BusFrame>,
    Arc<std::sync::Mutex<MockDaliTransport>>,
    std::thread::JoinHandle<()>,
) {
    let (publisher, host, conf_obs, _ev, transport, worker) = spawn_stack_with_events(scripted);
    (publisher, host, conf_obs, transport, worker)
}

fn recv_confirmation(
    rx: &std::sync::mpsc::Receiver<BusFrame>,
) -> Arc<dali2rust_contracts::msg::ConfirmationEnvelope> {
    let BusFrame::Confirmation(conf) = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("confirmation")
    else {
        panic!("expected confirmation frame");
    };
    conf
}

#[test]
fn broadcast_recall_sends_one_indirect_last_active_frame() {
    let expected = last_active_frame(DaliAddress::Broadcast);
    let (publisher, _host, conf_obs, transport, _worker) = spawn_stack(&[expected]);

    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            recall_frame(51, HclTargetScope::Broadcast, None)
        ),
        PublishResult::Queued
    );

    let conf = recv_confirmation(&conf_obs);
    assert_eq!(conf.meta.correlation_id, 51);
    assert_eq!(conf.status, DeliveryStatus::Ok);

    let mock = transport.lock().expect("mock lock");
    assert_eq!(mock.sent_frames(), vec![expected]);
    assert_eq!(expected, 0xFF0A);
    assert_eq!(mock.script_error(), None);
}

#[test]
fn group_recall_addresses_the_named_group() {
    let expected = last_active_frame(DaliAddress::Group(5));
    let (publisher, _host, conf_obs, transport, _worker) = spawn_stack(&[expected]);

    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            recall_frame(52, HclTargetScope::Group, Some(5))
        ),
        PublishResult::Queued
    );

    let conf = recv_confirmation(&conf_obs);
    assert_eq!(conf.meta.correlation_id, 52);
    assert_eq!(conf.status, DeliveryStatus::Ok);

    let mock = transport.lock().expect("mock lock");
    assert_eq!(mock.sent_frames(), vec![expected]);
    assert_eq!(expected, 0x8B0A);
}

#[test]
fn group_scope_without_group_id_fails_before_the_wire() {
    let (publisher, _host, conf_obs, transport, _worker) = spawn_stack(&[]);

    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            recall_frame(53, HclTargetScope::Group, None)
        ),
        PublishResult::Queued
    );

    let conf = recv_confirmation(&conf_obs);
    assert_eq!(conf.meta.correlation_id, 53);
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    let error = conf.confirmation.error.as_ref().expect("product error");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.message.as_str(), "missing_group_id");

    let mock = transport.lock().expect("mock lock");
    assert!(
        mock.sent_frames().is_empty(),
        "an unaddressable target must not reach the bus"
    );
}

#[test]
fn group_id_above_fifteen_fails_before_the_wire() {
    let (publisher, _host, conf_obs, transport, _worker) = spawn_stack(&[]);

    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            recall_frame(54, HclTargetScope::Group, Some(16))
        ),
        PublishResult::Queued
    );

    let conf = recv_confirmation(&conf_obs);
    let error = conf.confirmation.error.as_ref().expect("product error");
    assert_eq!(error.message.as_str(), "invalid_group_id");

    let mock = transport.lock().expect("mock lock");
    assert!(mock.sent_frames().is_empty());
}

#[test]
fn a_group_recall_publishes_an_applied_fact_with_no_level() {
    let expected = last_active_frame(DaliAddress::Group(5));
    let (publisher, _host, conf_obs, ev_obs, _transport, _worker) =
        spawn_stack_with_events(&[expected]);

    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            recall_frame(55, HclTargetScope::Group, Some(5))
        ),
        PublishResult::Queued
    );
    let conf = recv_confirmation(&conf_obs);
    assert_eq!(conf.status, DeliveryStatus::Ok);

    let BusFrame::Event(ev) = ev_obs
        .recv_timeout(Duration::from_secs(1))
        .expect("the recall must publish an applied fact")
    else {
        panic!("expected an event frame");
    };
    let dali2rust_contracts::msg::BusEventPayload::DaliTargetStateAppliedEvent(applied) =
        &ev.payload
    else {
        panic!("subscribed to one kind");
    };
    assert_eq!(applied.scope, dali2rust_contracts::msg::DaliTargetScope::Group);
    assert_eq!(applied.group_id, Some(5));
    assert_eq!(
        applied.setpoint.power,
        dali2rust_contracts::msg::PowerState::On
    );
    assert_eq!(
        applied.setpoint.level, 0,
        "a level stated here would be one number for every member"
    );
    assert!(applied.setpoint.color.is_none());
    assert!(
        !applied.dapc_applied,
        "opcode 10 is not a DAPC, and `last_dapc_source` must not say it was"
    );
}
