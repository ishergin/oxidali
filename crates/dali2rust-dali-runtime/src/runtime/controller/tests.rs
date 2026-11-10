use super::*;
use core::sync::atomic::{AtomicU64, Ordering};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::commands::{ExtendedCommand, SpecialCommand, StandardCommand};
use dali2rust_domain::dali::types::DaliAddress;
use crate::runtime::executor::test_helpers::shared::{assert_script_consumed, priorities_of};
use dali2rust_test_support::{AdvancingClock, StoppedClock};
use std::time::Instant;
use super::send_twice::STRAY_HALF_LIFETIME_MS;
use dali2rust_domain::dali::devices::dt6_led::Dt6Command;
use dali2rust_domain::dali::devices::dt8_color::Dt8Command;

fn test_clock() -> Box<dyn Clock> {
    Box::new(AdvancingClock::new(20))
}

fn retry_policy(max_attempts: u8) -> RetryPolicy {
    RetryPolicy::new(max_attempts, 0, 0)
}

fn frame_lease(gate: &Arc<dali2rust_platform::dali::WireActivity>) -> WireLease {
    WireLease::new(
        Arc::clone(gate),
        dali2rust_platform::dali::WirePriority::Attended,
        YieldGranularity::Frame,
    )
}

fn step_lease(gate: &Arc<dali2rust_platform::dali::WireActivity>) -> WireLease {
    WireLease::new(
        Arc::clone(gate),
        dali2rust_platform::dali::WirePriority::Attended,
        YieldGranularity::Step,
    )
}

fn retry_policy_with_contention(max_attempts: u8, enabled: bool) -> RetryPolicy {
    retry_policy(max_attempts).with_query_contention_retry(enabled)
}

fn query_status_command(short: u8) -> DaliCommand {
    DaliCommand::Standard {
        address: DaliAddress::short(short).unwrap(),
        command: StandardCommand::QueryStatus,
    }
}

fn query_status_frame(short: u8) -> u16 {
    query_status_command(short).to_forward_frame().raw()
}

#[test]
fn controller_send_simple_command() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x42);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(transport, test_clock());

    let cmd = DaliCommand::Standard {
        address: DaliAddress::short(1).unwrap(),
        command: StandardCommand::QueryStatus,
    };

    let response = controller.send_command(&cmd).unwrap();
    assert_eq!(response, DaliResponse::Answer(0x42));
}

#[test]
fn controller_send_raw_no_backward() {
    let mock = MockDaliTransport::new();
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(transport, test_clock());
    let frame = ForwardFrame::new(0x02, 0xFE);

    let result = controller.send_raw(frame, false).unwrap();
    assert_eq!(result, DaliResponse::NoAnswer);
}

#[test]
fn controller_session_references_available() {
    let mock = MockDaliTransport::new();
    let transport = Arc::new(Mutex::new(mock));

    let controller = DaliController::new(transport, test_clock());
    let _ = controller.session();
}

#[test]
fn controller_repeat_command_sends_twice() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x00);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(transport, test_clock());

    let cmd = DaliCommand::Standard {
        address: DaliAddress::short(0).unwrap(),
        command: StandardCommand::Reset,
    };

    let _ = controller.send_command(&cmd);
    let guard = controller.transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![0x0120, 0x0120]);
    assert_eq!(
        priorities_of(&guard.sent_frame_settle_us()),
        vec![DaliPriority::Configuration, DaliPriority::Transaction]
    );
}

#[test]
fn no_frame_of_a_new_command_opens_at_priority_one() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x00);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(transport, test_clock());

    let reset = DaliCommand::Standard {
        address: DaliAddress::short(0).unwrap(),
        command: StandardCommand::Reset,
    };
    let off = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::Off,
    };
    let _ = controller.send_command(&reset);
    let _ = controller.send_command(&off);

    let guard = controller.transport.lock().unwrap();
    let settle = guard.sent_frame_settle_us();
    assert_eq!(
        priorities_of(&settle),
        vec![
            DaliPriority::Configuration,
            DaliPriority::Transaction,
            DaliPriority::Configuration,
        ],
        "the second command must open its own transaction, not inherit one"
    );
}

#[test]
fn settling_is_drawn_at_random_from_inside_the_band() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x00);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(transport, test_clock());

    let off = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::Off,
    };
    for _ in 0..40 {
        let _ = controller.send_command(&off);
    }

    let guard = controller.transport.lock().unwrap();
    let settle = guard.sent_frame_settle_us();
    let claimed = priorities_of(&settle);
    assert_eq!(
        claimed,
        vec![claimed[0]; settle.len()],
        "every draw must stay inside the one band its command claims"
    );
    let distinct: std::collections::BTreeSet<u32> = settle.iter().copied().collect();
    assert!(
        distinct.len() > 1,
        "settling never varied across {} frames — the draw is not random, and \
         footnote c's collision avoidance buys nothing: {distinct:?}",
        settle.len()
    );
}

#[test]
fn a_mid_transaction_collision_keeps_priority_one_and_the_yield_shield() {
    let mock = MockDaliTransport::new();
    let query = DaliCommand::Standard {
        address: DaliAddress::short(1).unwrap(),
        command: StandardCommand::QueryStatus,
    };
    let frame = query.to_forward_frame().raw();
    mock.expect_forward_frame_with_backward(frame, Some(0x11));
    mock.expect_forward_frame_collision(frame);
    mock.expect_forward_frame_with_backward(frame, Some(0x11));
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());

    let outcome = controller.with_wire_class(Some(TransactionPriority::PeriodicQuery), |c| {
        c.with_wire_lease(frame_lease(&gate), |c| {
            c.transaction(|c| {
                c.send_command(&query)?;
                gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
                c.send_command(&query)
            })
        })
    });

    assert_eq!(
        outcome,
        Ok(DaliResponse::Answer(0x11)),
        "a started transaction is not abandoned to a yield"
    );
    let guard = transport.lock().unwrap();
    assert_eq!(
        priorities_of(&guard.sent_frame_settle_us()),
        vec![
            DaliPriority::PeriodicQuery,
            DaliPriority::Transaction,
            DaliPriority::Transaction,
        ],
        "the restart of a remaining frame stays at priority 1 (101 §9.2)"
    );
    assert_eq!(
        controller.wire_counters.transaction_reopened.load(Ordering::Relaxed),
        1,
        "the unit re-run inside the open transaction is still counted"
    );
}

#[test]
fn a_collision_does_not_open_a_transaction() {
    let mock = MockDaliTransport::new();
    let frame = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::Off,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame_collision(frame);
    mock.expect_forward_frame(frame);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(transport, test_clock());

    controller.with_wire_class(Some(TransactionPriority::UserAction), |c| {
        c.send_command(&DaliCommand::Standard {
            address: DaliAddress::Broadcast,
            command: StandardCommand::Off,
        })
    })
    .ok();

    let guard = controller.transport.lock().unwrap();
    assert_eq!(
        priorities_of(&guard.sent_frame_settle_us()),
        vec![DaliPriority::UserAction, DaliPriority::UserAction],
        "the retry after a collision is still the transaction's first frame"
    );
}

#[test]
fn controller_no_backward_for_non_query() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0xFF);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(transport, test_clock());

    let cmd = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::Off,
    };

    let response = controller.send_command(&cmd).unwrap();
    assert_eq!(response, DaliResponse::NoAnswer);
}

#[test]
fn controller_does_not_expect_backward_for_special_dtr_data_bytes() {
    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_receive_error(
        SpecialCommand::Dtr0(0xFE).to_forward_frame().raw(),
    );
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let response = controller
        .send_command(&DaliCommand::Special(SpecialCommand::Dtr0(0xFE)))
        .unwrap();

    assert_eq!(response, DaliResponse::NoAnswer);
    let guard = transport.lock().unwrap();
    assert_eq!(guard.script_error(), None);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn controller_sends_enable_device_type_before_extended_command() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x06);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let cmd = DaliCommand::Extended {
        address: DaliAddress::short(0).unwrap(),
        command: ExtendedCommand::Dt6(Dt6Command::QueryGearType),
    };

    let response = controller.send_command(&cmd).unwrap();
    assert_eq!(response, DaliResponse::Answer(0x06));

    let guard = transport.lock().unwrap();
    let frames = guard.sent_frames();
    assert_eq!(frames, vec![0xC106, 0x01ED]);
    assert_eq!(
        priorities_of(&guard.sent_frame_settle_us()),
        vec![DaliPriority::Configuration, DaliPriority::Transaction]
    );
}

