use std::sync::Arc;

use dali2rust_bus::{BusConfig, BusHost, BusId, BusPublisher, CommandArrivalObserver};
use dali2rust_dali_runtime::{WireArrivalObserver, DALI_WORKER_HANDLED_COMMANDS};
use dali2rust_display_runtime::DISPLAY_WORKER_HANDLED_EVENTS;
use dali2rust_fanout_runtime::PROJECTOR_HANDLED_EVENTS;
use dali2rust_hcl_runtime::{HCL_SCHEDULER_HANDLED_COMMANDS, HCL_SCHEDULER_HANDLED_EVENTS};
use dali2rust_operations_runtime::{
    APPLY_ORCHESTRATOR_HANDLED_COMMANDS, APPLY_ORCHESTRATOR_HANDLED_EVENTS,
    OPERATION_TRACKER_HANDLED_COMMANDS, OPERATION_TRACKER_HANDLED_EVENTS,
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
                reg.subscribe_events_named_indexed(128, dali2rust_api::ws::WS_PROJECTED_EVENTS, "ws_fanout"),
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
            operation_conf: reg.subscribe_confirmations(32),
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
                64,
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
            128,
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
    use super::{spawn_bus_subscribers, NAMED_EVENT_SUBSCRIBERS};

    #[test]
    fn the_composed_bus_names_every_event_subscriber_it_registers() {
        let subs = spawn_bus_subscribers(
            dali2rust_bus::BusConfig::default(),
            dali2rust_bus::BusId::default(),
            std::sync::Arc::new(dali2rust_platform::dali::WireActivity::new()),
        );
        let counters = subs.publisher.counters_snapshot();
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
