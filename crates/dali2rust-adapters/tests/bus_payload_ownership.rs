use dali2rust_contracts::msg::{COMMAND_VARIANT_NAMES, EVENT_VARIANT_NAMES};

const COMMAND_OWNERS: &[(&str, &[&str])] = &[
    (
        "rules_worker",
        dali2rust_rules_runtime::RULES_WORKER_HANDLED_COMMANDS,
    ),
    (
        "dali_worker",
        dali2rust_dali_runtime::DALI_WORKER_HANDLED_COMMANDS,
    ),
    (
        "registry_worker",
        dali2rust_registry_runtime::REGISTRY_WORKER_HANDLED_COMMANDS,
    ),
    (
        "operation_tracker",
        dali2rust_operations_runtime::OPERATION_TRACKER_HANDLED_COMMANDS,
    ),
    (
        "apply_orchestrator",
        dali2rust_operations_runtime::APPLY_ORCHESTRATOR_HANDLED_COMMANDS,
    ),
    (
        "hcl_scheduler",
        dali2rust_hcl_runtime::HCL_SCHEDULER_HANDLED_COMMANDS,
    ),
    (
        "mqtt_bridge",
        dali2rust_mqtt_runtime::MQTT_WORKER_HANDLED_COMMANDS,
    ),
    (
        "ota_worker",
        dali2rust_ota_runtime::OTA_WORKER_HANDLED_COMMANDS,
    ),
];

const EVENT_CONSUMERS: &[(&str, &[&str])] = &[
    (
        "rules_worker",
        dali2rust_rules_runtime::RULES_WORKER_HANDLED_EVENTS,
    ),
    (
        "registry_events_worker",
        dali2rust_registry_runtime::REGISTRY_EVENTS_HANDLED_EVENTS,
    ),
    (
        "operation_tracker",
        dali2rust_operations_runtime::OPERATION_TRACKER_HANDLED_EVENTS,
    ),
    (
        "display_worker",
        dali2rust_display_runtime::DISPLAY_WORKER_HANDLED_EVENTS,
    ),
    (
        "apply_orchestrator",
        dali2rust_operations_runtime::APPLY_ORCHESTRATOR_HANDLED_EVENTS,
    ),
    (
        "fanout_projector",
        dali2rust_fanout_runtime::PROJECTOR_HANDLED_EVENTS,
    ),
    (
        "arbitration_worker",
        dali2rust_redundancy_runtime::ARBITRATION_HANDLED_EVENTS,
    ),
    (
        "hcl_scheduler",
        dali2rust_hcl_runtime::HCL_SCHEDULER_HANDLED_EVENTS,
    ),
    (
        "poller",
        dali2rust_poller_runtime::POLLER_HANDLED_EVENTS,
    ),
    (
        "websocket_projector",
        dali2rust_api::ws::WS_PROJECTED_EVENTS,
    ),
    (
        "mqtt_bridge",
        dali2rust_mqtt_runtime::MQTT_WORKER_HANDLED_EVENTS,
    ),
];

const REQUIRED_EVENT_PUBLISHERS: &[(&str, &[&str])] = &[
    (
        "dali_worker",
        dali2rust_dali_runtime::DALI_WORKER_REQUIRED_EVENTS,
    ),
    (
        "registry",
        dali2rust_registry_runtime::REGISTRY_REQUIRED_EVENTS,
    ),
    (
        "mqtt_bridge",
        dali2rust_mqtt_runtime::MQTT_WORKER_REQUIRED_EVENTS,
    ),
    (
        "apply_orchestrator",
        dali2rust_operations_runtime::APPLY_ORCHESTRATOR_REQUIRED_EVENTS,
    ),
    (
        "sniffer_translator",
        dali2rust_fanout_runtime::SNIFFER_TRANSLATOR_REQUIRED_EVENTS,
    ),
    (
        "rules_worker",
        dali2rust_rules_runtime::RULES_WORKER_REQUIRED_EVENTS,
    ),
    (
        "ota_worker",
        dali2rust_ota_runtime::OTA_WORKER_REQUIRED_EVENTS,
    ),
];

