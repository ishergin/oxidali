use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::msg::{CommandEnvelope, ErrorCode, OperationType};

use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::http::handlers::common::json_err;
use crate::http::types::HttpResponse;

pub use dali2rust_contracts::msg::{OPERATION_FINISHED_RETENTION_MS, OPERATION_TTL_MS};

fn publish_worker_failed_ingress(
    publisher: &BusPublisher,
    bus_id: BusId,
    workflow: u64,
    message: &str,
) {
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        bus_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::OperationWorkerSignalEvent::failed(
            workflow,
            ErrorCode::CommandsIngressOverload,
            message,
        ),
    );
    let outcome = dali2rust_bus::publish_required(
        publisher,
        BusChannel::Events,
        BusFrame::event(ev),
        &dali2rust_bus::HANDLER_PUBLISH_BACKOFF_MS,
        dali2rust_bus::REQUIRED_PUBLISH_UNCAPPED,
        "operation-dispatch-ingress-failure",
    );
    let _ = outcome;
}

pub use dali2rust_contracts::msg::CONFIG_WRITE_TTL_MS;

pub fn publish_begin_then_chunk_series(
    publisher: &BusPublisher,
    bus_id: BusId,
    workflow: u64,
    op_key: &str,
    frames: Vec<BusFrame>,
) -> Result<(), HttpResponse> {
    let begin = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, workflow, bus_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::OperationBeginCommand::with_defaults(op_key, OperationType::ConfigWrite, 0));
    if publisher.try_publish(BusChannel::Commands, BusFrame::command(begin)) != PublishResult::Queued
    {
        return Err(json_err(503, "commands_ingress_overload"));
    }
    for frame in frames {
        if publisher.try_publish(BusChannel::Commands, frame) != PublishResult::Queued {
            publish_worker_failed_ingress(publisher, bus_id, workflow, "chunk_ingress_overload");
            return Err(json_err(503, "commands_ingress_overload"));
        }
    }
    Ok(())
}

pub fn publish_begin_then_paced_series(
    publisher: &BusPublisher,
    bus_id: BusId,
    workflow: u64,
    op_key: &str,
    frames: Vec<BusFrame>,
) -> Result<(), HttpResponse> {
    let begin = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        bus_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::OperationBeginCommand::with_defaults(
            op_key,
            OperationType::ConfigWrite,
            0,
        ),
    );
    if publisher.try_publish(BusChannel::Commands, BusFrame::command(begin)) != PublishResult::Queued
    {
        return Err(json_err(503, "commands_ingress_overload"));
    }
    for frame in frames {
        let outcome = dali2rust_bus::publish_required(
            publisher,
            BusChannel::Commands,
            frame,
            &dali2rust_bus::HANDLER_PUBLISH_BACKOFF_MS,
            dali2rust_bus::REQUIRED_PUBLISH_UNCAPPED,
            "rules-document-series",
        );
        if !outcome.queued {
            publish_worker_failed_ingress(publisher, bus_id, workflow, "chunk_ingress_overload");
            return Err(json_err(503, "commands_ingress_overload"));
        }
    }
    Ok(())
}

pub fn publish_begin_then_semantic_command_pair(
    publisher: &BusPublisher,
    bus_id: BusId,
    workflow: u64,
    op_key: &str,
    op_type: OperationType,
    semantic: CommandEnvelope,
) -> Result<(), HttpResponse> {
    let begin = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, workflow, bus_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::OperationBeginCommand::with_defaults(op_key, op_type, 0));
    let begin_frame = BusFrame::command(begin);
    if publisher.try_publish(BusChannel::Commands, begin_frame) != PublishResult::Queued {
        return Err(json_err(503, "commands_ingress_overload"));
    }
    let cmd_frame = BusFrame::command(semantic);
    if publisher.try_publish(BusChannel::Commands, cmd_frame) != PublishResult::Queued {
        publish_worker_failed_ingress(
            publisher,
            bus_id,
            workflow,
            "semantic_ingress_overload",
        );
        return Err(json_err(503, "commands_ingress_overload"));
    }
    Ok(())
}
