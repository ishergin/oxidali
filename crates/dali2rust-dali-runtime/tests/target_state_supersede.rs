use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{
    DaliTargetScope, DeliveryStatus, ErrorCode, LightSetpoint, PowerState,
};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_contracts::msg::BusEventPayload;
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::{recv_event_matching, wait_until};

const SHORT: u8 = 5;

fn dapc_frame(level: u8) -> u16 {
    DaliCommand::Standard {
        address: DaliAddress::short(SHORT).expect("short"),
        command: StandardCommand::DirectArcPower { level },
    }
    .to_forward_frame()
    .raw()
}

fn target_state_frame(correlation_id: u64, level: u8) -> BusFrame {
    let setpoint = LightSetpoint {
        power: PowerState::On,
        level,
        color: None,
    };
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        None,
        dali2rust_contracts::msg::DaliSetTargetStateCommand {
            registry_adapter_id: 0,
            scope: DaliTargetScope::Short,
            short_address: SHORT,
            virtual_lamp_id: 0,
            group_id: 0,
            setpoint,
        },
    ))
}

#[test]
fn stale_target_state_commands_are_superseded_within_one_drain() {
    let (host, publisher, (worker_cmd, event_obs, conf_obs)) =
        BusHost::spawn(BusConfig::default(), |reg| {
            (
                reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_events(16, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
                reg.subscribe_confirmations(16),
            )
        });

    for (corr, level) in [(11u64, 100u8), (12, 150), (13, 220)] {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, target_state_frame(corr, level)),
            PublishResult::Queued
        );
    }
    wait_until(
        || {
            host.counters_snapshot()
                .command_subscribers
                .first()
                .map(|sub| sub.delivered == 3)
                .unwrap_or(false)
        },
        Duration::from_secs(1),
    );

    let transport = Arc::new(std::sync::Mutex::new(MockDaliTransport::new()));
    transport
        .lock()
        .expect("mock lock")
        .expect_forward_frame(dapc_frame(220));
    let controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
    let read_port = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_dali_worker(
        worker_cmd,
        controller,
        DaliRuntimeConfig::default(),
        read_port,
        publisher.clone(),
        BusId::default(),
        Arc::clone(&counters),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    );

    let conf_rx = conf_obs;
    let mut superseded = Vec::new();
    for _ in 0..2 {
        let BusFrame::Confirmation(conf) = conf_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("confirmation")
        else {
            panic!("expected confirmation frame");
        };
        assert_ne!(conf.status, DeliveryStatus::Ok);
        assert_eq!(
            conf.confirmation.error.as_ref().map(|e| e.code),
            Some(ErrorCode::Superseded),
            "displaced commands answer superseded: {conf:?}"
        );
        superseded.push(conf.meta.correlation_id);
    }
    superseded.sort_unstable();
    assert_eq!(superseded, vec![11, 12], "older correlations are displaced");

    let applied_ev = recv_event_matching(&event_obs, Duration::from_secs(1), |payload| {
        matches!(payload, BusEventPayload::DaliTargetStateAppliedEvent(_))
    });
    let BusEventPayload::DaliTargetStateAppliedEvent(body) = &applied_ev.payload else {
        unreachable!("predicate selected this variant");
    };
    let applied = (applied_ev.meta.correlation_id, body.clone());
    assert_eq!(applied.0, 13, "newest correlation wins");
    assert_eq!(applied.1.setpoint.level, 220);
    assert_eq!(applied.1.short_address, Some(SHORT));

    assert_eq!(
        counters.target_state_superseded.load(Relaxed),
        2,
        "both displaced commands are counted"
    );
    assert_eq!(
        counters.commands_handled.load(Relaxed),
        1,
        "only the surviving command reached the dispatch table"
    );

    let mock = transport.lock().expect("mock lock");
    assert_eq!(
        mock.scripted_exchanges_remaining(),
        0,
        "only the winning setpoint touched the wire: {:?}",
        mock.sent_frames()
    );
    assert_eq!(mock.script_error(), None);
}
