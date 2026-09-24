use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, PublishResult, MAX_BUS_FRAME_BYTES};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_contracts::bus::{build_confirmation_envelope_with_product_error, command_envelope, event_envelope};
use dali2rust_contracts::msg::{
    CommandEnvelope, DeliveryStatus, ErrorCode, ErrorPayload, EventEnvelope,
};

fn meta_error(msg_len: usize, details_len: usize, fill: u8) -> ErrorPayload {
    ErrorPayload::with_details(
        ErrorCode::UnsupportedCapability,
        "e".repeat(msg_len),
        &vec![fill; details_len],
    )
}

fn command_envelope_with_meta_error(msg_len: usize, details_len: usize) -> CommandEnvelope {
    let mut env = command_envelope(u16::MAX, u64::MAX, u16::MAX, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliCommandPayload { wire_address: 1, command: 2, repeat_count: 1, raw_mode: false, raw_expects_backward: false });
    env.meta.error = Some(Box::new(meta_error(msg_len, details_len, 0xAB)));
    env
}

fn command_wire_len(msg_len: usize, details_len: usize) -> usize {
    BusFrame::command(command_envelope_with_meta_error(msg_len, details_len))
        .postcard_wire_len()
        .expect("postcard command wire len")
}

fn command_frame_with_lengths(msg_len: usize, details_len: usize) -> BusFrame {
    BusFrame::command(command_envelope_with_meta_error(msg_len, details_len))
}

fn command_frame_at_least_wire_bytes(min_len: usize) -> BusFrame {
    for msg_len in 0..=64 {
        for details_len in 0..=32 {
            if command_wire_len(msg_len, details_len) >= min_len {
                return command_frame_with_lengths(msg_len, details_len);
            }
        }
    }
    panic!("wire len {min_len} unreachable");
}

fn command_frame_exact_wire_bytes(target: usize) -> BusFrame {
    assert!(target > 0);
    for msg_len in 0..=64 {
        for details_len in 0..=32 {
            if command_wire_len(msg_len, details_len) == target {
                return command_frame_with_lengths(msg_len, details_len);
            }
        }
    }
    panic!("exact wire len {target} not reachable");
}

fn confirmation_wire_len(msg_len: usize, details_len: usize) -> usize {
    confirmation_frame_with_lengths(msg_len, details_len)
        .postcard_wire_len()
        .expect("postcard confirmation wire len")
}

fn confirmation_frame_with_lengths(msg_len: usize, details_len: usize) -> BusFrame {
    let msg = "e".repeat(msg_len);
    let mut env = build_confirmation_envelope_with_product_error(
        1,
        DeliveryStatus::ExecutionFailed,
        0,
        SOURCE_ID_UNSPECIFIED,
        Some((ErrorCode::UnsupportedCapability, &msg)),
    );
    env.confirmation.error = Some(ErrorPayload::with_details(
        ErrorCode::UnsupportedCapability,
        msg,
        &vec![0xAB; details_len],
    ));
    env.meta.error = Some(Box::new(meta_error(msg_len, details_len, 0xBC)));
    BusFrame::confirmation(env)
}

fn confirmation_frame_at_least_wire_bytes(min_len: usize) -> BusFrame {
    for msg_len in 0..=64 {
        for details_len in 0..=32 {
            if confirmation_wire_len(msg_len, details_len) >= min_len {
                return confirmation_frame_with_lengths(msg_len, details_len);
            }
        }
    }
    panic!("wire len {min_len} unreachable");
}

fn event_envelope_with_meta_error(msg_len: usize, details_len: usize) -> EventEnvelope {
    let mut env = event_envelope(
        u16::MAX,
        u64::MAX,
        u16::MAX,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::IpAddressAssignedEvent { ip_v4: [192, 168, 0, 1] },
    );
    env.meta.error = Some(Box::new(meta_error(msg_len, details_len, 0xCD)));
    env
}

