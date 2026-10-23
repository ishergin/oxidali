use std::sync::Arc;

use dali2rust_bus::{BusConfig, BusHost, BusId, BusPublisher, CommandArrivalObserver};
use dali2rust_dali_runtime::{WireArrivalObserver, DALI_WORKER_HANDLED_COMMANDS};
use dali2rust_display_runtime::DISPLAY_WORKER_HANDLED_EVENTS;
use dali2rust_fanout_runtime::PROJECTOR_HANDLED_EVENTS;
use dali2rust_hcl_runtime::{HCL_SCHEDULER_HANDLED_COMMANDS, HCL_SCHEDULER_HANDLED_EVENTS};
use dali2rust_operations_runtime::{
    APPLY_ORCHESTRATOR_HANDLED_COMMANDS, APPLY_ORCHESTRATOR_HANDLED_EVENTS,
    OPERATION_TRACKER_CONFIRMATION_INBOX, OPERATION_TRACKER_HANDLED_COMMANDS,
    OPERATION_TRACKER_HANDLED_EVENTS,
};
use dali2rust_poller_runtime::POLLER_HANDLED_EVENTS;
use dali2rust_registry_runtime::{
    REGISTRY_EVENTS_HANDLED_EVENTS, REGISTRY_WORKER_HANDLED_COMMANDS,
};

use super::workers::{
    PeriodicChannels, PipelineChannels, ServiceChannels, WorkerChannels, WsInboxPending,
};

pub(crate) struct BusSubscribers {
    pub host: BusHost,
    pub publisher: BusPublisher,
    pub channels: Option<WorkerChannels>,
}

pub const NAMED_EVENT_SUBSCRIBERS: &[&str] = &[
    "registry",
    "display",
    "operation_tracker",
    "apply_orchestrator",
    "fanout_projector",
    "hcl_scheduler",
    "poller",
    "rules",
    "mqtt_bridge",
    "ws_fanout",
    "arbitration",
    "arb-supervisor",
    "replication",
];

const WS_FANOUT_INBOX: usize = 128;
const MQTT_BRIDGE_INBOX: usize = 64;
const RULES_INBOX: usize = 128;

pub(crate) fn spawn_bus_subscribers(
    config: BusConfig,
    bus_id: BusId,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
) -> BusSubscribers {
    let arrival: Arc<dyn CommandArrivalObserver> =
        Arc::new(WireArrivalObserver::new(interactive, bus_id));
    let (host, publisher, channels) =
        BusHost::spawn_with_command_observer(config, Some(arrival), |reg| WorkerChannels {
            pipeline: pipeline_channels(reg),
            services: service_channels(reg),
            ws_ev: WsInboxPending::from(
                reg.subscribe_events_named_indexed(
                    WS_FANOUT_INBOX,
                    dali2rust_api::ws::WS_PROJECTED_EVENTS,
                    "ws_fanout",
                ),
            ),
        });

    BusSubscribers {
        host,
        publisher,
        channels: Some(channels),
    }
}

fn pipeline_channels<S: dali2rust_bus::Sender<dali2rust_bus::BusFrame> + Clone>(
    reg: &mut dali2rust_bus::BusRegistrar<S, dali2rust_bus::BusSubscriberRx>,
) -> PipelineChannels {
    PipelineChannels {
            dali_cmd: reg.subscribe_commands(128, DALI_WORKER_HANDLED_COMMANDS),
            registry: reg.subscribe_commands_and_events_named(
                64,
                REGISTRY_WORKER_HANDLED_COMMANDS,
                REGISTRY_EVENTS_HANDLED_EVENTS,
                "registry",
            ),
    }
}