#[test]
fn controller_sends_enable_device_type_color_before_dt8_query() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x00);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let cmd = DaliCommand::Extended {
        address: DaliAddress::short(0).unwrap(),
        command: ExtendedCommand::Dt8(Dt8Command::QueryColourStatus),
    };

    let response = controller.send_command(&cmd).unwrap();
    assert_eq!(response, DaliResponse::Answer(0x00));

    let frames = transport.lock().unwrap().sent_frames();
    assert_eq!(frames, vec![0xC108, 0x01F8]);
}

#[test]
fn controller_retries_collision_until_query_succeeds() {
    let mock = MockDaliTransport::new();
    let query = StandardCommand::QueryStatus;
    let frame = DaliCommand::Standard {
        address: DaliAddress::short(1).unwrap(),
        command: query,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame_collision(frame);
    mock.expect_forward_frame_with_backward(frame, Some(0x42));
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::with_retry_policy(
        Arc::clone(&transport),
        test_clock(),
        retry_policy(2),
    );
    let response = controller
        .send_command(&DaliCommand::Standard {
            address: DaliAddress::short(1).unwrap(),
            command: query,
        })
        .unwrap();

    assert_eq!(response, DaliResponse::Answer(0x42));
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![frame, frame]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn controller_retries_bus_busy_without_recording_fake_send() {
    let mock = MockDaliTransport::new();
    let query = StandardCommand::QueryStatus;
    let frame = DaliCommand::Standard {
        address: DaliAddress::short(2).unwrap(),
        command: query,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame_bus_busy(frame);
    mock.expect_forward_frame_with_backward(frame, Some(0x24));
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::with_retry_policy(
        Arc::clone(&transport),
        test_clock(),
        retry_policy(2),
    );
    let response = controller
        .send_command(&DaliCommand::Standard {
            address: DaliAddress::short(2).unwrap(),
            command: query,
        })
        .unwrap();

    assert_eq!(response, DaliResponse::Answer(0x24));
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![frame]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn controller_exhausts_foreign_in_window_retries_for_query() {
    let mock = MockDaliTransport::new();
    let frame = query_status_frame(3);
    mock.expect_forward_frame_foreign_in_window(frame);
    mock.expect_forward_frame_foreign_in_window(frame);
    mock.expect_forward_frame_foreign_in_window(frame);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::with_retry_policy(
        Arc::clone(&transport),
        test_clock(),
        retry_policy(3),
    );
    let error = controller.send_command(&query_status_command(3)).unwrap_err();

    assert_eq!(error, FrameError::Collision);
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![frame, frame, frame]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn a_violating_answer_is_terminal_after_one_frame() {
    let mock = MockDaliTransport::new();
    let frame = query_status_frame(4);
    mock.expect_forward_frame_corrupted_in_window(frame);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller =
        DaliController::with_retry_policy(Arc::clone(&transport), test_clock(), retry_policy(3));
    let response = controller.send_command(&query_status_command(4)).unwrap();

    assert_eq!(response, DaliResponse::Violation);
    let guard = transport.lock().unwrap();
    assert_eq!(
        guard.sent_frames(),
        vec![frame],
        "an answered query must not be re-asked, whatever the retry budget"
    );
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn a_violation_is_yes_but_never_a_value() {
    assert!(DaliResponse::Violation.is_yes());
    assert_eq!(DaliResponse::Violation.value(), None);
    assert!(!DaliResponse::NoAnswer.is_yes());
    assert!(DaliResponse::Answer(0x00).is_yes());
    assert_eq!(DaliResponse::Answer(0x2A).value(), Some(0x2A));
}

#[test]
fn controller_treats_corrupted_in_window_as_no_answer_for_non_query() {
    let mock = MockDaliTransport::new();
    let cmd = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::Off,
    };
    let frame = cmd.to_forward_frame().raw();
    mock.expect_forward_frame_corrupted_in_window(frame);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let response = controller.send_command(&cmd).unwrap();

    assert_eq!(response, DaliResponse::NoAnswer);
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![frame]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn controller_retries_contended_query_no_answer_when_enabled() {
    let mock = MockDaliTransport::new();
    let frame = query_status_frame(6);
    mock.expect_query_no_answer_contended(frame);
    mock.expect_forward_frame_with_backward(frame, Some(0x22));
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::with_retry_policy(
        Arc::clone(&transport),
        test_clock(),
        retry_policy_with_contention(2, true),
    );
    let response = controller.send_command(&query_status_command(6)).unwrap();

    assert_eq!(response, DaliResponse::Answer(0x22));
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![frame, frame]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn controller_keeps_quiet_query_no_answer_terminal() {
    let mock = MockDaliTransport::new();
    let frame = query_status_frame(7);
    mock.expect_forward_frame_with_backward(frame, None);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::with_retry_policy(
        Arc::clone(&transport),
        test_clock(),
        retry_policy_with_contention(2, true),
    );
    let response = controller.send_command(&query_status_command(7)).unwrap();

    assert_eq!(response, DaliResponse::NoAnswer);
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![frame]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn controller_does_not_retry_contended_query_no_answer_when_disabled() {
    let mock = MockDaliTransport::new();
    let frame = query_status_frame(8);
    mock.expect_query_no_answer_contended(frame);
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::with_retry_policy(
        Arc::clone(&transport),
        test_clock(),
        retry_policy_with_contention(2, false),
    );
    let response = controller.send_command(&query_status_command(8)).unwrap();

    assert_eq!(response, DaliResponse::NoAnswer);
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![frame]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn controller_send_command_observed_marks_contended_query_answer() {
    let mock = MockDaliTransport::new();
    let frame = query_status_frame(9);
    mock.expect_forward_frame_with_backward_contended(frame, Some(0x3A));
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let (response, contended) = controller.send_command_observed(&query_status_command(9)).unwrap();

    assert_eq!(response, DaliResponse::Answer(0x3A));
    assert!(contended);
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![frame]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn controller_send_command_observed_aggregates_contended_extended_prelude() {
    let mock = MockDaliTransport::new();
    let enable = SpecialCommand::EnableDeviceType(8).to_forward_frame().raw();
    let query = DaliCommand::Extended {
        address: DaliAddress::short(0).unwrap(),
        command: ExtendedCommand::Dt8(Dt8Command::QueryColourStatus),
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame_with_backward_contended(enable, None);
    mock.expect_forward_frame_with_backward(query, Some(0x00));
    let transport = Arc::new(Mutex::new(mock));

    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let (response, contended) = controller
        .send_command_observed(&DaliCommand::Extended {
            address: DaliAddress::short(0).unwrap(),
            command: ExtendedCommand::Dt8(Dt8Command::QueryColourStatus),
        })
        .unwrap();

    assert_eq!(response, DaliResponse::Answer(0x00));
    assert!(contended);
    let guard = transport.lock().unwrap();
    assert_eq!(guard.sent_frames(), vec![enable, query]);
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn background_work_stops_at_the_frame_boundary_after_the_lease_is_revoked() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::with_retry_policy(
        Arc::clone(&transport),
        test_clock(),
        retry_policy(3),
    );

    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);
    let sent_before_yield = 2;

    let outcome = controller.with_wire_lease(
        frame_lease(&gate),
        |c| {
            for _ in 0..sent_before_yield {
                c.send_command(&cmd).expect("a calm bus does not preempt");
            }
            gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
            c.send_command(&cmd)
        },
    );

    assert_eq!(outcome, Err(FrameError::Preempted));
    let guard = transport.lock().unwrap();
    assert_eq!(
        guard.sent_frames().len(),
        sent_before_yield,
        "the revoked frame must not reach the wire: {:?}",
        guard.sent_frames()
    );
}

#[test]
fn the_lease_is_scoped_so_the_command_after_a_preemption_runs_unguarded() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);

    let preempted = controller.with_wire_lease(
        frame_lease(&gate),
        |c| {
            gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
            c.send_command(&cmd)
        },
    );
    assert_eq!(preempted, Err(FrameError::Preempted));
    assert!(
        controller.send_command(&cmd).is_ok(),
        "interactive work runs unguarded"
    );
}

#[test]
fn step_work_does_not_yield_until_the_executor_closes_a_unit() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);

    let outcome = controller.with_wire_lease(step_lease(&gate), |c| {
        gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
        c.send_command(&cmd)
    });
    assert_eq!(
        outcome,
        Ok(DaliResponse::Answer(0x11)),
        "a half-written attribute may not be abandoned"
    );
}

#[test]
fn step_work_yields_at_the_boundary_the_executor_opens() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);

    let outcome = controller.with_wire_lease(step_lease(&gate), |c| {
        gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
        c.step_boundary();
        c.send_command(&cmd)
    });
    assert_eq!(outcome, Err(FrameError::Preempted));
    assert!(
        transport.lock().unwrap().sent_frames().is_empty(),
        "the yielded attribute must not put a frame on the wire"
    );
}

#[test]
fn a_step_under_a_frame_lease_holds_the_wire_after_its_first_frame() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);

    let (inside, after) = controller.with_wire_lease(frame_lease(&gate), |c| {
        let inside = c.step(|c| {
            c.send_command(&cmd).expect("a calm bus does not preempt");
            gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
            c.send_command(&cmd)
        });
        (inside, c.send_command(&cmd))
    });
    assert_eq!(inside, Ok(DaliResponse::Answer(0x11)), "a started step is finished");
    assert_eq!(after, Err(FrameError::Preempted), "the frame lease is back after the step");
}

#[test]
fn a_step_under_a_frame_lease_yields_at_its_opening() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);

    let outcome = controller.with_wire_lease(frame_lease(&gate), |c| {
        gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
        c.step(|c| c.send_command(&cmd))
    });
    assert_eq!(outcome, Err(FrameError::Preempted));
    assert!(transport.lock().unwrap().sent_frames().is_empty());
}