fn event_wire_len(msg_len: usize, details_len: usize) -> usize {
    BusFrame::event(event_envelope_with_meta_error(msg_len, details_len))
        .postcard_wire_len()
        .expect("postcard event wire len")
}

fn event_frame_at_least_wire_bytes(min_len: usize) -> BusFrame {
    for msg_len in 0..=64 {
        for details_len in 0..=32 {
            if event_wire_len(msg_len, details_len) >= min_len {
                return BusFrame::event(event_envelope_with_meta_error(msg_len, details_len));
            }
        }
    }
    panic!("wire len {min_len} unreachable");
}

#[test]
fn oversized_command_rejected_bus022_bus023() {
    let (host, publisher, ()) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            let _ = reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
        },
    );
    let f = command_frame_at_least_wire_bytes(MAX_BUS_FRAME_BYTES + 1);
    assert!(f.postcard_wire_len().unwrap() > MAX_BUS_FRAME_BYTES);
    let r = publisher.try_publish(BusChannel::Commands, f);
    assert_eq!(r, PublishResult::RejectedFrameTooLarge);
    let c = host.counters_snapshot();
    assert_eq!(c.commands.oversize_rejected, 1);
    assert_eq!(c.commands.publish_queued, 0);
}

#[test]
fn command_at_max_wire_accepted_bus024() {
    let (_host, publisher, ()) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            let _ = reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
        },
    );
    let f = command_frame_exact_wire_bytes(MAX_BUS_FRAME_BYTES);
    assert_eq!(f.postcard_wire_len().unwrap(), MAX_BUS_FRAME_BYTES);
    let r = publisher.try_publish(BusChannel::Commands, f);
    assert_eq!(r, PublishResult::Queued);
}

#[test]
fn minimal_command_frame_accepted_bus025() {
        let (_host, publisher, ()) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            let _ = reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
        },
    );
    let f = BusFrame::command(dali2rust_contracts::bus::command_envelope(0, 1, 0, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: 1, command: 2, repeat_count: 1, raw_mode: false, raw_expects_backward: false }));
    let r = publisher.try_publish(BusChannel::Commands, f);
    assert_eq!(r, PublishResult::Queued);
}

#[test]
fn oversized_confirmation_rejected_bus026() {
    let (host, publisher, ()) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            let _ = reg.subscribe_confirmations(4);
        },
    );
    let f = confirmation_frame_at_least_wire_bytes(MAX_BUS_FRAME_BYTES + 1);
    assert!(f.postcard_wire_len().unwrap() > MAX_BUS_FRAME_BYTES);
    let r = publisher.try_publish(BusChannel::Confirmations, f);
    assert_eq!(r, PublishResult::RejectedFrameTooLarge);
    assert_eq!(host.counters_snapshot().confirmations.oversize_rejected, 1);
}

#[test]
fn oversized_event_rejected_bus027() {
    let (host, publisher, ()) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            let _ = reg.subscribe_events(4, dali2rust_contracts::msg::EVENT_VARIANT_NAMES);
        },
    );
    let f = event_frame_at_least_wire_bytes(MAX_BUS_FRAME_BYTES + 1);
    assert!(f.postcard_wire_len().unwrap() > MAX_BUS_FRAME_BYTES);
    let r = publisher.try_publish(BusChannel::Events, f);
    assert_eq!(r, PublishResult::RejectedFrameTooLarge);
    assert_eq!(host.counters_snapshot().events.oversize_rejected, 1);
}

#[test]
fn oversize_counts_as_publish_attempt_bus028() {
    let (host, publisher, ()) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            let _ = reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
        },
    );
    let f = command_frame_at_least_wire_bytes(MAX_BUS_FRAME_BYTES + 1);
    let _ = publisher.try_publish(BusChannel::Commands, f);
    let c = host.counters_snapshot();
    assert_eq!(c.commands.publish_attempted, 1);
    assert_eq!(c.commands.oversize_rejected, 1);
}
