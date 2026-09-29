mod support;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_contracts::msg::{BusCommandPayload, BusEventPayload, MqttRuleMessageEvent};
use dali2rust_test_support::{recv_command_matching, recv_event_matching, wait_until};
use dali2rust_domain::registry::HomeAssistantSettingsView;
use support::{enabled_settings, spawn_bridge, Harness, StubHaReadPort, StubSettings};

const WAIT: Duration = Duration::from_secs(8);
const RULE_TOPIC: &str = "home/scene";

fn connected_bridge() -> Harness {
    let h = spawn_bridge(StubSettings::new(enabled_settings()), StubHaReadPort::with_lamp(1));
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT);
    h
}

fn following(h: &Harness, topic: &'static str) {
    let mock = Arc::clone(&h.mock);
    wait_until(move || mock.active_subscriptions().iter().any(|t| t == topic), WAIT);
}

fn next_rule_message(h: &Harness) -> MqttRuleMessageEvent {
    let envelope = recv_event_matching(&h.ev_obs_rx, WAIT, |payload| {
        matches!(payload, BusEventPayload::MqttRuleMessageEvent(_))
    });
    match &envelope.payload {
        BusEventPayload::MqttRuleMessageEvent(event) => event.clone(),
        other => panic!("matched a different event: {other:?}"),
    }
}

#[test]
fn the_first_session_subscribes_the_rule_topics_before_it_reads_connected() {
    let settings = StubSettings::new(HomeAssistantSettingsView { enabled: false, ..enabled_settings() });
    let h = spawn_bridge(Arc::clone(&settings), StubHaReadPort::with_lamp(1));
    h.rule_topics.set(&[RULE_TOPIC]);
    h.mock.hold_subacks();
    settings.set(enabled_settings());
    following(&h, RULE_TOPIC);
    assert!(!h.counters.is_connected(), "every SUBACK is still held");
    h.mock.release_subacks();
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT);
}

#[test]
fn a_topic_added_mid_session_holds_the_gauge_until_its_suback() {
    let h = connected_bridge();
    h.mock.hold_subacks();
    h.rule_topics.set(&[RULE_TOPIC]);
    following(&h, RULE_TOPIC);
    let counters = Arc::clone(&h.counters);
    wait_until(move || !counters.is_connected(), WAIT);
    h.mock.release_subacks();
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT);
}

#[test]
fn a_fresh_session_subscribes_the_rule_topics_again() {
    let h = connected_bridge();
    h.rule_topics.set(&[RULE_TOPIC]);
    following(&h, RULE_TOPIC);
    h.mock.set_connected(false);
    let mock = Arc::clone(&h.mock);
    wait_until(move || mock.connect_calls() >= 2, WAIT);
    following(&h, RULE_TOPIC);
    let subscribed = h.mock.subscriptions().iter().filter(|t| *t == RULE_TOPIC).count();
    assert_eq!(subscribed, 2, "one SUBSCRIBE per session: {:?}", h.mock.subscriptions());
}

#[test]
fn a_topic_the_document_drops_is_unsubscribed_within_the_session() {
    let h = connected_bridge();
    h.rule_topics.set(&[RULE_TOPIC, "home/mode"]);
    following(&h, "home/mode");
    h.rule_topics.set(&["home/mode"]);
    let mock = Arc::clone(&h.mock);
    wait_until(move || mock.unsubscriptions().iter().any(|t| t == RULE_TOPIC), WAIT);
    assert_eq!(h.mock.active_subscriptions().iter().filter(|t| t.starts_with("home/")).count(), 1);
    assert_eq!(h.mock.connect_calls(), 1, "the change rode the live session");
}

#[test]
fn one_message_is_one_event_and_no_command() {
    let h = connected_bridge();
    h.rule_topics.set(&[RULE_TOPIC]);
    following(&h, RULE_TOPIC);
    h.mock.deliver(RULE_TOPIC, b"evening");
    let event = next_rule_message(&h);
    assert_eq!(event.topic.as_str(), RULE_TOPIC);
    assert_eq!(event.payload.as_slice(), b"evening");
    assert!(!event.truncated);
    assert_eq!(h.counters.rule_messages_total.load(Ordering::Relaxed), 1);
    assert_eq!(h.counters.commands_received_total.load(Ordering::Relaxed), 0);
    assert_eq!(h.counters.commands_unroutable_total.load(Ordering::Relaxed), 0);
}