fn service_channels<S: dali2rust_bus::Sender<dali2rust_bus::BusFrame> + Clone>(
    reg: &mut dali2rust_bus::BusRegistrar<S, dali2rust_bus::BusSubscriberRx>,
) -> ServiceChannels {
    ServiceChannels {
            operation_cmd: reg.subscribe_commands(16, OPERATION_TRACKER_HANDLED_COMMANDS),
            operation_conf: reg.subscribe_confirmations(OPERATION_TRACKER_CONFIRMATION_INBOX),
            apply_cmd: reg.subscribe_commands(4, APPLY_ORCHESTRATOR_HANDLED_COMMANDS),
            confirmations: reg.subscribe_confirmations(32),
            display_ev: reg.subscribe_events_named(32, DISPLAY_WORKER_HANDLED_EVENTS, "display"),
            operation_ev: reg.subscribe_events_named(64, OPERATION_TRACKER_HANDLED_EVENTS, "operation_tracker"),
            apply_ev: reg.subscribe_events_named(64, APPLY_ORCHESTRATOR_HANDLED_EVENTS, "apply_orchestrator"),
            projector_ev: reg.subscribe_events_named(64, PROJECTOR_HANDLED_EVENTS, "fanout_projector"),
            periodic: periodic_channels(reg),
            ota_cmd: Some(reg.subscribe_commands(
                2,
                dali2rust_ota_runtime::OTA_WORKER_HANDLED_COMMANDS,
            )),
            supervisor_ev: Some(reg.subscribe_events_named(
                4,
                dali2rust_redundancy_runtime::SUPERVISOR_HANDLED_EVENTS,
                "arb-supervisor",
            )),
            mqtt_ev: Some(reg.subscribe_commands_and_events_named(
                MQTT_BRIDGE_INBOX,
                dali2rust_mqtt_runtime::MQTT_WORKER_HANDLED_COMMANDS,
                dali2rust_mqtt_runtime::MQTT_WORKER_HANDLED_EVENTS,
                "mqtt_bridge",
            )),
    }
}

