use dali2rust_contracts::msg::{
    fixed_text_48, FixedItems, MqttRuleMessageEvent, MQTT_RULE_PAYLOAD_BYTES,
};
use dali2rust_platform::mqtt::{MqttClient, MqttIncoming, MqttQos};

pub(crate) const RULE_MESSAGE_INTERVAL_MS: u32 = 100;

pub trait RuleTopicsReadPort: Send + Sync {
    fn rule_topics_generation(&self) -> u32;

    fn rule_topics(&self) -> Vec<String>;
}

#[derive(Default)]
pub(crate) struct RuleTopicCache {
    generation: Option<u32>,
    topics: Vec<String>,
    pub(crate) pacer: RuleMessagePacer,
}

impl RuleTopicCache {
    pub(crate) fn refresh(&mut self, port: &dyn RuleTopicsReadPort) {
        let generation = port.rule_topics_generation();
        if self.generation == Some(generation) {
            return;
        }
        self.topics = port.rule_topics();
        self.pacer.retain(&self.topics);
        self.generation = Some(generation);
    }

    pub(crate) fn topics(&self) -> &[String] {
        &self.topics
    }
}

pub(crate) enum Paced {
    Now(MqttIncoming),
    Held { superseded: bool },
}

struct PacedTopic {
    topic: String,
    released_at_ms: u32,
    held: Option<MqttIncoming>,
}

impl PacedTopic {
    fn due(&self, now_ms: u32) -> bool {
        now_ms.wrapping_sub(self.released_at_ms) >= RULE_MESSAGE_INTERVAL_MS
    }
}

#[derive(Default)]
pub(crate) struct RuleMessagePacer {
    slots: Vec<PacedTopic>,
}

impl RuleMessagePacer {
    pub(crate) fn offer(&mut self, message: MqttIncoming, now_ms: u32) -> Paced {
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.topic == message.topic) else {
            self.slots.push(PacedTopic { topic: message.topic.clone(), released_at_ms: now_ms, held: None });
            return Paced::Now(message);
        };
        if slot.held.is_none() && slot.due(now_ms) {
            slot.released_at_ms = now_ms;
            return Paced::Now(message);
        }
        Paced::Held { superseded: slot.held.replace(message).is_some() }
    }

    pub(crate) fn take_due(&mut self, now_ms: u32) -> Vec<MqttIncoming> {
        let mut due = Vec::new();
        for slot in self.slots.iter_mut().filter(|slot| slot.held.is_some() && slot.due(now_ms)) {
            slot.released_at_ms = now_ms;
            due.extend(slot.held.take());
        }
        due
    }

    pub(crate) fn wait_ms(&self, now_ms: u32, cap_ms: u32) -> u32 {
        self.slots
            .iter()
            .filter(|slot| slot.held.is_some())
            .map(|slot| RULE_MESSAGE_INTERVAL_MS.saturating_sub(now_ms.wrapping_sub(slot.released_at_ms)))
            .fold(cap_ms, u32::min)
    }

    pub(crate) fn retain(&mut self, topics: &[String]) {
        self.slots.retain(|slot| topics.contains(&slot.topic));
    }
}

pub(crate) fn follow_topics(
    client: &mut dyn MqttClient,
    followed: &mut Vec<String>,
    wanted: &[String],
    covering: &[String],
) -> u32 {
    let covered = |topic: &str| covering.iter().any(|filter| topic_filter_matches(filter, topic));
    followed.retain(|topic| {
        wanted.contains(topic) || (!covered(topic) && client.unsubscribe(topic).is_err())
    });
    let mut sent = 0;
    for topic in wanted {
        if followed.contains(topic) {
            continue;
        }
        let own = !covered(topic);
        if own && client.subscribe(topic, MqttQos::AtMostOnce).is_err() {
            continue;
        }
        sent += u32::from(own);
        followed.push(topic.clone());
    }
    sent
}