#[test]
fn a_step_spends_the_boundary_it_opens_and_hands_none_back() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);

    let (outer_step, next_step) = controller.with_wire_lease(step_lease(&gate), |c| {
        let outer_step = c.step(|c| {
            c.step(|c| c.send_command(&cmd)).expect("a calm bus does not preempt");
            gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
            c.send_command(&cmd)
        });
        (outer_step, c.step(|c| c.send_command(&cmd)))
    });
    assert_eq!(outer_step, Ok(DaliResponse::Answer(0x11)), "the outer step goes on after the inner one");
    assert_eq!(next_step, Err(FrameError::Preempted), "the next step opens the next boundary");
}

#[test]
fn a_step_that_sent_nothing_leaves_its_boundary_open() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);

    let outcome = controller.with_wire_lease(step_lease(&gate), |c| {
        c.step(|_| ());
        gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
        c.send_command(&cmd)
    });
    assert_eq!(outcome, Err(FrameError::Preempted), "nothing was sent since the boundary");
    assert!(transport.lock().unwrap().sent_frames().is_empty());
}

#[test]
fn a_step_boundary_is_consumed_by_the_exchange_that_follows_it() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let cmd = query_status_command(1);

    let outcome = controller.with_wire_lease(step_lease(&gate), |c| {
        c.step_boundary();
        c.send_command(&cmd).expect("a calm bus does not preempt");
        gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
        c.send_command(&cmd)
    });
    assert_eq!(
        outcome,
        Ok(DaliResponse::Answer(0x11)),
        "the spent boundary must not carry into the next triple"
    );
}

#[test]
fn an_arrival_that_predates_the_lease_does_not_preempt_it() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x11);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);

    let cmd = query_status_command(1);
    let outcome = controller.with_wire_lease(frame_lease(&gate), |c| c.send_command(&cmd));
    assert_eq!(outcome, Ok(DaliResponse::Answer(0x11)));
}

#[test]
fn background_yield_fits_inside_the_confirmation_timeout() {
    const SAFETY_FACTOR: u64 = 4;
    let budget = dali2rust_bus::BusConfig::default().confirmation_timeout_ms;
    assert!(
        BACKGROUND_YIELD_BUDGET_MS * SAFETY_FACTOR < budget,
        "yield budget {BACKGROUND_YIELD_BUDGET_MS} ms x{SAFETY_FACTOR} must stay well \
         under confirmation_timeout_ms {budget}"
    );
}

#[test]
fn the_yield_budget_covers_a_settle_plus_an_unanswered_exchange() {
    let worst_settle_ms = u64::from(DaliPriority::PeriodicQuery.min_settle_us()) / 1000;
    assert!(MAX_SETTLE_MS >= worst_settle_ms);
}

#[test]
fn send_raw_once_sends_exactly_one_frame_on_a_mangled_answer() {
    let mock = MockDaliTransport::new();
    let frame = query_status_frame(1);
    mock.expect_forward_frame_corrupted_in_window(frame);

    let (transport, mut controller) = {
        let transport = Arc::new(Mutex::new(mock));
        let controller =
            DaliController::new(Arc::clone(&transport), test_clock());
        (transport, controller)
    };
    let outcome = controller.send_raw_once(ForwardFrame::new(0x03, 0x90), true);

    assert_eq!(outcome, Ok((DaliResponse::Violation, false)));
    let guard = transport.lock().unwrap();
    assert_eq!(
        guard.sent_frames().len(),
        1,
        "one attempt means one frame: {:?}",
        guard.sent_frames()
    );
    assert_eq!(guard.script_error(), None);
}

struct ManualClock {
    now_ms: Arc<AtomicU64>,
}

impl Clock for ManualClock {
    fn monotonic_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

struct HeldFrom(Instant);

impl WireHold for HeldFrom {
    fn held(&self) -> Option<HoldCause> {
        (Instant::now() >= self.0).then_some(HoldCause::Quiescent)
    }
}

#[test]
fn a_wire_hold_that_begins_while_a_stray_half_is_waited_out_stops_the_frame() {
    const HOLD_BEGINS_AFTER_MS: u64 = 50;
    let (transport, mut controller) = scripted_controller_on(
        &[(None, RESET_SHORT_0), (Some(FOREIGN_QUERY), RESET_SHORT_0)],
        1,
        Box::new(StoppedClock::at(0)),
    );
    assert_eq!(controller.send_command(&reset_short_0()), Err(FrameError::BusBusy));
    let begins = Instant::now() + Duration::from_millis(HOLD_BEGINS_AFTER_MS);
    controller.hold = Some(Arc::new(HeldFrom(begins)));

    assert_eq!(
        controller.send_command(&reset_short_0()),
        Err(FrameError::Held(HoldCause::Quiescent)),
        "the hold and yield checks come after the wait, so they see what changed during it"
    );
    assert_eq!(transport.lock().unwrap().sent_frames().len(), 2);
}

#[test]
fn a_re_run_that_waited_out_its_stray_half_keeps_the_started_transaction_shield() {
    let mock = scripted_mock(&[
        (None, BROADCAST_OFF),
        (None, RESET_SHORT_0),
        (Some(FOREIGN_QUERY), RESET_SHORT_0),
    ]);
    mock.expect_forward_frame_collision(RESET_SHORT_0);
    mock.expect_forward_frame(RESET_SHORT_0);
    mock.expect_forward_frame(RESET_SHORT_0);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller =
        DaliController::new(Arc::clone(&transport), Box::new(StoppedClock::at(0)));
    controller.retry_policy = retry_policy(3);

    let outcome = controller.transaction(|controller| {
        controller.send_raw(ForwardFrame(BROADCAST_OFF), false)?;
        controller.hold = Some(Arc::new(HeldFrom(Instant::now())));
        controller.send_command(&reset_short_0())
    });

    assert!(
        outcome.is_ok(),
        "no hold lands inside a started transaction, even after the wait: {outcome:?}"
    );
    assert_script_consumed(&transport);
}

struct TimeAdvancingTransport {
    inner: MockDaliTransport,
    now_ms: Arc<AtomicU64>,
    advance_ms: u64,
}

impl DaliTransport for TimeAdvancingTransport {
    type Error = <MockDaliTransport as DaliTransport>::Error;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        self.inner.send_forward_frame(frame)
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        self.inner.receive_backward_frame()
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        self.inner.is_bus_idle()
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        let outcome = self
            .inner
            .exchange_frame_with_settle(frame, expects_backward, min_idle_us);
        self.now_ms.fetch_add(self.advance_ms, Ordering::SeqCst);
        outcome
    }

    fn honours_settle(&self) -> bool {
        true
    }

    fn captures_between_last_two_tx(&self) -> Option<u32> {
        self.inner.captures_between_last_two_tx()
    }
}

