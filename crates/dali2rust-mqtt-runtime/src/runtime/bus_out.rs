use dali2rust_bus::worker_counters::bump;
use dali2rust_bus::{BusChannel, BusFrame, PublishResult};
use dali2rust_contracts::msg::Origin;
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::runtime::worker::MqttWorkerPorts;

pub const MQTT_WORKER_REQUIRED_EVENTS: &[&str] = &[
    "HomeAssistantDiscoveryPublishedEvent",
    "OperationWorkerSignalEvent",
    "MqttRuleMessageEvent",
];

pub(super) fn publish_event<P>(ports: &MqttWorkerPorts, correlation_id: u64, payload: P)
where
    P: Into<dali2rust_contracts::msg::BusEventPayload>,
{
    publish_required_event(
        ports,
        correlation_id,
        payload,
        dali2rust_bus::RequiredPublishCounters::new(
            &ports.counters.terminal_event_publish_retried_total,
            &ports.counters.terminal_event_publish_failed_total,
        ),
    );
}

pub(super) fn publish_required_event<P>(
    ports: &MqttWorkerPorts,
    correlation_id: u64,
    payload: P,
    counters: dali2rust_bus::RequiredPublishCounters<'_>,
) where
    P: Into<dali2rust_contracts::msg::BusEventPayload>,
{
    let envelope = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        ports.bus_id.0,
        Some(Origin::Mqtt),
        payload,
    );
    dali2rust_bus::publish_required_counted(
        &ports.publisher,
        BusChannel::Events,
        BusFrame::event(envelope),
        &dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS,
        dali2rust_bus::REQUIRED_PUBLISH_UNCAPPED,
        "mqtt-bridge",
        counters,
    );
}

pub(super) fn refuse_command(ports: &MqttWorkerPorts, workflow: u64) {
    publish_event(
        ports,
        workflow,
        dali2rust_contracts::msg::OperationWorkerSignalEvent::failed(
            workflow,
            dali2rust_contracts::msg::ErrorCode::ExecutionFailed,
            "ha_bridge_unavailable",
        ),
    );
}

pub(super) fn publish_command<P>(ports: &MqttWorkerPorts, payload: P)
where
    P: Into<dali2rust_contracts::msg::BusCommandPayload>,
{
    let corr = ports.correlation.next_id();
    let envelope = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        corr,
        ports.bus_id.0,
        Some(Origin::Mqtt),
        payload,
    );
    let result = ports
        .publisher
        .try_publish(BusChannel::Commands, BusFrame::command(envelope));
    if result != PublishResult::Queued {
        log::warn!("mqtt: command dropped at bus ingress: {result:?}");
        bump(&ports.counters.commands_ingress_rejected_total);
    }
}