#[test]
fn a_payload_past_the_frame_arrives_cut_and_flagged() {
    let h = connected_bridge();
    h.rule_topics.set(&[RULE_TOPIC]);
    following(&h, RULE_TOPIC);
    let long = vec![b'a'; dali2rust_contracts::msg::MQTT_RULE_PAYLOAD_BYTES + 1];
    h.mock.deliver(RULE_TOPIC, &long);
    let event = next_rule_message(&h);
    assert_eq!(event.payload.as_slice(), &long[..dali2rust_contracts::msg::MQTT_RULE_PAYLOAD_BYTES]);
    assert!(event.truncated);
}

#[test]
fn a_retained_replay_on_subscription_fires_nothing() {
    let h = connected_bridge();
    h.rule_topics.set(&[RULE_TOPIC]);
    following(&h, RULE_TOPIC);
    h.mock.deliver_retained(RULE_TOPIC, b"stale");
    h.mock.deliver(RULE_TOPIC, b"fresh");
    let event = next_rule_message(&h);
    assert_eq!(event.payload.as_slice(), b"fresh", "the replay published nothing before it");
    assert_eq!(h.counters.rule_messages_total.load(Ordering::Relaxed), 1);
}

#[test]
fn a_rule_topic_a_command_filter_covers_is_one_copy_both_rule_message_and_command() {
    let h = connected_bridge();
    let command_topic = "dali/ctl1/a0/vl/1/set";
    let control_topic = "home/control";
    h.rule_topics.set(&[command_topic, control_topic]);
    following(&h, control_topic);
    assert!(
        !h.mock.subscriptions().iter().any(|t| t == command_topic),
        "the command wildcard already delivers it: {:?}",
        h.mock.subscriptions()
    );
    h.mock.broker_publish(command_topic, br#"{"state":"ON","brightness":180}"#);
    h.mock.broker_publish(control_topic, b"after");
    let first = next_rule_message(&h);
    assert_eq!(first.topic.as_str(), command_topic);
    let _ = recv_command_matching(&h.cmd_rx, WAIT, |payload| {
        matches!(payload, BusCommandPayload::DaliSetTargetStateCommand(_))
    });
    let second = next_rule_message(&h);
    assert_eq!(second.topic.as_str(), control_topic, "one copy, then the control");
    assert_eq!(h.counters.rule_messages_total.load(Ordering::Relaxed), 2);
    assert_eq!(h.counters.commands_received_total.load(Ordering::Relaxed), 1);
}

#[test]
fn a_burst_on_one_topic_is_paced_and_its_latest_message_arrives_last() {
    let h = connected_bridge();
    h.rule_topics.set(&[RULE_TOPIC]);
    following(&h, RULE_TOPIC);
    for payload in ["1", "2", "3", "4", "5"] {
        h.mock.broker_publish(RULE_TOPIC, payload.as_bytes());
    }
    let mut seen: Vec<Vec<u8>> = Vec::new();
    while seen.last().map(Vec::as_slice) != Some(&b"5"[..]) {
        seen.push(next_rule_message(&h).payload.as_slice().to_vec());
    }
    assert_eq!(seen[0], b"1", "the first message goes at once");
    let coalesced = h.counters.rule_messages_coalesced_total.load(Ordering::Relaxed);
    assert_eq!(h.counters.rule_messages_total.load(Ordering::Relaxed), 5);
    assert_eq!(
        usize::try_from(coalesced).unwrap() + seen.len(),
        5,
        "every message is published or counted as coalesced: {seen:?}"
    );
}

#[test]
fn a_refused_rule_subscription_holds_connected_down_and_is_counted() {
    let h = connected_bridge();
    h.mock.refuse_next_subacks(1);
    h.rule_topics.set(&[RULE_TOPIC]);
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.subscriptions_refused_total.load(Ordering::Relaxed) == 1, WAIT);
    let counters = Arc::clone(&h.counters);
    wait_until(move || !counters.is_connected(), WAIT);
    assert!(
        !h.mock.active_subscriptions().iter().any(|t| t == RULE_TOPIC),
        "the broker denied the topic, so nothing arrives on it"
    );
}
