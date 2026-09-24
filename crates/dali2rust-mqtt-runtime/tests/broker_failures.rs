mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use dali2rust_bus::{BusChannel, PublishResult};
use dali2rust_test_support::{remains_false_for, wait_until};
use support::{enabled_settings, runtime_frame, spawn_bridge, StubHaReadPort, StubSettings};

const RECOVERY_WAIT: Duration = Duration::from_secs(20);
const PUBLISH_WAIT: Duration = Duration::from_secs(6);

#[test]
fn a_refused_dial_parks_the_worker_and_a_returning_broker_recovers_it() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(settings, read_port);
    h.mock.fail_next_connects(2);

    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(move || mock.connect_calls() >= 1, RECOVERY_WAIT);
    let mock = std::sync::Arc::clone(&h.mock);
    assert!(
        remains_false_for(move || mock.connect_calls() > 1, Duration::from_millis(1500)),
        "a refused dial was redialled inside the minimum re-dial interval: {} dials",
        h.mock.connect_calls()
    );

    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(move || mock.connect_calls() >= 3, RECOVERY_WAIT);
    let counters = std::sync::Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), RECOVERY_WAIT);

    assert!(h.counters.is_connected(), "third dial succeeds");
    assert!(
        h.mock.connect_calls() >= 3,
        "two refusals then a success: {}",
        h.mock.connect_calls()
    );
    let online = h.mock.published_on("dali/ctl1/availability");
    assert_eq!(
        online.last().map(|m| m.payload.clone()),
        Some(b"online".to_vec()),
        "the session that finally opened announced itself"
    );
}

#[test]
fn a_dial_still_in_flight_is_not_dialled_again() {
    let settings = StubSettings::new(dali2rust_domain::registry::HomeAssistantSettingsView {
        enabled: false,
        ..enabled_settings()
    });
    let h = spawn_bridge(std::sync::Arc::clone(&settings), StubHaReadPort::with_lamp(1));
    h.mock.stall_next_connects(1);
    settings.set(enabled_settings());

    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(move || mock.connect_calls() >= 1, RECOVERY_WAIT);
    let mock = std::sync::Arc::clone(&h.mock);
    assert!(
        remains_false_for(move || mock.connect_calls() > 1, Duration::from_secs(1)),
        "a dial in flight was redialled: {} calls",
        h.mock.connect_calls()
    );
    assert!(
        !h.counters.is_connected(),
        "an unanswered dial is not a session"
    );

    h.mock.set_connected(true);
    let counters = std::sync::Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), RECOVERY_WAIT);
    assert_eq!(h.mock.connect_calls(), 1, "one dial, one session");
}

#[test]
fn a_settings_change_mid_hold_redials_without_waiting_out_the_backoff() {
    let settings = StubSettings::new(enabled_settings());
    let h = spawn_bridge(std::sync::Arc::clone(&settings), StubHaReadPort::with_lamp(1));
    h.mock.fail_next_connects(2);

    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(move || mock.connect_calls() >= 2, RECOVERY_WAIT);

    let patched_at = std::time::Instant::now();
    settings.set(dali2rust_domain::registry::HomeAssistantSettingsView {
        broker_port: 1884,
        ..enabled_settings()
    });

    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(move || mock.connect_calls() >= 3, RECOVERY_WAIT);
    assert!(
        patched_at.elapsed() < Duration::from_secs(4),
        "the PATCH did not void the hold: redial {:?} after it",
        patched_at.elapsed()
    );
    assert_eq!(
        h.mock.session_config().map(|c| c.broker_port),
        Some(1884),
        "the redial used the settings the operator just fixed"
    );
    let counters = std::sync::Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), RECOVERY_WAIT);
}

#[test]
fn a_refused_subscribe_erases_the_retained_birth_before_retrying() {
    let settings = StubSettings::new(dali2rust_domain::registry::HomeAssistantSettingsView {
        enabled: false,
        ..enabled_settings()
    });
    let h = spawn_bridge(std::sync::Arc::clone(&settings), StubHaReadPort::with_lamp(1));
    h.mock.fail_next_subscribes(1);
    settings.set(enabled_settings());

    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(
        move || {
            mock.published_on("dali/ctl1/availability")
                .iter()
                .any(|m| m.payload == b"offline")
        },
        RECOVERY_WAIT,
    );
    let counters = std::sync::Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), RECOVERY_WAIT);
    let avail = h.mock.published_on("dali/ctl1/availability");
    let payloads: Vec<&[u8]> = avail.iter().map(|m| m.payload.as_slice()).collect();
    assert_eq!(
        &payloads[..3],
        &[b"online".as_slice(), b"offline", b"online"],
        "birth, erasure after the refused subscribe, then the retry's birth"
    );
    assert!(
        avail[1].retain,
        "a non-retained erasure leaves the retained birth standing"
    );
}