#[test]
fn continuous_traffic_earns_a_deliberate_release_every_budget() {
    let now_ms = Arc::new(AtomicU64::new(0));
    let transport = TimeAdvancingTransport {
        inner: MockDaliTransport::new(),
        now_ms: Arc::clone(&now_ms),
        advance_ms: 30,
    };
    let transport = Arc::new(Mutex::new(transport));
    let mut controller = DaliController::new(
        Arc::clone(&transport),
        Box::new(ManualClock {
            now_ms: Arc::clone(&now_ms),
        }),
    );

    let off = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::Off,
    };
    for _ in 0..30 {
        controller.send_command(&off).expect("send");
    }

    let releases = controller.wire_counters.bus_releases.load(Ordering::Relaxed);
    assert!(
        (2..=3).contains(&releases),
        "900 ms of continuous traffic owes a release per 400 ms budget, got {releases}"
    );
    let guard = transport.lock().unwrap();
    let settles = guard.inner.sent_frame_settle_us();
    let released = settles
        .iter()
        .filter(|&&us| us == BUS_RELEASE_SETTLE_US)
        .count();
    assert_eq!(
        released as u32, releases,
        "every counted release is a frame that actually presented 22 ms: {settles:?}"
    );
}

#[test]
fn idle_gaps_discharge_the_release_obligation() {
    let now_ms = Arc::new(AtomicU64::new(0));
    let transport = TimeAdvancingTransport {
        inner: MockDaliTransport::new(),
        now_ms: Arc::clone(&now_ms),
        advance_ms: 30,
    };
    let transport = Arc::new(Mutex::new(transport));
    let mut controller = DaliController::new(
        Arc::clone(&transport),
        Box::new(ManualClock {
            now_ms: Arc::clone(&now_ms),
        }),
    );

    let off = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::Off,
    };
    for _ in 0..30 {
        now_ms.fetch_add(25, Ordering::SeqCst);
        controller.send_command(&off).expect("send");
    }

    assert_eq!(
        controller.wire_counters.bus_releases.load(Ordering::Relaxed),
        0,
        "an idle bus owes no deliberate release"
    );
}