fn periodic_channels<S: dali2rust_bus::Sender<dali2rust_bus::BusFrame> + Clone>(
    reg: &mut dali2rust_bus::BusRegistrar<S, dali2rust_bus::BusSubscriberRx>,
) -> PeriodicChannels {
    PeriodicChannels {
        hcl_rx: reg.subscribe_commands_and_events_named(
            64,
            HCL_SCHEDULER_HANDLED_COMMANDS,
            HCL_SCHEDULER_HANDLED_EVENTS,
            "hcl_scheduler",
        ),
        hcl_conf: reg.subscribe_confirmations(32),
        poller_ev: reg.subscribe_events_named(64, POLLER_HANDLED_EVENTS, "poller"),
        poller_conf: reg.subscribe_confirmations(32),
        rules_cmd: reg.subscribe_commands_and_events_named(
            RULES_INBOX,
            dali2rust_rules_runtime::RULES_WORKER_HANDLED_COMMANDS,
            dali2rust_rules_runtime::RULES_WORKER_HANDLED_EVENTS,
            "rules",
        ),
        arbitration_ev: reg.subscribe_events_named(
            16,
            dali2rust_redundancy_runtime::ARBITRATION_HANDLED_EVENTS,
            "arbitration",
        ),
        replication_ev: reg.subscribe_events_named(
            8,
            dali2rust_redundancy_runtime::REPLICATION_HANDLED_EVENTS,
            "replication",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        spawn_bus_subscribers, MQTT_BRIDGE_INBOX, NAMED_EVENT_SUBSCRIBERS, RULES_INBOX,
        WS_FANOUT_INBOX,
    };
    use dali2rust_registry_runtime::{ANNOUNCEMENT_INBOX_FLOOR, ANNOUNCEMENT_PARK_WINDOW};

    const PER_RECORD_ANNOUNCEMENTS: &[&str] = &[
        "AdapterSettingsChangedEvent",
        "PhysicalDeviceChangedEvent",
        "VirtualLampChangedEvent",
        "GroupChangedEvent",
        "GroupMatrixChangedEvent",
        "SceneChangedEvent",
        "SceneMatrixChangedEvent",
        "InputDeviceChangedEvent",
    ];

    fn takes_announcements(handled: &[&str]) -> bool {
        handled.iter().any(|kind| PER_RECORD_ANNOUNCEMENTS.contains(kind))
    }

    #[test]
    fn a_parked_second_of_reload_announcements_fits_every_inbox_that_takes_them() {
        let takers = [
            ("ws_fanout", dali2rust_api::ws::WS_PROJECTED_EVENTS, WS_FANOUT_INBOX),
            ("mqtt_bridge", dali2rust_mqtt_runtime::MQTT_WORKER_HANDLED_EVENTS, MQTT_BRIDGE_INBOX),
            ("rules", dali2rust_rules_runtime::RULES_WORKER_HANDLED_EVENTS, RULES_INBOX),
        ];
        for (name, handled, inbox) in takers {
            assert!(takes_announcements(handled), "{name} no longer takes reload announcements");
            let floor = ANNOUNCEMENT_INBOX_FLOOR;
            assert!(inbox >= floor, "{name}'s inbox of {inbox} is under the floor of {floor}");
        }
        let others = [
            super::DISPLAY_WORKER_HANDLED_EVENTS,
            super::OPERATION_TRACKER_HANDLED_EVENTS,
            super::APPLY_ORCHESTRATOR_HANDLED_EVENTS,
            super::PROJECTOR_HANDLED_EVENTS,
            super::HCL_SCHEDULER_HANDLED_EVENTS,
            super::POLLER_HANDLED_EVENTS,
            dali2rust_redundancy_runtime::SUPERVISOR_HANDLED_EVENTS,
            dali2rust_redundancy_runtime::ARBITRATION_HANDLED_EVENTS,
            dali2rust_redundancy_runtime::REPLICATION_HANDLED_EVENTS,
        ];
        assert!(!others.iter().any(|handled| takes_announcements(handled)), "a new taker");
        assert!(dali2rust_ws_runtime::NO_CLIENT_PARK <= ANNOUNCEMENT_PARK_WINDOW);
    }
    use dali2rust_api::http::diagnostics_state::{
        WORST_COMMAND_SUBSCRIBERS, WORST_CONFIRMATION_SUBSCRIBERS, WORST_EVENT_SUBSCRIBERS,
        WORST_EVENT_SUBSCRIBER_NAME,
    };

    fn composed_bus() -> super::BusSubscribers {
        spawn_bus_subscribers(
            dali2rust_bus::BusConfig::default(),
            dali2rust_bus::BusId::default(),
            std::sync::Arc::new(dali2rust_platform::dali::WireActivity::new()),
        )
    }

    #[test]
    fn the_diagnostics_frame_ceiling_models_the_composed_bus() {
        let counters = composed_bus().publisher.counters_snapshot();
        let rows = [
            ("command", counters.command_subscribers.len(), WORST_COMMAND_SUBSCRIBERS),
            ("confirmation", counters.confirmation_subscribers.len(), WORST_CONFIRMATION_SUBSCRIBERS),
            ("event", counters.event_subscribers.len(), WORST_EVENT_SUBSCRIBERS),
        ];
        for (kind, live, modelled) in rows {
            assert_eq!(
                live, modelled,
                "{live} {kind} subscribers against a modelled {modelled}: move the model \
                 with the bus, and the DiagnosticsSnapshot ceiling test judges the new worst case"
            );
        }
        for subscriber in &counters.event_subscribers {
            assert!(
                subscriber.name.len() <= WORST_EVENT_SUBSCRIBER_NAME.len(),
                "subscriber name {:?} is longer than the modelled worst case",
                subscriber.name
            );
        }
    }

    #[test]
    fn the_composed_bus_names_every_event_subscriber_it_registers() {
        let counters = composed_bus().publisher.counters_snapshot();
        let registered: Vec<&str> = counters
            .event_subscribers
            .iter()
            .map(|s| s.name)
            .collect();

        assert!(
            !registered.iter().any(|name| name.is_empty() || *name == "unnamed"),
            "an anonymous subscriber is an overflow nobody can attribute: {registered:?}"
        );
        let mut got: Vec<&str> = registered.clone();
        let mut want: Vec<&str> = NAMED_EVENT_SUBSCRIBERS.to_vec();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(
            got, want,
            "the declared list and the live registration must be the same set"
        );
    }
}
