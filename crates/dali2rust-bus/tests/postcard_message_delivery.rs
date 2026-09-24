use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, PublishResult};
use dali2rust_contracts::msg::DeliveryStatus;
use dali2rust_test_support::command_frame;

#[test]
fn owner_inbox_full_emits_synthetic_rejection_bus015() {
    let (_host, publisher, (_owner, other_rx, conf_rx)) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            (
                reg.subscribe_commands(1, &["DaliCommandPayload"]),
                reg.subscribe_commands(16, &["OperationBeginCommand"]),
                reg.subscribe_confirmations(8),
            )
        },
    );
    let f = command_frame();
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, f.clone()),
        PublishResult::Queued
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, f),
        PublishResult::Queued
    );
    let cf = conf_rx
        .recv_timeout(Duration::from_millis(500))
        .expect("synthetic confirmation");
    let BusFrame::Confirmation(ce) = cf else {
        panic!("expected confirmation");
    };
    assert_eq!(ce.status, DeliveryStatus::DeliveryRejected);
    assert!(
        other_rx.try_recv().is_err(),
        "non-owner must not receive a routed command"
    );
}