#[test]
fn a_terminally_failed_unit_still_books_its_completion() {
    let mock = MockDaliTransport::new();
    let reset_frame = DaliCommand::Standard {
        address: DaliAddress::short(0).unwrap(),
        command: StandardCommand::Reset,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(reset_frame);
    mock.expect_forward_frame_collision(reset_frame);

    let transport = Arc::new(Mutex::new(mock));
    let mut controller =
        DaliController::new(Arc::clone(&transport), test_clock());
    controller.retry_policy = retry_policy(1);

    let reset = DaliCommand::Standard {
        address: DaliAddress::short(0).unwrap(),
        command: StandardCommand::Reset,
    };
    controller
        .send_command(&reset)
        .expect_err("the pair never completed");

    let counters = &controller.wire_counters;
    assert_eq!(counters.transactions_started.load(Ordering::Relaxed), 1);
    assert_eq!(counters.transactions_completed.load(Ordering::Relaxed), 1);
    assert_eq!(
        counters.transaction_reopened.load(Ordering::Relaxed),
        0,
        "no retry followed, so nothing was reopened"
    );
}

#[test]
fn a_split_send_twice_pair_is_refused_at_the_acceptance_edge() {
    let reset_frame = DaliCommand::Standard {
        address: DaliAddress::short(0).unwrap(),
        command: StandardCommand::Reset,
    }
    .to_forward_frame()
    .raw();
    let reset = DaliCommand::Standard {
        address: DaliAddress::short(0).unwrap(),
        command: StandardCommand::Reset,
    };

    let mock = MockDaliTransport::new();
    mock.expect_forward_frame(reset_frame);
    mock.expect_forward_frame(reset_frame);
    mock.set_last_tx_settle_ticks(Some(904));
    let transport = Arc::new(Mutex::new(mock));
    let mut controller =
        DaliController::new(Arc::clone(&transport), test_clock());
    controller.retry_policy = retry_policy(1);

    controller
        .send_command(&reset)
        .expect_err("a pair split past 94 ms cannot be trusted as executed");
    assert_eq!(
        controller
            .wire_counters
            .send_twice_split
            .load(Ordering::Relaxed),
        1
    );

    let mock = MockDaliTransport::new();
    mock.expect_forward_frame(reset_frame);
    mock.expect_forward_frame(reset_frame);
    mock.set_last_tx_settle_ticks(Some(722));
    let transport = Arc::new(Mutex::new(mock));
    let mut controller =
        DaliController::new(Arc::clone(&transport), test_clock());

    controller
        .send_command(&reset)
        .expect("the gear still executed the pair");
    assert_eq!(
        controller
            .wire_counters
            .send_twice_over_transmitter_max
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        controller
            .wire_counters
            .send_twice_split
            .load(Ordering::Relaxed),
        0
    );
}

const RESET_SHORT_0: u16 = 0x0120;
const SELECT_DIMMING_CURVE_SHORT_0: u16 = 0x01E3;
const ENABLE_DT6: u16 = 0xC106;
const FOREIGN_QUERY: u16 = 0xFF90;
const BROADCAST_OFF: u16 = 0xFF00;

fn reset_short_0() -> DaliCommand {
    DaliCommand::Standard {
        address: DaliAddress::short(0).unwrap(),
        command: StandardCommand::Reset,
    }
}

fn select_dimming_curve_short_0() -> DaliCommand {
    DaliCommand::Extended {
        address: DaliAddress::short(0).unwrap(),
        command: ExtendedCommand::Dt6(Dt6Command::SelectDimmingCurve),
    }
}

fn scripted_mock(script: &[(Option<u16>, u16)]) -> MockDaliTransport {
    let mock = MockDaliTransport::new();
    for &(foreign_before, frame) in script {
        if let Some(foreign) = foreign_before {
            mock.expect_foreign_frame_before_next(foreign);
        }
        mock.expect_forward_frame(frame);
    }
    mock
}

fn scripted_controller(
    script: &[(Option<u16>, u16)],
    attempts: u8,
) -> (Arc<Mutex<MockDaliTransport>>, DaliController<MockDaliTransport>) {
    scripted_controller_on(script, attempts, test_clock())
}

fn scripted_controller_on(
    script: &[(Option<u16>, u16)],
    attempts: u8,
    clock: Box<dyn Clock>,
) -> (Arc<Mutex<MockDaliTransport>>, DaliController<MockDaliTransport>) {
    let transport = Arc::new(Mutex::new(scripted_mock(script)));
    let mut controller = DaliController::new(Arc::clone(&transport), clock);
    controller.retry_policy = retry_policy(attempts);
    (transport, controller)
}

fn plain_half(raw: u16) -> WireFrame {
    WireFrame::Forward16(ForwardFrame(raw))
}

fn split_pairs(controller: &DaliController<MockDaliTransport>) -> u32 {
    controller.wire_counters.send_twice_split.load(Ordering::Relaxed)
}

#[test]
fn a_foreign_frame_between_the_halves_of_a_pair_re_runs_the_pair() {
    let (transport, mut controller) = scripted_controller(
        &[
            (None, RESET_SHORT_0),
            (Some(FOREIGN_QUERY), RESET_SHORT_0),
            (None, RESET_SHORT_0),
            (None, RESET_SHORT_0),
        ],
        3,
    );

    controller
        .send_command(&reset_short_0())
        .expect("the re-run lands an unbroken pair");

    assert_script_consumed(&transport);
    assert_eq!(
        split_pairs(&controller),
        1,
        "101 §9.3: a receiver reads two frames with an active state between them as separate"
    );
}

#[test]
fn a_foreign_frame_between_the_halves_re_runs_an_extended_pair_from_its_prelude() {
    let (transport, mut controller) = scripted_controller(
        &[
            (None, ENABLE_DT6),
            (None, SELECT_DIMMING_CURVE_SHORT_0),
            (Some(FOREIGN_QUERY), SELECT_DIMMING_CURVE_SHORT_0),
            (None, ENABLE_DT6),
            (None, SELECT_DIMMING_CURVE_SHORT_0),
            (None, SELECT_DIMMING_CURVE_SHORT_0),
        ],
        3,
    );

    controller
        .send_command(&select_dimming_curve_short_0())
        .expect("the re-run lands prelude and pair unbroken");

    assert_script_consumed(&transport);
    assert_eq!(split_pairs(&controller), 1);
}

#[test]
fn a_pair_split_by_a_foreign_frame_on_its_last_attempt_fails() {
    let (transport, mut controller) = scripted_controller(
        &[(None, RESET_SHORT_0), (Some(FOREIGN_QUERY), RESET_SHORT_0)],
        1,
    );

    assert_eq!(
        controller.send_command(&reset_short_0()),
        Err(FrameError::BusBusy),
        "101 §9.3: an active state between the halves leaves no pair, so none is reported as sent"
    );
    assert_script_consumed(&transport);
    assert_eq!(split_pairs(&controller), 1);
}

#[test]
fn a_pair_split_twice_in_a_row_lands_on_its_third_attempt() {
    let (transport, mut controller) = scripted_controller(
        &[
            (None, RESET_SHORT_0),
            (Some(FOREIGN_QUERY), RESET_SHORT_0),
            (None, RESET_SHORT_0),
            (Some(FOREIGN_QUERY), RESET_SHORT_0),
            (None, RESET_SHORT_0),
            (None, RESET_SHORT_0),
        ],
        3,
    );

    controller
        .send_command(&reset_short_0())
        .expect("the third attempt lands an unbroken pair");

    assert_script_consumed(&transport);
    assert_eq!(split_pairs(&controller), 2);
}

#[test]
fn a_re_run_holds_the_copy_identical_to_the_stray_half_past_the_grey_area() {
    const MS_PER_EXCHANGE: u64 = 30;
    let now_ms = Arc::new(AtomicU64::new(0));
    let transport = Arc::new(Mutex::new(TimeAdvancingTransport {
        inner: scripted_mock(&[
            (None, RESET_SHORT_0),
            (Some(FOREIGN_QUERY), RESET_SHORT_0),
            (None, RESET_SHORT_0),
            (None, RESET_SHORT_0),
        ]),
        now_ms: Arc::clone(&now_ms),
        advance_ms: MS_PER_EXCHANGE,
    }));
    let clock = Box::new(ManualClock { now_ms });
    let mut controller = DaliController::new(Arc::clone(&transport), clock);
    controller.retry_policy = retry_policy(3);

    controller
        .send_command(&reset_short_0())
        .expect("the held re-run lands an unbroken pair");

    let gap_ms = transport.lock().unwrap().inner.forward_frame_gap_ms(1, 2).expect("four frames");
    let stray_to_copy = Duration::from_millis(u64::try_from(gap_ms).expect("fits"));
    assert!(
        stray_to_copy
            >= Duration::from_micros(u64::from(RetryPolicy::SEND_TWICE_GREY_MAX_INTERVAL_US)),
        "101 §9.3, Table 20: counted from the stray half, not from the pair's first frame, \
         {stray_to_copy:?}"
    );
}

#[test]
fn a_held_re_run_opens_a_new_transaction_at_its_class_priority() {
    let (transport, mut controller) = scripted_controller_on(
        &[
            (None, RESET_SHORT_0),
            (Some(FOREIGN_QUERY), RESET_SHORT_0),
            (None, RESET_SHORT_0),
            (None, RESET_SHORT_0),
        ],
        3,
        Box::new(StoppedClock::at(0)),
    );

    controller.send_command(&reset_short_0()).expect("the held re-run lands");

    let class = DaliPriority::Configuration;
    assert_eq!(
        priorities_of(&transport.lock().unwrap().sent_frame_settle_us()),
        vec![class, DaliPriority::Transaction, class, DaliPriority::Transaction],
        "101 §9.2: the line was released for longer than a transaction may pause"
    );
    assert_eq!(
        controller.wire_counters.transactions_completed.load(Ordering::Relaxed),
        2
    );
}

#[test]
fn a_stray_half_holds_only_the_frame_identical_to_it_until_it_expires() {
    let now_ms = Arc::new(AtomicU64::new(0));
    let clock = Box::new(ManualClock {
        now_ms: Arc::clone(&now_ms),
    });
    let (_, mut controller) = scripted_controller_on(
        &[
            (None, ENABLE_DT6),
            (None, SELECT_DIMMING_CURVE_SHORT_0),
            (Some(FOREIGN_QUERY), SELECT_DIMMING_CURVE_SHORT_0),
        ],
        1,
        clock,
    );
    assert_eq!(
        controller.send_command(&select_dimming_curve_short_0()),
        Err(FrameError::BusBusy)
    );

    assert_eq!(
        controller.stray_half_wait(plain_half(SELECT_DIMMING_CURVE_SHORT_0)),
        Duration::from_millis(STRAY_HALF_LIFETIME_MS)
    );
    assert_eq!(
        controller.stray_half_wait(plain_half(ENABLE_DT6)),
        Duration::ZERO,
        "a different frame between them already keeps the stray half and an identical frame apart"
    );
    now_ms.fetch_add(STRAY_HALF_LIFETIME_MS, Ordering::SeqCst);
    assert_eq!(
        controller.stray_half_wait(plain_half(SELECT_DIMMING_CURVE_SHORT_0)),
        Duration::ZERO,
        "Table 20: past the grey area no gear pairs the stray half"
    );
}

#[test]
fn only_a_frame_of_ours_on_the_wire_ends_the_stray_half() {
    let (transport, mut controller) = scripted_controller_on(
        &[(None, RESET_SHORT_0), (Some(FOREIGN_QUERY), RESET_SHORT_0)],
        1,
        Box::new(StoppedClock::at(0)),
    );
    let broadcast_off = ForwardFrame::new(0xFF, 0x00);
    let held = Duration::from_millis(STRAY_HALF_LIFETIME_MS);
    assert_eq!(controller.send_command(&reset_short_0()), Err(FrameError::BusBusy));

    transport
        .lock()
        .unwrap()
        .expect_forward_frame_bus_busy(broadcast_off.raw());
    let _ = controller.send_raw(broadcast_off, false);
    assert_eq!(
        controller.stray_half_wait(plain_half(RESET_SHORT_0)),
        held,
        "a frame that never reached the wire leaves the stray half live"
    );

    transport.lock().unwrap().expect_forward_frame(broadcast_off.raw());
    controller.send_raw(broadcast_off, false).expect("sent");
    assert_script_consumed(&transport);
    assert_eq!(
        controller.stray_half_wait(plain_half(RESET_SHORT_0)),
        Duration::ZERO,
        "101 §9.3: a frame of ours after the stray half leaves it no consecutive frame to pair with"
    );
}

#[test]
fn a_foreign_frame_before_the_unit_leaves_the_pair_alone() {
    let (transport, mut controller) = scripted_controller(
        &[(Some(FOREIGN_QUERY), RESET_SHORT_0), (None, RESET_SHORT_0)],
        3,
    );
    controller.send_command(&reset_short_0()).expect("an unbroken pair");
    assert_script_consumed(&transport);
    assert_eq!(split_pairs(&controller), 0, "the line before the first half is not between the halves");

    let (transport, mut controller) = scripted_controller(
        &[
            (Some(FOREIGN_QUERY), ENABLE_DT6),
            (None, SELECT_DIMMING_CURVE_SHORT_0),
            (None, SELECT_DIMMING_CURVE_SHORT_0),
        ],
        3,
    );
    controller
        .send_command(&select_dimming_curve_short_0())
        .expect("an unbroken prelude and pair");
    assert_script_consumed(&transport);
    assert_eq!(split_pairs(&controller), 0);
}

#[test]
fn a_yield_close_hands_back_the_exemption_with_the_depth() {
    let mock = MockDaliTransport::new();
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(transport, test_clock());

    controller.transaction_exempt(|controller| {
        let _ = controller.send_raw(ForwardFrame::new(0x03, 0x90), false);
        assert!(controller.tx.first_frame_sent, "the unit has started");
        controller.close_transaction_for_yield();
        assert!(
            !controller.tx.first_frame_sent,
            "the yield-close ended the unit"
        );
        assert_eq!(controller.tx.depth, 1, "the depth is handed back");
        assert!(controller.tx.exempt, "and so is the exemption");
    });
}

#[test]
fn the_outermost_scope_owns_the_exemption() {
    let mock = MockDaliTransport::new();
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(transport, test_clock());

    controller.transaction(|controller| {
        controller.transaction_exempt(|controller| {
            assert!(
                !controller.tx.exempt,
                "an inner exempt scope joins the outer budget"
            );
        });
    });
}

#[test]
fn a_raw_send_twice_pair_is_one_transaction() {
    let frame = ForwardFrame::new(0x01, 0x20);
    let mock = MockDaliTransport::new();
    mock.expect_forward_frame(frame.raw());
    mock.expect_forward_frame(frame.raw());

    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let counters = Arc::new(dali2rust_platform::dali::DaliWireCounters::default());
    controller.set_wire_counters(Arc::clone(&counters));

    controller
        .send_raw_pair(frame, false)
        .expect("both halves go out");

    assert_eq!(
        transport.lock().expect("mock").sent_frames(),
        vec![frame.raw(), frame.raw()],
        "a pair is two identical frames"
    );
    assert_eq!(
        counters.transactions_completed.load(Ordering::Relaxed),
        1,
        "two transactions means the second half opened its own, at the wrong priority"
    );
}

#[test]
fn a_retried_dt8_query_carries_a_fresh_enable_prelude() {
    const DT8_QUERY_COLOUR_STATUS: u8 = 0xF8;
    let mock = MockDaliTransport::new();
    let enable = DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
        .to_forward_frame()
        .raw();
    let address = DaliAddress::short(4).unwrap();
    let query =
        ForwardFrame::new(address.encode_address_byte() | 0x01, DT8_QUERY_COLOUR_STATUS).raw();

    mock.expect_forward_frame(enable);
    mock.expect_forward_frame_foreign_in_window(query);
    mock.expect_forward_frame(enable);
    mock.expect_forward_frame_with_backward(query, Some(0x42));
    let transport = Arc::new(Mutex::new(mock));

    let mut controller =
        DaliController::with_retry_policy(Arc::clone(&transport), test_clock(), retry_policy(3));
    let (response, _contended) = controller
        .send_raw_enabled_query(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8)).to_forward_frame(),
            ForwardFrame::new(address.encode_address_byte() | 0x01, DT8_QUERY_COLOUR_STATUS),
        )
        .expect("the retry must be able to succeed");

    assert_eq!(response, DaliResponse::Answer(0x42));
    let guard = transport.lock().unwrap();
    assert_eq!(
        guard.sent_frames(),
        vec![enable, query, enable, query],
        "the retry must restart from the prelude; [E, Q, Q] is a bare extended \
         opcode in the standard space, which the gear ignores"
    );
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