pub(crate) fn topic_filter_matches(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && filter.starts_with(['+', '#']) {
        return false;
    }
    let mut levels = topic.split('/');
    for part in filter.split('/') {
        if part == "#" {
            return true;
        }
        let Some(level) = levels.next() else {
            return false;
        };
        if part != "+" && part != level {
            return false;
        }
    }
    levels.next().is_none()
}

pub(crate) fn all_followed(subscribed: &[String], wanted: &[String]) -> bool {
    wanted.iter().all(|topic| subscribed.contains(topic))
}

pub(crate) fn message_event(message: &MqttIncoming) -> MqttRuleMessageEvent {
    let mut payload = FixedItems::new();
    for byte in message.payload.iter().take(MQTT_RULE_PAYLOAD_BYTES) {
        let _ = payload.push(*byte);
    }
    MqttRuleMessageEvent {
        topic: fixed_text_48(&message.topic),
        payload,
        truncated: message.payload.len() > MQTT_RULE_PAYLOAD_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{MockMqttClient, MockMqttHandle};
    use dali2rust_platform::mqtt::MqttSessionConfig;
    use std::sync::Arc;

    fn connected() -> (Arc<MockMqttClient>, MockMqttHandle) {
        let (mock, _rx) = MockMqttClient::new();
        let mut client = MockMqttHandle::new(Arc::clone(&mock));
        client
            .connect(&MqttSessionConfig {
                broker_host: "broker".to_string(),
                broker_port: 1883,
                client_id: "ctl1".to_string(),
                username: String::new(),
                password: String::new(),
                keep_alive: std::time::Duration::from_secs(30),
                last_will: None,
            })
            .expect("the mock connects");
        (mock, client)
    }

    fn topics(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn a_document_change_subscribes_the_new_topics_and_drops_the_gone_ones() {
        let (mock, mut client) = connected();
        let mut subscribed = Vec::new();
        assert_eq!(follow_topics(&mut client, &mut subscribed, &topics(&["a", "b"]), &[]), 2);
        assert_eq!(follow_topics(&mut client, &mut subscribed, &topics(&["b", "c"]), &[]), 1);
        assert_eq!(mock.active_subscriptions(), topics(&["b", "c"]));
        assert_eq!(mock.unsubscriptions(), topics(&["a"]));
        assert_eq!(
            follow_topics(&mut client, &mut subscribed, &topics(&["b", "c"]), &[]),
            0,
            "an unchanged document sends nothing"
        );
    }

    #[test]
    fn a_refused_subscription_is_retried_and_holds_the_set_incomplete() {
        let (mock, mut client) = connected();
        let mut subscribed = Vec::new();
        let wanted = topics(&["a"]);
        mock.fail_next_subscribes(1);
        assert_eq!(follow_topics(&mut client, &mut subscribed, &wanted, &[]), 0);
        assert!(!all_followed(&subscribed, &wanted), "a refused topic is not followed");
        assert_eq!(follow_topics(&mut client, &mut subscribed, &wanted, &[]), 1);
        assert!(all_followed(&subscribed, &wanted));
    }

    #[test]
    fn a_topic_a_filter_covers_is_followed_without_its_own_subscription() {
        let (mock, mut client) = connected();
        let covering = topics(&["dali/ctl1/+/vl/+/set"]);
        let wanted = topics(&["dali/ctl1/a0/vl/1/set"]);
        let mut followed = Vec::new();
        assert_eq!(follow_topics(&mut client, &mut followed, &wanted, &covering), 0);
        assert!(all_followed(&followed, &wanted));
        assert!(mock.subscriptions().is_empty(), "no SUBSCRIBE for a covered topic");
        follow_topics(&mut client, &mut followed, &[], &covering);
        assert!(followed.is_empty());
        assert!(mock.unsubscriptions().is_empty(), "no UNSUBSCRIBE for what was never subscribed");
    }

    #[test]
    fn a_filter_matches_topics_by_mqtt_levels() {
        let matching = [
            ("dali/ctl1/+/vl/+/set", "dali/ctl1/a0/vl/1/set"),
            ("sport/#", "sport"),
            ("sport/#", "sport/tennis/player1"),
            ("sport/+", "sport/"),
            ("#", "home/mode"),
            ("home/mode", "home/mode"),
        ];
        for (filter, topic) in matching {
            assert!(topic_filter_matches(filter, topic), "{filter} should match {topic}");
        }
        let apart = [
            ("dali/ctl1/+/vl/+/set", "dali/ctl1/a0/vl/1/state"),
            ("dali/ctl1/+/vl/+/set", "dali/ctl1/a0/vl/1/set/x"),
            ("+", "/finance"),
            ("sport/+", "sport"),
            ("#", "$SYS/uptime"),
            ("+/uptime", "$SYS/uptime"),
            ("home/mode", "home/mode/x"),
        ];
        for (filter, topic) in apart {
            assert!(!topic_filter_matches(filter, topic), "{filter} should not match {topic}");
        }
    }

    fn message(topic: &str, payload: &str) -> MqttIncoming {
        MqttIncoming { topic: topic.to_string(), payload: payload.as_bytes().to_vec(), retained: false }
    }

    fn payload_of(paced: &Paced) -> Option<&[u8]> {
        match paced {
            Paced::Now(message) => Some(&message.payload),
            Paced::Held { .. } => None,
        }
    }

    #[test]
    fn a_topic_publishes_at_most_once_per_interval_and_the_latest_waiting_message_wins() {
        let mut pacer = RuleMessagePacer::default();
        assert_eq!(payload_of(&pacer.offer(message("a", "1"), 1_000)), Some(&b"1"[..]));
        assert!(matches!(pacer.offer(message("a", "2"), 1_010), Paced::Held { superseded: false }));
        assert!(matches!(pacer.offer(message("a", "3"), 1_020), Paced::Held { superseded: true }));
        assert_eq!(payload_of(&pacer.offer(message("b", "x"), 1_030)), Some(&b"x"[..]), "topics pace apart");
        assert_eq!(pacer.wait_ms(1_050, 100), 50, "wake when the waiting message falls due");
        assert!(pacer.take_due(1_099).is_empty());
        let due = pacer.take_due(1_100);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].payload, b"3");
        assert_eq!(pacer.wait_ms(1_100, 100), 100, "nothing waits");
        assert!(matches!(pacer.offer(message("a", "4"), 1_150), Paced::Held { superseded: false }));
        assert_eq!(payload_of(&pacer.offer(message("a", "5"), 2_000)), None, "a waiting message goes first");
        assert_eq!(pacer.take_due(2_000)[0].payload, b"5");
    }

    #[test]
    fn a_topic_the_document_dropped_loses_its_waiting_message() {
        let mut pacer = RuleMessagePacer::default();
        let _ = pacer.offer(message("a", "1"), 0);
        let _ = pacer.offer(message("a", "2"), 10);
        pacer.retain(&topics(&["b"]));
        assert!(pacer.take_due(1_000).is_empty());
        assert_eq!(payload_of(&pacer.offer(message("a", "3"), 1_000)), Some(&b"3"[..]));
    }

    #[test]
    fn a_long_payload_travels_cut_to_the_frame_and_says_so() {
        let fits = MqttIncoming {
            topic: "home/scene".to_string(),
            payload: vec![b'a'; MQTT_RULE_PAYLOAD_BYTES],
            retained: false,
        };
        let event = message_event(&fits);
        assert_eq!(event.topic.as_str(), "home/scene");
        assert_eq!(event.payload.as_slice(), fits.payload.as_slice());
        assert!(!event.truncated, "exactly the frame's worth is whole");

        let long = MqttIncoming { payload: vec![b'a'; MQTT_RULE_PAYLOAD_BYTES + 1], ..fits };
        let event = message_event(&long);
        assert_eq!(event.payload.len(), MQTT_RULE_PAYLOAD_BYTES);
        assert!(event.truncated);
    }
}