const OBSERVED_ONLY_EVENTS: &[(&str, &str)] = &[
    ("DaliEventPayload", "raw wire observability for the sniffer and BDD assertions"),
    ("StatsReportedEvent", "the WS stats channel samples the read ports on a timer"),
    ("DaliDiscoveryCompletedEvent", "operation completion travels by worker signal"),
    ("DaliDiscoveryFailedEvent", "operation completion travels by worker signal"),
    ("PersistenceLoadResultEvent", "boot-only hydrate summary for the diagnostic surface and BDD"),
    (
        "HomeAssistantSettingsChangedEvent",
        "the bridge re-reads its settings from the read port on every pass; a dropped event must not park it",
    ),
    (
        "RedundancySettingsChangedEvent",
        "the arbitration worker re-reads its settings from the read port on every turn",
    ),
    (
        "PoliciesChangedEvent",
        "the policy is read from the port when it is applied, never latched from an event",
    ),
];

fn owners_of<'a>(name: &str, tables: &'a [(&str, &[&str])]) -> Vec<&'a str> {
    tables
        .iter()
        .filter(|(_, handled)| handled.contains(&name))
        .map(|(worker, _)| *worker)
        .collect()
}

#[test]
fn every_command_has_exactly_one_owner() {
    for name in COMMAND_VARIANT_NAMES {
        let owners = owners_of(name, COMMAND_OWNERS);
        assert_eq!(
            owners.len(),
            1,
            "command {name}: owners = {owners:?} (must be exactly one owning worker)"
        );
    }
}

#[test]
fn every_event_has_a_consumer_or_is_documented_observed_only() {
    for name in EVENT_VARIANT_NAMES {
        let consumers = owners_of(name, EVENT_CONSUMERS);
        let observed_only = OBSERVED_ONLY_EVENTS.iter().any(|(entry, _)| entry == name);
        assert!(
            !consumers.is_empty() || observed_only,
            "event {name}: no consumer and not in OBSERVED_ONLY_EVENTS — wire a consumer \
             or document why it is observed-only"
        );
        assert!(
            !observed_only || consumers.is_empty(),
            "event {name}: consumed by {consumers:?} but still listed in \
             OBSERVED_ONLY_EVENTS — remove the stale entry"
        );
    }
}

#[test]
fn handled_lists_contain_only_real_variant_names() {
    for (worker, handled) in COMMAND_OWNERS {
        for name in *handled {
            assert!(
                COMMAND_VARIANT_NAMES.contains(name),
                "{worker}: handled command {name} is not a BusCommandPayload variant"
            );
        }
    }
    for (worker, handled) in EVENT_CONSUMERS {
        for name in *handled {
            assert!(
                EVENT_VARIANT_NAMES.contains(name),
                "{worker}: handled event {name} is not a BusEventPayload variant"
            );
        }
    }
    for (name, reason) in OBSERVED_ONLY_EVENTS {
        assert!(
            EVENT_VARIANT_NAMES.contains(name),
            "OBSERVED_ONLY_EVENTS entry {name} is not a BusEventPayload variant"
        );
        assert!(!reason.is_empty(), "OBSERVED_ONLY_EVENTS entry {name} names no reason");
    }
}

#[test]
fn every_tracker_consumed_event_is_published_required() {
    let required: Vec<&str> = REQUIRED_EVENT_PUBLISHERS
        .iter()
        .flat_map(|(_, kinds)| kinds.iter().copied())
        .collect();
    let missing: Vec<&str> = dali2rust_operations_runtime::OPERATION_TRACKER_HANDLED_EVENTS
        .iter()
        .copied()
        .filter(|kind| !required.contains(kind))
        .collect();
    assert!(
        missing.is_empty(),
        "operation tracker consumes {missing:?}, which no publisher declares as required \
         delivery (ADR-021): a drop at ingress ends the operation by TTL instead"
    );
}

#[test]
fn required_event_names_are_valid() {
    for (publisher, kinds) in REQUIRED_EVENT_PUBLISHERS {
        for kind in *kinds {
            assert!(
                EVENT_VARIANT_NAMES.contains(kind),
                "{publisher} declares required delivery for unknown event kind {kind}"
            );
        }
    }
}

#[test]
fn observed_only_events_are_never_required() {
    let required: Vec<&str> = REQUIRED_EVENT_PUBLISHERS
        .iter()
        .flat_map(|(_, kinds)| kinds.iter().copied())
        .collect();
    for (kind, _) in OBSERVED_ONLY_EVENTS {
        assert!(
            !required.contains(kind),
            "{kind} is observed-only yet declared required: it routes to nobody"
        );
    }
}
