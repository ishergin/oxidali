use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, PublishResult};
use dali2rust_contracts::msg::DeliveryStatus;
use dali2rust_test_support::{command_frame, wait_until};


#[test]
fn owner_receives_routed_command_and_non_owner_does_not() {
    let (host, publisher, (owner, other)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands(4, &["DaliCommandPayload"]),
            reg.subscribe_commands(4, &["OperationBeginCommand"]),
        )
    });

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    let frame = owner
        .recv_timeout(Duration::from_millis(500))
        .expect("owner receives the routed command");
    assert!(matches!(frame, BusFrame::Command(_)));
    assert!(
        other.try_recv().is_err(),
        "non-owner must not receive the command"
    );
    wait_until(
        || host.counters_snapshot().command_subscribers[0].delivered == 1,
        Duration::from_millis(500),
    );
    let c = host.counters_snapshot();
    assert_eq!(c.command_subscribers[0].delivered, 1);
    assert_eq!(c.command_subscribers[1].delivered, 0);
    assert_eq!(c.commands_unrouted, 0);
}


#[test]
fn unrouted_command_is_counted_and_rejected() {
    let (host, publisher, (other, conf)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands(4, &["OperationBeginCommand"]),
            reg.subscribe_confirmations(8),
        )
    });

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    let cf = conf
        .recv_timeout(Duration::from_millis(500))
        .expect("synthetic rejection for an unrouted command");
    let BusFrame::Confirmation(ce) = cf else {
        panic!("expected confirmation frame");
    };
    assert_eq!(ce.status, DeliveryStatus::DeliveryRejected);
    assert!(other.try_recv().is_err());
    wait_until(
        || host.counters_snapshot().commands_unrouted >= 1,
        Duration::from_millis(500),
    );
    assert!(host.counters_snapshot().commands_unrouted >= 1);
}


#[test]
fn overlapping_handled_sets_deliver_to_all_declaring_subscribers() {
    let (host, publisher, (a, b)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands(4, &["DaliCommandPayload"]),
            reg.subscribe_commands(4, &["DaliCommandPayload"]),
        )
    });

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    assert!(a.recv_timeout(Duration::from_millis(500)).is_ok());
    assert!(b.recv_timeout(Duration::from_millis(500)).is_ok());
    wait_until(
        || host.counters_snapshot().command_subscribers[1].delivered == 1,
        Duration::from_millis(500),
    );
    let c = host.counters_snapshot();
    assert_eq!(c.command_subscribers[0].delivered, 1);
    assert_eq!(c.command_subscribers[1].delivered, 1);
}

#[test]
fn no_rejection_when_one_of_two_declared_subscribers_accepts() {
    let (host, publisher, (_full, open, conf)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands(1, &["DaliCommandPayload"]),
            reg.subscribe_commands(16, &["DaliCommandPayload"]),
            reg.subscribe_confirmations(8),
        )
    });

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    assert!(open.recv_timeout(Duration::from_millis(500)).is_ok());
    assert!(open.recv_timeout(Duration::from_millis(500)).is_ok());
    wait_until(
        || host.counters_snapshot().command_subscribers[0].receiver_overflow >= 1,
        Duration::from_millis(500),
    );
    assert!(
        conf.try_recv().is_err(),
        "no synthetic rejection while a declared sibling accepted the frame"
    );
}


#[test]
#[should_panic(expected = "subscribe_commands: NoSuchCommandVariant is not a payload variant")]
fn unknown_handled_name_panics_at_spawn() {
    let _ = BusHost::spawn(BusConfig::default(), |reg| {
        let _ = reg.subscribe_commands(4, &["NoSuchCommandVariant"]);
    });
}

#[test]
fn synthetic_rejections_survive_conf_ingress_saturation() {
    const BURST: usize = 32;
    let config = BusConfig {
        confirmations_ingress: 2,
        ..BusConfig::default()
    };
    let (host, publisher, (_other, conf)) = BusHost::spawn(config, |reg| {
        (
            reg.subscribe_commands(4, &["OperationBeginCommand"]),
            reg.subscribe_confirmations(BURST * 2),
        )
    });

    for _ in 0..BURST {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, command_frame()),
            PublishResult::Queued
        );
    }
    for i in 0..BURST {
        let frame = conf
            .recv_timeout(Duration::from_millis(500))
            .unwrap_or_else(|_| panic!("synthetic rejection {i} lost"));
        let BusFrame::Confirmation(ce) = frame else {
            panic!("expected confirmation frame");
        };
        assert_eq!(ce.status, DeliveryStatus::DeliveryRejected);
    }
    wait_until(
        || host.counters_snapshot().commands_unrouted >= BURST as u32,
        Duration::from_millis(500),
    );
    let c = host.counters_snapshot();
    assert_eq!(c.commands_unrouted, BURST as u32);
    assert_eq!(c.delivery_rejected_dropped, 0);
}
