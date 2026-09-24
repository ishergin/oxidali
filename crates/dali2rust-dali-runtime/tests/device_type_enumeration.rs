mod support;

use support::{
    dt8_read_frame, interactive_target_state_frame, spawn_mock_worker, standard_frame,
    EVENT_BUDGET,
};

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{AttributeGroupReadOutcome, BusEventPayload};
use dali2rust_dali_runtime::{
    DaliWorkerCounters, WireArrivalObserver, DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_test_support::{recv_event_matching, try_recv_event_matching_envelope, wait_until};

fn dt8_outcome(events: &dali2rust_bus::BusSubscriberRx) -> AttributeGroupReadOutcome {
    let event = recv_event_matching(events, EVENT_BUDGET, |payload| {
        matches!(payload, BusEventPayload::DaliAttributeReadOutcomesEvent(_))
    });
    let BusEventPayload::DaliAttributeReadOutcomesEvent(body) = &event.payload else {
        unreachable!("predicate selected this variant");
    };
    body.dt8_color
}

#[test]
fn a_mid_walk_arrival_waits_out_the_bracket_and_never_truncates() {
    let interactive = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let (_host, publisher, (worker_cmd, events)) = BusHost::spawn_with_command_observer(
        BusConfig::default(),
        Some(Arc::new(WireArrivalObserver::new(
            Arc::clone(&interactive),
            BusId::default(),
        ))),
        |reg| {
            (
                reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_events(64, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            )
        },
    );

    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_with_backward(
        standard_frame(StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(StandardCommand::QueryDeviceType),
        Some(255),
    );
    for answer in [6u8, 8, 254] {
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QueryNextDeviceType),
            Some(answer),
        );
    }
    let unblock = mock.get_unblock_flag();
    mock.block_send_at(2);
    let transport = Arc::new(std::sync::Mutex::new(mock));

    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_mock_worker(
        worker_cmd,
        &transport,
        &publisher,
        &counters,
        Arc::clone(&interactive),
    );

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, dt8_read_frame(71)),
        PublishResult::Queued
    );
    wait_until(
        || {
            transport
                .lock()
                .map(|mock| mock.sent_frames().len() == 2)
                .unwrap_or(false)
        },
        Duration::from_secs(2),
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, interactive_target_state_frame(81)),
        PublishResult::Queued
    );
    assert!(
        !interactive.quiet_for(Duration::from_secs(5)),
        "the published command must already count as an arrival"
    );
    unblock.store(false, Ordering::Release);

    assert_eq!(
        dt8_outcome(&events),
        AttributeGroupReadOutcome::Preempted,
        "the section the yield displaced says so"
    );
    assert!(
        try_recv_event_matching_envelope(&events, EVENT_BUDGET, |ev| {
            ev.meta.correlation_id == 81
                && matches!(ev.payload, BusEventPayload::DaliTargetStateAppliedEvent(_))
        })
        .is_some(),
        "the operator's setpoint must be applied after the yield"
    );

    let frames = transport.lock().expect("mock lock").sent_frames();
    let walk: Vec<u16> = vec![
        standard_frame(StandardCommand::QueryControlGearPresent),
        standard_frame(StandardCommand::QueryDeviceType),
        standard_frame(StandardCommand::QueryNextDeviceType),
        standard_frame(StandardCommand::QueryNextDeviceType),
        standard_frame(StandardCommand::QueryNextDeviceType),
    ];
    assert_eq!(
        &frames[..5],
        walk.as_slice(),
        "every walk frame stays contiguous through 254 — the arrival could not land inside"
    );
    assert!(
        frames[5..].contains(&standard_frame(StandardCommand::DirectArcPower { level: 220 })),
        "the operator's frame follows the complete walk: {frames:?}"
    );
    assert_eq!(
        counters.read_attributes_preempted.load(Ordering::Relaxed),
        1,
        "one honest preemption, not a transport fault"
    );
}
