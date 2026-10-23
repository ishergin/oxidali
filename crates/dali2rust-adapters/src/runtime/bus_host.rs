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
const REGISTRY_INBOX: usize = 64;
const DISPLAY_INBOX: usize = 32;
const OPERATION_TRACKER_INBOX: usize = 64;
const APPLY_ORCHESTRATOR_INBOX: usize = 64;
const PROJECTOR_INBOX: usize = 64;
const HCL_SCHEDULER_INBOX: usize = 64;
const POLLER_INBOX: usize = 64;
const ARBITRATION_INBOX: usize = 16;
const SUPERVISOR_INBOX: usize = 4;
const REPLICATION_INBOX: usize = 8;

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
                REGISTRY_INBOX,
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
            display_ev: reg.subscribe_events_named(
                DISPLAY_INBOX,
                DISPLAY_WORKER_HANDLED_EVENTS,
                "display",
            ),
            operation_ev: reg.subscribe_events_named(
                OPERATION_TRACKER_INBOX,
                OPERATION_TRACKER_HANDLED_EVENTS,
                "operation_tracker",
            ),
            apply_ev: reg.subscribe_events_named(
                APPLY_ORCHESTRATOR_INBOX,
                APPLY_ORCHESTRATOR_HANDLED_EVENTS,
                "apply_orchestrator",
            ),
            projector_ev: reg.subscribe_events_named(
                PROJECTOR_INBOX,
                PROJECTOR_HANDLED_EVENTS,
                "fanout_projector",
            ),
            periodic: periodic_channels(reg),
            ota_cmd: Some(reg.subscribe_commands(
                2,
                dali2rust_ota_runtime::OTA_WORKER_HANDLED_COMMANDS,
            )),
            supervisor_ev: Some(reg.subscribe_events_named(
                SUPERVISOR_INBOX,
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
            HCL_SCHEDULER_INBOX,
            HCL_SCHEDULER_HANDLED_COMMANDS,
            HCL_SCHEDULER_HANDLED_EVENTS,
            "hcl_scheduler",
        ),
        hcl_conf: reg.subscribe_confirmations(32),
        poller_ev: reg.subscribe_events_named(POLLER_INBOX, POLLER_HANDLED_EVENTS, "poller"),
        poller_conf: reg.subscribe_confirmations(32),
        rules_cmd: reg.subscribe_commands_and_events_named(
            RULES_INBOX,
            dali2rust_rules_runtime::RULES_WORKER_HANDLED_COMMANDS,
            dali2rust_rules_runtime::RULES_WORKER_HANDLED_EVENTS,
            "rules",
        ),
        arbitration_ev: reg.subscribe_events_named(
            ARBITRATION_INBOX,
            dali2rust_redundancy_runtime::ARBITRATION_HANDLED_EVENTS,
            "arbitration",
        ),
        replication_ev: reg.subscribe_events_named(
            REPLICATION_INBOX,
            dali2rust_redundancy_runtime::REPLICATION_HANDLED_EVENTS,
            "replication",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        spawn_bus_subscribers, APPLY_ORCHESTRATOR_HANDLED_EVENTS, APPLY_ORCHESTRATOR_INBOX,
        ARBITRATION_INBOX, DISPLAY_INBOX, DISPLAY_WORKER_HANDLED_EVENTS,
        HCL_SCHEDULER_HANDLED_EVENTS, HCL_SCHEDULER_INBOX, MQTT_BRIDGE_INBOX,
        NAMED_EVENT_SUBSCRIBERS, OPERATION_TRACKER_HANDLED_EVENTS, OPERATION_TRACKER_INBOX,
        POLLER_HANDLED_EVENTS, POLLER_INBOX, PROJECTOR_HANDLED_EVENTS, PROJECTOR_INBOX,
        REGISTRY_EVENTS_HANDLED_EVENTS, REGISTRY_INBOX, REPLICATION_INBOX, RULES_INBOX,
        SUPERVISOR_INBOX, WS_FANOUT_INBOX,
    };
    use dali2rust_api::ws::WS_PROJECTED_EVENTS;
    use dali2rust_domain::registry::MAX_HCL_SCHEDULES;
    use dali2rust_mqtt_runtime::MQTT_WORKER_HANDLED_EVENTS;
    use dali2rust_redundancy_runtime::{
        ARBITRATION_HANDLED_EVENTS, REPLICATION_HANDLED_EVENTS, SUPERVISOR_HANDLED_EVENTS,
    };
    use dali2rust_rules_runtime::RULES_WORKER_HANDLED_EVENTS;
    use dali2rust_registry_runtime::{
        ANNOUNCEMENTS_PER_PARK, ANNOUNCEMENT_PARK_WINDOW, ANNOUNCEMENT_SHARE_OF_INBOX,
    };

    const PER_RECORD: usize = usize::MAX;
    const ONE_RECORD: usize = 1;
    const SCHEDULES_BEFORE_AND_AFTER: usize = 2 * MAX_HCL_SCHEDULES;

    const ANNOUNCED: &[(&str, usize)] = &[
        ("AdapterSettingsChangedEvent", PER_RECORD),
        ("PhysicalDeviceChangedEvent", PER_RECORD),
        ("VirtualLampChangedEvent", PER_RECORD),
        ("GroupChangedEvent", PER_RECORD),
        ("GroupMatrixChangedEvent", PER_RECORD),
        ("SceneChangedEvent", PER_RECORD),
        ("SceneMatrixChangedEvent", PER_RECORD),
        ("InputDeviceChangedEvent", PER_RECORD),
        ("HclScheduleChangedEvent", SCHEDULES_BEFORE_AND_AFTER),
        ("PollerSettingsChangedEvent", ONE_RECORD),
        ("DaliSettingsChangedEvent", ONE_RECORD),
        ("RedundancySettingsChangedEvent", ONE_RECORD),
        ("PoliciesChangedEvent", ONE_RECORD),
        ("HomeAssistantSettingsChangedEvent", ONE_RECORD),
    ];

    type InboxRow = (&'static str, &'static [&'static str], usize);

    const EVENT_INBOXES: &[InboxRow] = &[
        ("registry", REGISTRY_EVENTS_HANDLED_EVENTS, REGISTRY_INBOX),
        ("display", DISPLAY_WORKER_HANDLED_EVENTS, DISPLAY_INBOX),
        ("operation_tracker", OPERATION_TRACKER_HANDLED_EVENTS, OPERATION_TRACKER_INBOX),
        ("apply_orchestrator", APPLY_ORCHESTRATOR_HANDLED_EVENTS, APPLY_ORCHESTRATOR_INBOX),
        ("fanout_projector", PROJECTOR_HANDLED_EVENTS, PROJECTOR_INBOX),
        ("hcl_scheduler", HCL_SCHEDULER_HANDLED_EVENTS, HCL_SCHEDULER_INBOX),
        ("poller", POLLER_HANDLED_EVENTS, POLLER_INBOX),
        ("rules", RULES_WORKER_HANDLED_EVENTS, RULES_INBOX),
        ("mqtt_bridge", MQTT_WORKER_HANDLED_EVENTS, MQTT_BRIDGE_INBOX),
        ("ws_fanout", WS_PROJECTED_EVENTS, WS_FANOUT_INBOX),
        ("arbitration", ARBITRATION_HANDLED_EVENTS, ARBITRATION_INBOX),
        ("arb-supervisor", SUPERVISOR_HANDLED_EVENTS, SUPERVISOR_INBOX),
        ("replication", REPLICATION_HANDLED_EVENTS, REPLICATION_INBOX),
    ];

    fn most_announced_in_a_park(handled: &[&str]) -> usize {
        let records = ANNOUNCED
            .iter()
            .filter(|(kind, _)| handled.contains(kind))
            .fold(0usize, |sum, (_, records)| sum.saturating_add(*records));
        records.min(ANNOUNCEMENTS_PER_PARK)
    }

    #[test]
    fn a_park_of_reload_announcements_fits_every_named_subscriber_with_room_to_spare() {
        for name in NAMED_EVENT_SUBSCRIBERS {
            let row = EVENT_INBOXES.iter().find(|(subscriber, _, _)| subscriber == name);
            let missing = || panic!("{name} declares no inbox here");
            let (_, handled, inbox) = row.unwrap_or_else(missing);
            let most = most_announced_in_a_park(handled);
            let room = inbox / ANNOUNCEMENT_SHARE_OF_INBOX;
            assert!(most <= room, "{name}: {most} announcements a park against {inbox} slots");
        }
        assert_eq!(EVENT_INBOXES.len(), NAMED_EVENT_SUBSCRIBERS.len(), "a row names no subscriber");
    }

    #[test]
    fn the_workers_that_park_on_their_inbox_park_no_longer_than_the_announcement_park() {
        assert!(dali2rust_ws_runtime::NO_CLIENT_PARK <= ANNOUNCEMENT_PARK_WINDOW);
        assert!(dali2rust_mqtt_runtime::MQTT_LINK_PARK <= ANNOUNCEMENT_PARK_WINDOW);
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

#[cfg(test)]
mod reload_pacing_tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use dali2rust_api::http::router::Router;
    use dali2rust_bsp::slice_store_files::InMemorySliceStore;
    use dali2rust_bus::BusConfig;
    use dali2rust_dali_runtime::DaliRuntimeConfig;
    use dali2rust_platform::slice_store::SliceStore;
    use dali2rust_registry_runtime::{
        encode_persistence_blob, PersistableGroupRecord, PersistableGroupsSlice,
        PersistableVirtualLampsSlice, PersistableVlRecord, PersistenceEnvelope,
        GROUPS_SLICE_VERSION, VIRTUAL_LAMPS_SLICE_VERSION,
    };
    use dali2rust_test_support::{remains_false_for, wait_until};

    use crate::runtime::{build_http_test_stack, BusStackRuntime, HttpTestPorts};
    use crate::dali::transport::mock::MockDaliTransport;

    const ADAPTERS: u8 = 2;
    const LAMPS_PER_ADAPTER: u8 = 64;
    const GROUPS_PER_ADAPTER: u8 = 16;
    const GROUP_MATRICES_PER_ADAPTER: u8 = 1;
    const RECORDS_PER_ADAPTER: u8 =
        LAMPS_PER_ADAPTER + GROUPS_PER_ADAPTER + GROUP_MATRICES_PER_ADAPTER;
    const ANNOUNCED_TO_THE_HUB: u32 = ADAPTERS as u32 * RECORDS_PER_ADAPTER as u32;
    const IMPORT_ACCEPTED_WITHIN: Duration = Duration::from_secs(10);
    const DRAINED_WITHIN: Duration = Duration::from_secs(12);
    const WS_FANOUT: &str = "ws_fanout";
    const ACCEPTED: u16 = 202;

    fn lamps_blob(adapter_id: u8) -> Vec<u8> {
        let lamp = |virtual_lamp_id| PersistableVlRecord {
            virtual_lamp_id,
            name: format!("lamp {virtual_lamp_id}"),
            ha_entity_enabled: true,
            binding_short: None,
        };
        let lamps = (0..LAMPS_PER_ADAPTER).map(lamp).collect();
        let slice = PersistableVirtualLampsSlice { adapter_id, lamps };
        encode_persistence_blob(&PersistenceEnvelope::new(VIRTUAL_LAMPS_SLICE_VERSION, slice))
            .expect("encode lamps")
    }

    fn groups_blob(adapter_id: u8) -> Vec<u8> {
        let group = |group_id| PersistableGroupRecord {
            group_id,
            name: format!("group {group_id}"),
            ha_entity_enabled: true,
        };
        let groups = (0..GROUPS_PER_ADAPTER).map(group).collect();
        let slice = PersistableGroupsSlice { adapter_id, groups, rows: Vec::new() };
        encode_persistence_blob(&PersistenceEnvelope::new(GROUPS_SLICE_VERSION, slice))
            .expect("encode groups")
    }

    fn import(router: &Router, name: &str, body: &[u8]) {
        let uri = format!("/api/v1/config/slices/{}", name.replace('/', "%2F"));
        let deadline = Instant::now() + IMPORT_ACCEPTED_WITHIN;
        let mut status = 0;
        while status != ACCEPTED && Instant::now() < deadline {
            status = router.dispatch("PUT", &uri, body).status;
        }
        assert_eq!(status, ACCEPTED, "the import of {name} was not accepted");
    }

    fn hub_inbox(runtime: &BusStackRuntime) -> (u32, u32) {
        let counters = runtime.event_publisher.counters_snapshot();
        let hub = counters.event_subscribers.iter().find(|s| s.name == WS_FANOUT);
        hub.map_or((0, 0), |s| (s.delivered, s.receiver_overflow))
    }

    #[test]
    fn a_large_import_reaches_a_parked_websocket_hub_without_an_inbox_overflow() {
        let slices: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        let mock = Arc::new(Mutex::new(MockDaliTransport::new()));
        let (router, _hub, runtime) = build_http_test_stack(
            "0.1.0-test",
            mock,
            BusConfig::default(),
            DaliRuntimeConfig::default(),
            Some(slices),
            &[],
            HttpTestPorts::default(),
        );
        let (delivered_before, _) = hub_inbox(&runtime);
        for adapter_id in 0..ADAPTERS {
            import(&router, &format!("a{adapter_id}/virtual_lamps"), &lamps_blob(adapter_id));
            import(&router, &format!("a{adapter_id}/groups"), &groups_blob(adapter_id));
        }
        let overflow = || hub_inbox(&runtime).1;
        let lossless = remains_false_for(|| overflow() > 0, DRAINED_WITHIN);
        assert!(lossless, "the parked hub's inbox overflowed {} times", overflow());
        let delivered = || hub_inbox(&runtime).0.wrapping_sub(delivered_before);
        wait_until(|| delivered() >= ANNOUNCED_TO_THE_HUB, IMPORT_ACCEPTED_WITHIN);
        assert!(delivered() >= ANNOUNCED_TO_THE_HUB, "only {} reached the hub", delivered());
    }
}