#[test]
fn an_uncontended_dt8_query_sends_its_prelude_exactly_once() {
    const DT8_QUERY_COLOUR_STATUS: u8 = 0xF8;
    let mock = MockDaliTransport::new();
    let enable = DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
        .to_forward_frame()
        .raw();
    let address = DaliAddress::short(4).unwrap();
    let query =
        ForwardFrame::new(address.encode_address_byte() | 0x01, DT8_QUERY_COLOUR_STATUS).raw();
    mock.expect_forward_frame(enable);
    mock.expect_forward_frame_with_backward(query, Some(0x07));
    let transport = Arc::new(Mutex::new(mock));

    let mut controller =
        DaliController::with_retry_policy(Arc::clone(&transport), test_clock(), retry_policy(3));
    let (response, _) = controller
        .send_raw_enabled_query(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8)).to_forward_frame(),
            ForwardFrame::new(address.encode_address_byte() | 0x01, DT8_QUERY_COLOUR_STATUS),
        )
        .expect("exchange");
    assert_eq!(response, DaliResponse::Answer(0x07));
    assert_eq!(transport.lock().unwrap().sent_frames(), vec![enable, query]);
}

const PROBE_24: [u8; 3] = [0xFF, 0xFE, 0x3D];

#[test]
fn a_24_bit_frame_is_announced_at_its_command_class() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

    controller
        .with_wire_class(Some(TransactionPriority::PeriodicQuery), |c| {
            c.send_frame24_once(PROBE_24, true)
        })
        .expect("a mock answers or stays silent, never faults");

    let guard = transport.lock().unwrap();
    assert_eq!(
        priorities_of(&guard.sent_frame24_settle_us()),
        vec![DaliPriority::PeriodicQuery],
        "the probe must announce priority 5 (DiiA 351 §7), from its row"
    );
}

#[test]
fn a_24_bit_continuation_inside_a_bracket_goes_at_priority_one() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let frame = [0x07, 0xFE, 0x00];

    controller.with_wire_class(Some(TransactionPriority::Configuration), |c| {
        c.transaction(|c| {
            c.send_frame24(frame, false)?;
            c.send_frame24(frame, false)
        })
    })
    .expect("two plain frames on a mock");

    let guard = transport.lock().unwrap();
    assert_eq!(
        priorities_of(&guard.sent_frame24_settle_us()),
        vec![DaliPriority::Configuration, DaliPriority::Transaction],
        "the first frame carries the class, every following frame priority 1"
    );
    assert_eq!(
        controller.wire_counters.transactions_started.load(Ordering::Relaxed),
        1,
        "a 24-bit bracket is now booked as a transaction"
    );
}

#[test]
fn a_collided_24_bit_first_frame_restarts_at_its_class() {
    let mock = MockDaliTransport::new();
    mock.script_frame24_outcome(TransferOutcome::Collision);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::with_retry_policy(
        Arc::clone(&transport),
        test_clock(),
        retry_policy(3),
    );

    controller
        .with_wire_class(Some(TransactionPriority::UserAction), |c| {
            c.transaction(|c| c.send_frame24([0x07, 0xFE, 0x00], false))
        })
        .expect("the resend completes");

    let guard = transport.lock().unwrap();
    assert_eq!(
        priorities_of(&guard.sent_frame24_settle_us()),
        vec![DaliPriority::UserAction, DaliPriority::UserAction],
        "a destroyed first frame is still a first frame"
    );
}

#[test]
fn a_foreign_frame_in_a_24_bit_window_is_terminal_and_leaves_the_bracket_started() {
    let mock = MockDaliTransport::new();
    mock.script_frame24_outcome(TransferOutcome::ForeignInWindow);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let frame = [0x07, 0xFE, 0x00];

    let (first, second) = controller.with_wire_class(Some(TransactionPriority::Configuration), |c| {
        c.transaction(|c| {
            let first = c.send_frame24(frame, true);
            let second = c.send_frame24(frame, false);
            (first, second)
        })
    });

    assert_eq!(first, Err(Frame24Fault::Contended), "a foreign frame is not resent");
    assert!(second.is_ok());
    let guard = transport.lock().unwrap();
    assert_eq!(
        priorities_of(&guard.sent_frame24_settle_us()),
        vec![DaliPriority::Configuration, DaliPriority::Transaction],
        "the bracket the heard frame opened is still open"
    );
}

#[test]
fn a_24_bit_walk_earns_the_deliberate_release_every_budget() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller =
        DaliController::new(Arc::clone(&transport), Box::new(AdvancingClock::new(4)));

    controller.with_wire_class(Some(TransactionPriority::Configuration), |c| {
        for short in 0..30u8 {
            c.send_frame24([short << 1 | 1, 0xFE, 0x35], true)
                .expect("a silent mock is a no-answer, not a fault");
        }
    });

    let releases = controller.wire_counters.bus_releases.load(Ordering::Relaxed);
    assert!(releases >= 1, "a 24-bit walk must release the bus at least once per budget");
    let guard = transport.lock().unwrap();
    let lead = u32::from(TX_ARM_LEAD_TICKS) * PHY_TICK_US;
    let released = guard
        .sent_frame24_settle_us()
        .iter()
        .filter(|&&us| us + lead > DaliPriority::PeriodicQuery.max_settle_us())
        .count();
    assert_eq!(
        released as u32, releases,
        "every release is a frame whose settling exceeds the priority-5 maximum"
    );
}

#[test]
fn a_24_bit_read_yields_at_a_frame_boundary_when_an_operator_arrives() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());

    let outcome = controller.with_wire_lease(frame_lease(&gate), |c| {
        gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
        c.send_frame24([0x01, 0xFE, 0x35], true)
    });

    assert_eq!(outcome, Err(Frame24Fault::Preempted));
    assert!(
        transport.lock().unwrap().sent_frames24().is_empty(),
        "the revoked frame must not reach the wire"
    );
}

#[test]
fn a_24_bit_exchange_is_offered_to_the_sniffer_tap_as_forward24() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let (tap, records) = dali2rust_platform::dali::SnifferTap::new(8);
    tap.set_enabled(true);
    controller.set_sniffer_tap(tap, 0);

    controller
        .send_frame24_once(PROBE_24, false)
        .expect("a plain frame on a mock");

    let record = records.try_recv().expect("the tap saw our frame");
    assert_eq!(record.kind, dali2rust_platform::dali::ObservedRawFrameKind::Forward24);
    assert_eq!(record.bytes, PROBE_24);
    assert_eq!(record.direction, dali2rust_platform::dali::FrameDirection::Tx);
}

