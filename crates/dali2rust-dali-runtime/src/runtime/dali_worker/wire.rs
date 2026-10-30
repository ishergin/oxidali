use super::*;

use dali2rust_contracts::msg::DaliCommandPayload;

use crate::runtime::executor::held_cause_of;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WireFailure {
    Invalid,
    Held(HoldCause),
    Failed,
}

impl WireFailure {
    fn of(error: &impl core::fmt::Debug) -> Self {
        held_cause_of(error).map_or(Self::Failed, Self::Held)
    }
}

pub(super) fn process_wire_payload(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    correlation_id: u64,
    pl: &DaliCommandPayload,
    counters: &DaliWorkerCounters,
) {
    let Some((backward_u8, status)) = run_wire_payload(controller, publisher, correlation_id, pl, counters)
    else {
        return;
    };

    publish_wire_payload_results(
        publisher,
        correlation_id,
        pl.wire_address,
        pl.command,
        backward_u8,
        status,
        counters,
    );
}

fn run_wire_payload(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    correlation_id: u64,
    pl: &DaliCommandPayload,
    counters: &DaliWorkerCounters,
) -> Option<(u8, DeliveryStatus)> {
    let result = if pl.raw_mode {
        process_raw_payload(
            controller,
            pl.wire_address,
            pl.command,
            pl.repeat_count,
            pl.raw_expects_backward,
        )
    } else {
        process_typed_payload(controller, pl.wire_address, pl.command, pl.repeat_count)
    };
    match result {
        Ok(res) => Some(res),
        Err(WireFailure::Held(cause)) => {
            answer_each_failed(publisher, &[correlation_id], counters, (ErrorCode::Conflict, cause.message()));
            None
        }
        Err(failure) => {
            if failure == WireFailure::Invalid {
                counters.invalid_command.fetch_add(1, Ordering::Relaxed);
            } else {
                counters.execution_failed.fetch_add(1, Ordering::Relaxed);
            }
            emit_execution_failed(publisher, correlation_id, counters);
            None
        }
    }
}

fn process_raw_payload(
    controller: &mut impl DaliApplicationController,
    wire_address: u8,
    command: u8,
    repeat_count: u8,
    raw_expects_backward: bool,
) -> Result<(u8, DeliveryStatus), WireFailure> {
    let frame = ForwardFrame::new(wire_address, command);
    let n = repeat_count.max(1);
    let answer = match n {
        2 => controller
            .send_raw_pair(frame, raw_expects_backward)
            .map(|(response, _)| response)
            .map_err(|error| WireFailure::of(&error))?,
        _ => controller
            .transaction(|controller| {
                let mut answer = DaliResponse::NoAnswer;
                for i in 0..n {
                    let expect = raw_expects_backward && i + 1 == n;
                    let response = controller.send_raw(frame, expect).map_err(|error| WireFailure::of(&error))?;
                    if !matches!(response, DaliResponse::NoAnswer) {
                        answer = response;
                    }
                }
                Ok(answer)
            })?,
    };
    if raw_expects_backward {
        match answer {
            DaliResponse::Answer(value) => Ok((value, DeliveryStatus::Ok)),
            DaliResponse::Violation | DaliResponse::NoAnswer => {
                Ok((0, DeliveryStatus::ExecutionFailed))
            }
        }
    } else {
        Ok((0, DeliveryStatus::Ok))
    }
}

fn process_typed_payload(
    controller: &mut impl DaliApplicationController,
    wire_address: u8,
    command: u8,
    repeat_count: u8,
) -> Result<(u8, DeliveryStatus), WireFailure> {
    let cmd = dali_command_from_wire(wire_address, command, repeat_count)
        .map_err(|_| WireFailure::Invalid)?;

    let response = controller
        .with_wire_class(Some(command_priority(&cmd)), |c| c.send_command(&cmd))
        .map_err(|error| WireFailure::of(&error))?;
    let expects_backward = cmd.is_query();

    match response {
        DaliResponse::Answer(v) => Ok((v, DeliveryStatus::Ok)),
        DaliResponse::Violation => Ok((0, DeliveryStatus::ExecutionFailed)),
        DaliResponse::NoAnswer if expects_backward => Ok((0, DeliveryStatus::ExecutionFailed)),
        DaliResponse::NoAnswer => Ok((0, DeliveryStatus::Ok)),
    }
}

fn publish_wire_payload_results(
    publisher: &BusPublisher,
    correlation_id: u64,
    wire_address: u8,
    command: u8,
    backward_u8: u8,
    status: DeliveryStatus,
    counters: &DaliWorkerCounters,
) {
    let conf =
        build_confirmation_envelope(correlation_id, status, backward_u8, SOURCE_ID_UNSPECIFIED);
    publish_confirmation_typed(publisher, conf, counters);

    let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, 0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliEventPayload { wire_address, command, repeat_count: 1 });
    publish_event_typed(publisher, ev);
}