#[test]
fn a_retain_state_toggle_applies_live_without_tearing_the_session_down() {
    let settings = StubSettings::new(enabled_settings());
    let h = spawn_bridge(std::sync::Arc::clone(&settings), StubHaReadPort::with_lamp(1));
    let counters = std::sync::Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), PUBLISH_WAIT);

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 50)),
        PublishResult::Queued
    );
    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(
        move || {
            mock.published_on("dali/ctl1/a0/vl/1/state")
                .last()
                .is_some_and(|m| m.retain)
        },
        PUBLISH_WAIT,
    );

    settings.set(dali2rust_domain::registry::HomeAssistantSettingsView {
        retain_state: false,
        ..enabled_settings()
    });
    let mock = std::sync::Arc::clone(&h.mock);
    let publisher = h.publisher.clone();
    let mut level = 60u8;
    wait_until(
        move || {
            let _ = publisher.try_publish(BusChannel::Events, runtime_frame(1, level));
            level = level.wrapping_add(1);
            mock.published_on("dali/ctl1/a0/vl/1/state")
                .last()
                .is_some_and(|m| !m.retain)
        },
        PUBLISH_WAIT,
    );
    assert_eq!(
        h.mock.disconnect_calls(),
        0,
        "a per-publish flag must not tear the session down"
    );
    assert_eq!(h.mock.connect_calls(), 1, "one dial, one session");

    settings.set(dali2rust_domain::registry::HomeAssistantSettingsView {
        retain_state: false,
        state_topic_prefix: "dali2".to_string(),
        ..enabled_settings()
    });
    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(move || mock.disconnect_calls() >= 1, RECOVERY_WAIT);
    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(move || mock.connect_calls() >= 2, RECOVERY_WAIT);
}

#[test]
fn a_refused_publish_is_counted_and_the_worker_stays_alive() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(settings, read_port);
    let counters = std::sync::Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), PUBLISH_WAIT);

    h.mock.fail_next_publishes(1);
    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 120)),
        PublishResult::Queued
    );
    let counters = std::sync::Arc::clone(&h.counters);
    wait_until(
        move || counters.publish_failures_total.load(Ordering::Relaxed) >= 1,
        PUBLISH_WAIT,
    );

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 180)),
        PublishResult::Queued
    );
    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(
        move || {
            mock.published_on("dali/ctl1/a0/vl/1/state")
                .last()
                .is_some_and(|m| m.payload.windows(3).any(|w| w == b"180"))
        },
        PUBLISH_WAIT,
    );
    assert!(
        h.mock
            .published_on("homeassistant/light/ctl1/a0_vl_1/config")
            .last()
            .is_some(),
        "the config lost to the refused publish was re-announced"
    );
    assert!(h.counters.publish_failures_total.load(Ordering::Relaxed) >= 1);
}

#[test]
fn a_dropped_session_reconnects_reannounces_and_republishes_discovery() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(settings, read_port);
    let counters = std::sync::Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), PUBLISH_WAIT);

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(1, 100)),
        PublishResult::Queued
    );
    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(
        move || !mock.published_on("homeassistant/light/ctl1/a0_vl_1/config").is_empty(),
        PUBLISH_WAIT,
    );

    h.mock.set_connected(false);
    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(move || mock.connect_calls() >= 2, RECOVERY_WAIT);

    let mock = std::sync::Arc::clone(&h.mock);
    wait_until(
        move || mock.published_on("homeassistant/light/ctl1/a0_vl_1/config").len() >= 2,
        PUBLISH_WAIT,
    );
    assert_eq!(
        h.mock
            .published_on("homeassistant/light/ctl1/a0_vl_1/config")
            .last()
            .map(|m| m.payload.is_empty()),
        Some(false),
        "the re-announce carries the config, not a retraction"
    );
    assert!(
        h.mock
            .published_on("homeassistant/light/ctl1/a0_vl_2/config")
            .iter()
            .any(|m| m.payload.is_empty()),
        "a fresh session retracts the ids the read model no longer exposes"
    );
    assert!(
        h.mock.subscriptions().len() >= 6,
        "three command wildcards per session, twice: {:?}",
        h.mock.subscriptions()
    );
    let online: Vec<_> = h
        .mock
        .published_on("dali/ctl1/availability")
        .into_iter()
        .filter(|m| m.payload == b"online")
        .collect();
    assert!(online.len() >= 2, "each session announces its own birth");
}