#[test]
fn a_24_bit_step_unit_holds_the_yield_until_its_boundary() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0x02);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let gate = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let frame = [0x01, 0xFE, 0x36];

    let (held, yielded) = controller.with_wire_lease(step_lease(&gate), |c| {
        gate.note_arrival(dali2rust_platform::dali::WirePriority::Interactive);
        let held = c.send_frame24(frame, true);
        c.step_boundary();
        let yielded = c.send_frame24(frame, true);
        (held, yielded)
    });

    assert_eq!(held, Ok(DaliResponse::Answer(0x02)), "a half-written 103 field is finished");
    assert_eq!(yielded, Err(Frame24Fault::Preempted), "the next field waits for the operator");
    assert_eq!(
        transport.lock().unwrap().sent_frames24().len(),
        1,
        "the yielded frame must not reach the wire"
    );
}

struct UnsentFrame24Transport {
    inner: MockDaliTransport,
}

impl DaliTransport for UnsentFrame24Transport {
    type Error = <MockDaliTransport as DaliTransport>::Error;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        self.inner.send_forward_frame(frame)
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        self.inner.receive_backward_frame()
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        self.inner.is_bus_idle()
    }

    fn exchange_frame24_with_settle(
        &mut self,
        _frame: [u8; 3],
        _expects_backward: bool,
        _min_idle_us: u32,
    ) -> Result<TransferOutcome, Frame24Error<Self::Error>> {
        Err(Frame24Error::Transport(()))
    }

    fn supports_frame24(&self) -> bool {
        true
    }
}

#[test]
fn a_24_bit_frame_the_transport_never_sent_is_not_silence() {
    let transport = Arc::new(Mutex::new(UnsentFrame24Transport {
        inner: MockDaliTransport::new(),
    }));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

    assert_eq!(
        controller.send_frame24_once(PROBE_24, true),
        Err(Frame24Fault::Contended),
        "a query whose frame never left proves nothing about the bus"
    );
    assert_eq!(
        controller.send_frame24(PROBE_24, false),
        Err(Frame24Fault::Contended),
        "a command whose frame never left did not happen"
    );
}

struct ScriptedFrame24Transport {
    inner: MockDaliTransport,
    outcomes: std::collections::VecDeque<TransferOutcome>,
    sent: Vec<[u8; 3]>,
    settle_ticks: Option<u16>,
    captured_before: Vec<usize>,
}

impl ScriptedFrame24Transport {
    fn new(outcomes: &[TransferOutcome], settle_ticks: Option<u16>) -> Self {
        Self {
            inner: MockDaliTransport::new(),
            outcomes: outcomes.iter().copied().collect(),
            sent: Vec::new(),
            settle_ticks,
            captured_before: Vec::new(),
        }
    }

    fn with_a_capture_before(mut self, frame_index: usize) -> Self {
        self.captured_before.push(frame_index);
        self
    }
}

impl DaliTransport for ScriptedFrame24Transport {
    type Error = <MockDaliTransport as DaliTransport>::Error;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        self.inner.send_forward_frame(frame)
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        self.inner.receive_backward_frame()
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        self.inner.is_bus_idle()
    }

    fn exchange_frame24_with_settle(
        &mut self,
        frame: [u8; 3],
        _expects_backward: bool,
        _min_idle_us: u32,
    ) -> Result<TransferOutcome, Frame24Error<Self::Error>> {
        self.sent.push(frame);
        Ok(self.outcomes.pop_front().unwrap_or(TransferOutcome::NoAnswer))
    }

    fn supports_frame24(&self) -> bool {
        true
    }

    fn last_tx_settle_ticks(&self) -> Option<u16> {
        self.settle_ticks
    }

    fn captures_between_last_two_tx(&self) -> Option<u32> {
        let last = self.sent.len().checked_sub(1)?;
        Some(u32::from(self.captured_before.contains(&last)))
    }
}

#[test]
fn a_24_bit_send_twice_pair_split_by_a_foreign_frame_is_resent_whole() {
    let transport = Arc::new(Mutex::new(
        ScriptedFrame24Transport::new(&[], None).with_a_capture_before(1),
    ));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    controller.retry_policy = retry_policy(3);

    controller
        .send_frame24_twice(PROBE_24)
        .expect("the second attempt lands an unbroken pair");

    assert_eq!(transport.lock().unwrap().sent, vec![PROBE_24; 4]);
    assert_eq!(
        controller.wire_counters.send_twice_split.load(Ordering::Relaxed),
        1
    );
}

#[test]
fn a_24_bit_pair_split_by_a_foreign_frame_on_its_last_attempt_is_contended() {
    let transport = Arc::new(Mutex::new(
        ScriptedFrame24Transport::new(&[], None).with_a_capture_before(1),
    ));
    let mut controller =
        DaliController::new(Arc::clone(&transport), Box::new(StoppedClock::at(0)));
    controller.retry_policy = retry_policy(1);

    assert_eq!(
        controller.send_frame24_twice(PROBE_24),
        Err(Frame24Fault::Contended)
    );
    assert_eq!(transport.lock().unwrap().sent, vec![PROBE_24; 2]);
    assert_eq!(
        controller.stray_half_wait(WireFrame::Forward24(PROBE_24)),
        Duration::from_millis(STRAY_HALF_LIFETIME_MS),
        "101 §9.3: the next identical 24-bit frame waits out the stray half"
    );
}

#[test]
fn a_24_bit_send_twice_pair_broken_by_a_collision_is_resent_whole() {
    let transport = Arc::new(Mutex::new(ScriptedFrame24Transport::new(
        &[TransferOutcome::NoAnswer, TransferOutcome::Collision],
        None,
    )));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    controller.retry_policy = retry_policy(3);

    controller
        .send_frame24_twice(PROBE_24)
        .expect("the second attempt lands both halves");

    assert_eq!(
        transport.lock().unwrap().sent,
        vec![PROBE_24; 4],
        "a lone second half is ignored by the gear; the unit is the pair (101 §9.3)"
    );
}

#[test]
fn a_24_bit_send_twice_pair_split_past_the_interval_is_not_executed() {
    let transport = Arc::new(Mutex::new(ScriptedFrame24Transport::new(&[], Some(904))));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    controller.retry_policy = retry_policy(1);

    assert_eq!(
        controller.send_frame24_twice(PROBE_24),
        Err(Frame24Fault::Contended),
        "a pair split past 94 ms cannot be trusted as executed"
    );
    assert_eq!(
        controller.wire_counters.send_twice_split.load(Ordering::Relaxed),
        1
    );
}

const ARMED_SHORT: u8 = 3;
const ARMED_OPERAND: u8 = 0x0B;

fn armed_short(command: StandardCommand) -> DaliCommand {
    DaliCommand::Standard {
        address: DaliAddress::short(ARMED_SHORT).unwrap(),
        command,
    }
}

struct ArmedFrames {
    arm: u16,
    proof: u16,
    act: u16,
}

fn armed_frames() -> ArmedFrames {
    ArmedFrames {
        arm: DaliCommand::Special(SpecialCommand::Dtr0(ARMED_OPERAND))
            .to_forward_frame()
            .raw(),
        proof: armed_short(StandardCommand::QueryContentDtr0)
            .to_forward_frame()
            .raw(),
        act: armed_short(StandardCommand::SetShortAddress)
            .to_forward_frame()
            .raw(),
    }
}

// IEC 62386-102 §11.4.12
fn arm_prove_and_act<T: DaliTransport + Send>(
    controller: &mut DaliController<T>,
) -> Result<bool, FrameError> {
    controller.send_command(&DaliCommand::Special(SpecialCommand::Dtr0(ARMED_OPERAND)))?;
    let proof = controller.send_command(&armed_short(StandardCommand::QueryContentDtr0))?;
    if proof != DaliResponse::Answer(ARMED_OPERAND) {
        return Ok(false);
    }
    controller.send_command(&armed_short(StandardCommand::SetShortAddress))?;
    Ok(true)
}

fn expect_armed_unit(mock: &MockDaliTransport, f: &ArmedFrames) {
    mock.expect_forward_frame(f.arm);
    mock.expect_forward_frame_with_backward(f.proof, Some(ARMED_OPERAND));
}

type ScriptRefusal = fn(&MockDaliTransport, u16);

