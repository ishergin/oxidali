use dali2rust_bus::{publish_required, BusChannel, BusFrame, BusId, BusPublisher, REQUIRED_PUBLISH_BACKOFF_MS};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{ClusterInputEventObservedEvent, Origin};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};

use super::counters::{bump, ClusterCounters};
use super::shared::HeardInput;

pub const CLUSTER_RX_REQUIRED_EVENTS: &[&str] = &["ClusterInputEventObservedEvent"];

pub(crate) const INPUT_PUBLISH_BUDGET_MS: u32 = 25;

pub(crate) struct InputIntake {
    pub(crate) publisher: BusPublisher,
    pub(crate) bus_id: BusId,
}

impl InputIntake {
    pub(crate) fn take(&self, heard: HeardInput, counters: &ClusterCounters) {
        let envelope =
            event_envelope(SOURCE_ID_UNSPECIFIED, CORRELATION_NONE, self.bus_id.0, Some(Origin::Cluster), observed(heard));
        let frame = BusFrame::event(envelope);
        let outcome = publish_required(
            &self.publisher,
            BusChannel::Events,
            frame,
            &REQUIRED_PUBLISH_BACKOFF_MS,
            INPUT_PUBLISH_BUDGET_MS,
            "cluster-input",
        );
        let counter = if outcome.queued { &counters.input_events_received_total } else { &counters.input_events_dropped_total };
        bump(counter);
    }
}

pub(crate) const fn observed(heard: HeardInput) -> ClusterInputEventObservedEvent {
    let HeardInput { node, event, now_ms } = heard;
    ClusterInputEventObservedEvent {
        node,
        registry_adapter_id: event.address.adapter,
        short_address: event.address.device,
        instance_number: event.address.instance,
        instance_type: event.instance_type,
        instance_groups: event.groups,
        scheme: event.scheme,
        event_info: event.event_info,
        typed: event.kind,
        typed_value: event.value,
        received_at_mono_ms: now_ms,
    }
}
