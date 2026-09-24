mod support;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusId, PublishResult};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{CapabilityFlagsView, HaGroupView, HaSceneView};
use dali2rust_test_support::{
    command_frame, recv_command_matching, remains_false_for, wait_until,
};
use support::{
    absence_frame, button_press_frame, enabled_settings, registry_event, runtime_frame,
    spawn_bridge, spawn_bridge_with_config, StubHaReadPort, StubSettings,
};

const WAIT_S: Duration = Duration::from_secs(8);

#[test]
fn a_backlog_from_a_disabled_period_is_discarded_not_replayed() {
    let settings = StubSettings::new(dali2rust_domain::registry::HomeAssistantSettingsView {
        enabled: false,
        ..enabled_settings()
    });
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(Arc::clone(&settings) as Arc<StubSettings>, read_port);

    for level in 1..=10u8 {
        assert_eq!(
            h.publisher.try_publish(BusChannel::Events, runtime_frame(1, level)),
            PublishResult::Queued
        );
    }
    let counters = Arc::clone(&h.counters);
    wait_until(
        move || counters.bus_discarded_total.load(Ordering::Relaxed) >= 10,
        WAIT_S,
    );
    assert!(
        h.counters.bus_discarded_total.load(Ordering::Relaxed) >= 10,
        "a disabled bridge must discard, not accumulate"
    );

    settings.set(enabled_settings());
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);
    let mock = Arc::clone(&h.mock);
    assert!(
        remains_false_for(
            move || !mock.published_on("dali/ctl1/a0/vl/1/state").is_empty(),
            Duration::from_secs(2),
        ),
        "the discarded backlog was replayed after enabling"
    );

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 42)),
        PublishResult::Queued
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || !mock.published_on("dali/ctl1/a0/vl/1/state").is_empty(),
        WAIT_S,
    );
    let states = h.mock.published_on("dali/ctl1/a0/vl/1/state");
    assert_eq!(states.len(), 1, "one commit, one publish — no backlog");
    assert!(
        states[0].payload.windows(2).any(|w| w == b"42"),
        "the one publish is the fresh commit: {:?}",
        String::from_utf8_lossy(&states[0].payload)
    );
}

#[test]
fn a_discovery_run_keeps_answering_commands_between_batches() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(settings, read_port);
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    let envelope = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        77,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::HomeAssistantDiscoveryPublishCommand {},
    );
    assert_eq!(
        h.publisher.try_publish(
            BusChannel::Commands,
            dali2rust_bus::BusFrame::command(envelope)
        ),
        PublishResult::Queued
    );
    let counters = Arc::clone(&h.counters);
    wait_until(
        move || counters.discovery_published_total.load(Ordering::Relaxed) >= 1,
        WAIT_S,
    );

    h.mock.deliver("dali/ctl1/a0/vl/1/set", br#"{"state":"ON","brightness":180}"#);
    let _command = recv_command_matching(&h.cmd_rx, WAIT_S, |payload| {
        matches!(
            payload,
            dali2rust_contracts::msg::BusCommandPayload::DaliSetTargetStateCommand(_)
        )
    });

    let counters = Arc::clone(&h.counters);
    wait_until(
        move || counters.discovery_published_total.load(Ordering::Relaxed) >= 80,
        WAIT_S,
    );
    assert!(
        h.counters.discovery_published_total.load(Ordering::Relaxed) >= 80,
        "the run finished: {}",
        h.counters.discovery_published_total.load(Ordering::Relaxed)
    );
    assert_eq!(h.counters.discovery_failed_total.load(Ordering::Relaxed), 0);
}