#[test]
fn a_refused_action_reruns_its_unit_from_the_arming_frame() {
    let refusals: [(ScriptRefusal, bool); 2] = [
        (MockDaliTransport::expect_forward_frame_collision, true),
        (MockDaliTransport::expect_forward_frame_bus_busy, false),
    ];
    for (refuse, reached_the_wire) in refusals {
        let f = armed_frames();
        let mock = MockDaliTransport::new();
        expect_armed_unit(&mock, &f);
        refuse(&mock, f.act);
        expect_armed_unit(&mock, &f);
        mock.expect_forward_frame(f.act);
        mock.expect_forward_frame(f.act);
        let transport = Arc::new(Mutex::new(mock));
        let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

        let written = controller.unit(arm_prove_and_act);

        assert_eq!(written, Ok(true));
        let mut expected = vec![f.arm, f.proof];
        if reached_the_wire {
            expected.push(f.act);
        }
        expected.extend([f.arm, f.proof, f.act, f.act]);
        let guard = transport.lock().unwrap();
        assert_eq!(
            guard.sent_frames(),
            expected,
            "the retry re-arms and re-proves DTR0, which another master may have written"
        );
        assert_eq!(guard.script_error(), None);
        assert_eq!(controller.wire_counters.exchange_retries.load(Ordering::Relaxed), 1);
        assert_eq!(
            controller.wire_counters.transaction_reopened.load(Ordering::Relaxed),
            1,
            "a unit re-run is a re-run, whether or not it reopens the same transaction"
        );
    }
}

#[test]
fn a_unit_inside_a_started_transaction_reruns_at_priority_one() {
    let f = armed_frames();
    let lead = query_status_frame(ARMED_SHORT);
    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_with_backward(lead, Some(0));
    expect_armed_unit(&mock, &f);
    mock.expect_forward_frame_collision(f.act);
    expect_armed_unit(&mock, &f);
    mock.expect_forward_frame(f.act);
    mock.expect_forward_frame(f.act);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

    let written = controller.transaction(|c| {
        c.send_command(&query_status_command(ARMED_SHORT))?;
        c.unit(arm_prove_and_act)
    });

    assert_eq!(written, Ok(true));
    let guard = transport.lock().unwrap();
    assert_eq!(
        guard.sent_frames(),
        vec![lead, f.arm, f.proof, f.act, f.arm, f.proof, f.act, f.act],
        "only the unit restarts; what the enclosing transaction sent before it stands"
    );
    assert_eq!(
        priorities_of(&guard.sent_frame_settle_us())[1..],
        [DaliPriority::Transaction; 7],
        "the transaction stays open, so the re-run keeps priority 1"
    );
}

#[test]
fn a_unit_inside_a_unit_is_rerun_with_the_outer_one() {
    let f = armed_frames();
    let lead = query_status_frame(ARMED_SHORT);
    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_with_backward(lead, Some(0));
    expect_armed_unit(&mock, &f);
    mock.expect_forward_frame_collision(f.act);
    mock.expect_forward_frame_with_backward(lead, Some(0));
    expect_armed_unit(&mock, &f);
    mock.expect_forward_frame(f.act);
    mock.expect_forward_frame(f.act);
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

    let written = controller.unit(|c| {
        c.send_command(&query_status_command(ARMED_SHORT))?;
        c.unit(arm_prove_and_act)
    });

    assert_eq!(written, Ok(true));
    let guard = transport.lock().unwrap();
    assert_eq!(
        guard.sent_frames(),
        vec![lead, f.arm, f.proof, f.act, lead, f.arm, f.proof, f.act, f.act],
        "the inner unit is a part of the outer one and restarts with it"
    );
    assert_eq!(guard.script_error(), None);
}

#[test]
fn nothing_more_of_a_broken_attempt_reaches_the_wire() {
    let f = armed_frames();
    let lead = query_status_frame(ARMED_SHORT);
    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_collision(f.arm);
    mock.expect_forward_frame(f.arm);
    mock.expect_forward_frame_with_backward(lead, Some(0x42));
    let transport = Arc::new(Mutex::new(mock));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

    let answer = controller.unit(|c| {
        let _ignored = c.send_command(&DaliCommand::Special(SpecialCommand::Dtr0(ARMED_OPERAND)));
        c.send_command(&query_status_command(ARMED_SHORT))
    });

    assert_eq!(answer, Ok(DaliResponse::Answer(0x42)));
    assert_eq!(
        transport.lock().unwrap().sent_frames(),
        vec![f.arm, f.arm, lead],
        "the query of the broken attempt never left: the operand it follows was not staged"
    );
    assert_eq!(
        controller.wire_counters.transaction_reopened.load(Ordering::Relaxed),
        0,
        "the first frame collided, so no transaction was opened to be reopened"
    );
}

#[test]
fn a_unit_refused_on_every_attempt_fails_with_the_wire_error() {
    let f = armed_frames();
    let mock = MockDaliTransport::new();
    for _ in 0..2 {
        expect_armed_unit(&mock, &f);
        mock.expect_forward_frame_collision(f.act);
    }
    let transport = Arc::new(Mutex::new(mock));
    let mut controller =
        DaliController::with_retry_policy(Arc::clone(&transport), test_clock(), retry_policy(2));

    assert_eq!(controller.unit(arm_prove_and_act), Err(FrameError::Collision));
    let guard = transport.lock().unwrap();
    assert_eq!(
        guard.sent_frames(),
        vec![f.arm, f.proof, f.act, f.arm, f.proof, f.act],
        "every attempt is the whole unit, and there are no more attempts than the policy allows"
    );
    assert_eq!(guard.scripted_exchanges_remaining(), 0);
}

struct HoldAfter {
    transport: Arc<Mutex<MockDaliTransport>>,
    frames: usize,
    cause: HoldCause,
}

impl WireHold for HoldAfter {
    fn held(&self) -> Option<HoldCause> {
        let sent = self.transport.lock().unwrap().sent_frames24().len();
        (sent >= self.frames).then_some(self.cause)
    }
}

fn held_after(transport: &Arc<Mutex<MockDaliTransport>>, frames: usize) -> Arc<dyn WireHold> {
    Arc::new(HoldAfter { transport: Arc::clone(transport), frames, cause: HoldCause::Quiescent })
}

const HELD_FRAME: [u8; 3] = [0x07, 0xFE, 0x00];

#[test]
fn a_hold_that_starts_between_units_stops_the_next_frame() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

    let outcomes = controller.with_wire_hold(held_after(&transport, 1), |c| {
        let first = c.transaction(|c| c.send_frame24(HELD_FRAME, false));
        let second = c.transaction(|c| c.send_frame24(HELD_FRAME, false));
        (first, second)
    });

    assert_eq!(outcomes, (Ok(DaliResponse::NoAnswer), Err(Frame24Fault::Held(HoldCause::Quiescent))));
    assert_eq!(transport.lock().unwrap().sent_frames24().len(), 1, "nothing of the second unit left");
}

#[test]
fn a_hold_that_starts_inside_a_started_transaction_lets_it_finish() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

    let (inside, after) = controller.with_wire_hold(held_after(&transport, 1), |c| {
        let inside = c.transaction(|c| {
            c.send_frame24(HELD_FRAME, false)?;
            c.send_frame24(HELD_FRAME, false)
        });
        (inside, c.send_frame24(HELD_FRAME, false))
    });

    assert!(inside.is_ok(), "a started transaction is never broken: {inside:?}");
    assert_eq!(after, Err(Frame24Fault::Held(HoldCause::Quiescent)));
    assert_eq!(transport.lock().unwrap().sent_frames24().len(), 2);
}

#[test]
fn the_hold_ends_with_its_command() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());

    let held = controller.with_wire_hold(held_after(&transport, 0), |c| c.send_frame24(HELD_FRAME, false));
    let free = controller.send_frame24(HELD_FRAME, false);

    assert_eq!(held, Err(Frame24Fault::Held(HoldCause::Quiescent)));
    assert!(free.is_ok());
}

#[test]
fn a_held_16_bit_frame_names_its_cause() {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let mut controller = DaliController::new(Arc::clone(&transport), test_clock());
    let hold: Arc<dyn WireHold> =
        Arc::new(HoldAfter { transport: Arc::clone(&transport), frames: 0, cause: HoldCause::Passive });

    let held = controller.with_wire_hold(hold, |c| c.send_raw(ForwardFrame::new(0xFF, 0x00), false));

    assert_eq!(held, Err(FrameError::Held(HoldCause::Passive)));
    assert!(transport.lock().unwrap().sent_frames().is_empty());
}
