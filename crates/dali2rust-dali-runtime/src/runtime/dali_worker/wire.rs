use super::*;

pub(super) fn process_wire_payload(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    correlation_id: u64,
    wire_address: u8,
    command: u8,
    repeat_count: u8,
    raw_mode: bool,
    raw_expects_backward: bool,
    counters: &DaliWorkerCounters,
) {
    let Some((backward_u8, status)) = run_wire_payload(
        controller,
        publisher,
        correlation_id,
        wire_address,
        command,
        repeat_count,
        raw_mode,
        raw_expects_backward,
        counters,
    ) else {
        return;
    };

    publish_wire_payload_results(
        publisher,
        correlation_id,
        wire_address,
        command,
        backward_u8,
        status,
        counters,
    );
}

#[allow(clippy::too_many_arguments, reason = "mirrors the dispatch-arm signature")]
fn run_wire_payload(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    correlation_id: u64,
    wire_address: u8,
    command: u8,
    repeat_count: u8,
    raw_mode: bool,
    raw_expects_backward: bool,
    counters: &DaliWorkerCounters,
) -> Option<(u8, DeliveryStatus)> {
    let result = if raw_mode {
        process_raw_payload(controller, wire_address, command, repeat_count, raw_expects_backward)
            .map_err(|_| "execution_failed")
    } else {
        process_typed_payload(controller, wire_address, command, repeat_count)
    };
    match result {
        Ok(res) => Some(res),
        Err(err) => {
            if err == "invalid_command" {
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
) -> Result<(u8, DeliveryStatus), ()> {
    let frame = ForwardFrame::new(wire_address, command);
    let n = repeat_count.max(1);
    let answer = match n {
        2 => controller
            .send_raw_pair(frame, raw_expects_backward)
            .map(|(response, _)| response)
            .map_err(|_| ())?,
        _ => controller
            .transaction(|controller| {
                let mut answer = DaliResponse::NoAnswer;
                for i in 0..n {
                    let expect = raw_expects_backward && i + 1 == n;
                    let Ok(response) = controller.send_raw(frame, expect) else {
                        return Err(());
                    };
                    if !matches!(response, DaliResponse::NoAnswer) {
                        answer = response;
                    }
                }
                Ok(answer)
            })
            .map_err(|_| ())?,
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
) -> Result<(u8, DeliveryStatus), &'static str> {
    let cmd = dali_command_from_wire(wire_address, command, repeat_count)
        .map_err(|_| "invalid_command")?;

    let response = controller
        .with_wire_class(Some(command_priority(&cmd)), |c| c.send_command(&cmd))
        .map_err(|_| "execution_failed")?;
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