#[test]
fn an_ingress_refused_light_command_is_counted_as_ingress_rejected_not_unroutable() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge_with_config(
        settings,
        read_port,
        BusConfig {
            commands_ingress: 1,
            ..BusConfig::default()
        },
    );
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flooder = {
        let publisher = h.publisher.clone();
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = publisher.try_publish(BusChannel::Commands, command_frame());
            }
        })
    };
    let counters = Arc::clone(&h.counters);
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || {
            mock.deliver("dali/ctl1/a0/vl/1/set", br#"{"state":"ON","brightness":10}"#);
            counters
                .commands_ingress_rejected_total
                .load(Ordering::Relaxed)
                >= 1
        },
        WAIT_S,
    );
    stop.store(true, Ordering::Relaxed);
    flooder.join().expect("flooder joins");

    assert!(
        h.counters
            .commands_ingress_rejected_total
            .load(Ordering::Relaxed)
            >= 1
    );
    assert_eq!(
        h.counters.commands_unroutable_total.load(Ordering::Relaxed),
        0,
        "an ingress refusal must not read as a topic that matched no entity"
    );
}

#[test]
fn a_group_matrix_change_reannounces_the_groups_config() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    read_port.set_group(HaGroupView {
        group_id: 5,
        name: "Hall".to_string(),
        ha_entity_enabled: true,
        capabilities: CapabilityFlagsView {
            brightness: true,
            ..Default::default()
        },
        state: Default::default(),
    });
    let h = spawn_bridge(settings, Arc::clone(&read_port));
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    assert_eq!(
        h.publisher.try_publish(
            BusChannel::Events,
            registry_event(dali2rust_contracts::msg::GroupMatrixChangedEvent { adapter_id: 0 }),
        ),
        PublishResult::Queued
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || !mock.published_on("homeassistant/light/ctl1/a0_group_5/config").is_empty(),
        WAIT_S,
    );
}

#[test]
fn a_scene_change_refreshes_the_scene_select_options() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    read_port.set_scenes(vec![HaSceneView {
        scene_id: 3,
        name: "Evening".to_string(),
        ha_select_enabled: true,
    }]);
    let h = spawn_bridge(settings, Arc::clone(&read_port));
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    let select_config = "homeassistant/select/ctl1/a0_scene_select/config";
    assert_eq!(
        h.publisher.try_publish(
            BusChannel::Events,
            registry_event(dali2rust_contracts::msg::SceneChangedEvent {
                adapter_id: 0,
                scene_id: 3,
            }),
        ),
        PublishResult::Queued
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || {
            mock.published_on(select_config)
                .last()
                .is_some_and(|m| String::from_utf8_lossy(&m.payload).contains("Evening"))
        },
        WAIT_S,
    );

    read_port.set_scenes(vec![HaSceneView {
        scene_id: 3,
        name: "Morning".to_string(),
        ha_select_enabled: true,
    }]);
    assert_eq!(
        h.publisher.try_publish(
            BusChannel::Events,
            registry_event(dali2rust_contracts::msg::SceneChangedEvent {
                adapter_id: 0,
                scene_id: 3,
            }),
        ),
        PublishResult::Queued
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || {
            mock.published_on(select_config)
                .last()
                .is_some_and(|m| String::from_utf8_lossy(&m.payload).contains("Morning"))
        },
        WAIT_S,
    );
}

#[test]
fn a_refused_select_announce_is_retried_rather_than_recorded_as_done() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    read_port.set_scenes(vec![HaSceneView {
        scene_id: 1,
        name: "Evening".to_string(),
        ha_select_enabled: true,
    }]);
    let h = spawn_bridge(settings, read_port);
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    h.mock.fail_next_publishes(3);
    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 30)),
        PublishResult::Queued
    );
    let counters = Arc::clone(&h.counters);
    wait_until(
        move || counters.publish_failures_total.load(Ordering::Relaxed) >= 3,
        WAIT_S,
    );

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 31)),
        PublishResult::Queued
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || {
            !mock
                .published_on("homeassistant/select/ctl1/a0_scene_select/config")
                .is_empty()
        },
        WAIT_S,
    );
}

