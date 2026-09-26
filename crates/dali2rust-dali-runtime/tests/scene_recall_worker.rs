use std::sync::Arc;
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{BusEventPayload, DeliveryStatus, ErrorCode};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::wait_until;

fn go_to_scene_frame(scene: u8) -> u16 {
    DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::GoToScene { scene },
    }
    .to_forward_frame()
    .raw()
}

fn recall_frame(correlation_id: u64, scene_id: u8) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        None,
        dali2rust_contracts::msg::DaliRecallSceneCommand::broadcast(0, scene_id),
    ))
}

type ScriptedWorker = (
    dali2rust_bus::BusPublisher,
    dali2rust_bus::BusHost,
    std::sync::mpsc::Receiver<BusFrame>,
    std::sync::mpsc::Receiver<BusFrame>,
    Arc<std::sync::Mutex<MockDaliTransport>>,
    dali2rust_bus::BusSubscriberRx,
);

fn spawn_worker_with_scripted_frames(scripted: &[u16]) -> ScriptedWorker {
    let (host, publisher, (worker_cmd, conf_obs, ev_obs)) =
        BusHost::spawn(BusConfig::default(), |reg| {
            (
                reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_confirmations(16),
                reg.subscribe_events(16, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            )
        });
    let transport = Arc::new(std::sync::Mutex::new(MockDaliTransport::new()));
    {
        let mock = transport.lock().expect("mock lock");
        for frame in scripted {
            mock.expect_forward_frame(*frame);
        }
    }
    (publisher, host, conf_obs, ev_obs, transport, worker_cmd)
}

fn spawn_worker(
    worker_cmd: dali2rust_bus::BusSubscriberRx,
    publisher: dali2rust_bus::BusPublisher,
    transport: &Arc<std::sync::Mutex<MockDaliTransport>>,
) -> std::thread::JoinHandle<()> {
    let controller = DaliController::new(Arc::clone(transport), Box::new(StdClock::new()));
    spawn_dali_worker(
        worker_cmd,
        controller,
        DaliRuntimeConfig::default(),
        Arc::new(RegistryStore::with_adapter_count(1)),
        publisher,
        BusId::default(),
        Arc::new(DaliWorkerCounters::default()),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    )
}

#[test]
fn recall_sends_one_broadcast_frame_and_publishes_event() {
    let (publisher, _host, conf_obs, ev_obs, transport, worker_cmd) =
        spawn_worker_with_scripted_frames(&[go_to_scene_frame(3)]);
    let _worker = spawn_worker(worker_cmd, publisher.clone(), &transport);

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, recall_frame(21, 3)),
        PublishResult::Queued
    );

    let BusFrame::Confirmation(conf) = conf_obs
        .recv_timeout(Duration::from_secs(1))
        .expect("confirmation")
    else {
        panic!("expected confirmation frame");
    };
    assert_eq!(conf.meta.correlation_id, 21);
    assert_eq!(conf.status, DeliveryStatus::Ok);

    let mut saw_recalled = false;
    for _ in 0..8 {
        let Ok(BusFrame::Event(ev)) = ev_obs.recv_timeout(Duration::from_millis(200)) else {
            break;
        };
        if let BusEventPayload::DaliSceneRecalledEvent(body) = &ev.payload {
            assert_eq!(body.scene_id, 3);
            assert!(body.error.is_none());
            saw_recalled = true;
            break;
        }
    }
    assert!(saw_recalled, "expected DaliSceneRecalledEvent");

    let mock = transport.lock().expect("mock lock");
    assert_eq!(mock.sent_frames(), vec![go_to_scene_frame(3)]);
    assert_eq!(mock.script_error(), None);
}

#[test]
fn newer_recall_of_same_scene_supersedes_queued_one() {
    let (publisher, host, conf_obs, _ev_obs, transport, worker_cmd) =
        spawn_worker_with_scripted_frames(&[go_to_scene_frame(3)]);

    for corr in [31u64, 32] {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, recall_frame(corr, 3)),
            PublishResult::Queued
        );
    }
    wait_until(
        || {
            host.counters_snapshot()
                .command_subscribers
                .first()
                .map(|sub| sub.delivered == 2)
                .unwrap_or(false)
        },
        Duration::from_secs(1),
    );
    let _worker = spawn_worker(worker_cmd, publisher.clone(), &transport);

    let BusFrame::Confirmation(first) = conf_obs
        .recv_timeout(Duration::from_secs(1))
        .expect("superseded confirmation")
    else {
        panic!("expected confirmation frame");
    };
    assert_eq!(first.meta.correlation_id, 31);
    assert_eq!(
        first.confirmation.error.as_ref().map(|e| e.code),
        Some(ErrorCode::Superseded)
    );

    let BusFrame::Confirmation(winner) = conf_obs
        .recv_timeout(Duration::from_secs(1))
        .expect("winner confirmation")
    else {
        panic!("expected confirmation frame");
    };
    assert_eq!(winner.meta.correlation_id, 32);
    assert_eq!(winner.status, DeliveryStatus::Ok);

    let mock = transport.lock().expect("mock lock");
    assert_eq!(
        mock.sent_frames(),
        vec![go_to_scene_frame(3)],
        "one GoToScene for the winning recall only"
    );
}

#[test]
fn recalls_of_different_scenes_execute_fifo() {
    let (publisher, host, conf_obs, _ev_obs, transport, worker_cmd) =
        spawn_worker_with_scripted_frames(&[go_to_scene_frame(1), go_to_scene_frame(2)]);

    for (corr, scene) in [(41u64, 1u8), (42, 2)] {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, recall_frame(corr, scene)),
            PublishResult::Queued
        );
    }
    wait_until(
        || {
            host.counters_snapshot()
                .command_subscribers
                .first()
                .map(|sub| sub.delivered == 2)
                .unwrap_or(false)
        },
        Duration::from_secs(1),
    );
    let _worker = spawn_worker(worker_cmd, publisher.clone(), &transport);

    for corr in [41u64, 42] {
        let BusFrame::Confirmation(conf) = conf_obs
            .recv_timeout(Duration::from_secs(1))
            .expect("confirmation")
        else {
            panic!("expected confirmation frame");
        };
        assert_eq!(conf.meta.correlation_id, corr);
        assert_eq!(conf.status, DeliveryStatus::Ok);
    }

    let mock = transport.lock().expect("mock lock");
    assert_eq!(
        mock.sent_frames(),
        vec![go_to_scene_frame(1), go_to_scene_frame(2)],
        "different scenes do not coalesce and run FIFO"
    );
}