#[test]
fn a_lamp_config_is_never_announced_without_its_availability_mqtt030() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(settings, read_port);
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 42)),
        PublishResult::Queued
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || !mock.published_on("homeassistant/light/ctl1/a0_vl_1/config").is_empty(),
        WAIT_S,
    );

    let avail = h.mock.published_on("dali/ctl1/a0/vl/1/availability");
    assert!(
        !avail.is_empty(),
        "a config that lists an availability topic must publish on it"
    );
    assert_eq!(avail[0].payload, b"online".to_vec());
    assert!(avail[0].retain, "availability must be retained");
}

#[test]
fn a_burst_of_commits_for_one_lamp_is_coalesced_before_the_broker_sees_it() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(Arc::clone(&settings) as Arc<StubSettings>, read_port);
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    for level in 1..=60u8 {
        assert_eq!(
            h.publisher.try_publish(BusChannel::Events, runtime_frame(1, level)),
            PublishResult::Queued,
            "the inbox must not shed: that is the defect under test"
        );
    }

    let counters = Arc::clone(&h.counters);
    wait_until(
        move || counters.bus_coalesced_total.load(Ordering::Relaxed) > 0,
        WAIT_S,
    );
    assert!(
        h.counters.bus_coalesced_total.load(Ordering::Relaxed) > 0,
        "sixty commits for one lamp must supersede each other, not queue"
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || {
            mock.published_on("dali/ctl1/a0/vl/1/state")
                .last()
                .is_some_and(|p| String::from_utf8_lossy(&p.payload).contains("\"brightness\":60"))
        },
        WAIT_S,
    );
}

#[test]
fn an_absent_gear_takes_its_bound_lamp_offline() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    read_port.bind_to_short(7);
    let h = spawn_bridge(settings, Arc::clone(&read_port));
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 42)),
        PublishResult::Queued
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || !mock.published_on("dali/ctl1/a0/vl/1/availability").is_empty(),
        WAIT_S,
    );
    assert_eq!(
        h.mock.published_on("dali/ctl1/a0/vl/1/availability")[0].payload,
        b"online".to_vec()
    );

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, absence_frame(7)),
        PublishResult::Queued
    );
    let mock = Arc::clone(&h.mock);
    wait_until(
        move || {
            mock.published_on("dali/ctl1/a0/vl/1/availability")
                .last()
                .is_some_and(|m| m.payload == b"offline".to_vec())
        },
        WAIT_S,
    );

    let avail = h.mock.published_on("dali/ctl1/a0/vl/1/availability");
    assert_eq!(
        avail.last().expect("an availability publish").payload,
        b"offline".to_vec(),
        "an absence commit must republish availability for the bound lamp"
    );
    assert!(
        avail.last().expect("an availability publish").retain,
        "offline must survive a Home Assistant restart"
    );

    let state = h.mock.published_on("dali/ctl1/a0/vl/1/state");
    assert_eq!(
        state.len(),
        1,
        "a read that reached nobody has no level to report"
    );
}

#[test]
fn a_press_announces_the_config_before_publishing_the_event_mqtt032() {
    let mut view = enabled_settings();
    view.expose_input_devices = true;
    let settings = StubSettings::new(view);
    let read_port = StubHaReadPort::with_lamp(1);
    read_port.set_button_device(0, 4);
    let h = spawn_bridge(settings, Arc::clone(&read_port));
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, button_press_frame(0, 2)),
        PublishResult::Queued
    );

    let mock = Arc::clone(&h.mock);
    wait_until(
        move || !mock.published_on("dali/ctl1/a0/in/0/2/state").is_empty(),
        WAIT_S,
    );

    let config = h.mock.published_on("homeassistant/event/ctl1/a0_in0_2/config");
    assert!(
        !config.is_empty(),
        "a press must announce the entity it publishes for, or it lands nowhere"
    );
    let state = h.mock.published_on("dali/ctl1/a0/in/0/2/state");
    assert_eq!(state[0].payload, br#"{"event_type":"short_press"}"#.to_vec());
    assert!(!state[0].retain, "a button event must not be retained");
}
