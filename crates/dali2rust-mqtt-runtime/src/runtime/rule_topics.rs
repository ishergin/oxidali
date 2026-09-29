use dali2rust_contracts::msg::{
    fixed_text_48, FixedItems, MqttRuleMessageEvent, MQTT_RULE_PAYLOAD_BYTES,
};
use dali2rust_platform::mqtt::{MqttClient, MqttIncoming, MqttQos};

pub trait RuleTopicsReadPort: Send + Sync {
    fn rule_topics_generation(&self) -> u32;

    fn rule_topics(&self) -> Vec<String>;
}

#[derive(Default)]
pub(crate) struct RuleTopicCache {
    generation: Option<u32>,
    topics: Vec<String>,
}

impl RuleTopicCache {
    pub(crate) fn refresh(&mut self, port: &dyn RuleTopicsReadPort) {
        let generation = port.rule_topics_generation();
        if self.generation == Some(generation) {
            return;
        }
        self.topics = port.rule_topics();
        self.generation = Some(generation);
    }

    pub(crate) fn topics(&self) -> &[String] {
        &self.topics
    }
}

pub(crate) fn follow_topics(
    client: &mut dyn MqttClient,
    subscribed: &mut Vec<String>,
    wanted: &[String],
) -> u32 {
    subscribed.retain(|topic| wanted.contains(topic) || client.unsubscribe(topic).is_err());
    let mut sent = 0;
    for topic in wanted {
        if subscribed.contains(topic) {
            continue;
        }
        if client.subscribe(topic, MqttQos::AtMostOnce).is_ok() {
            subscribed.push(topic.clone());
            sent += 1;
        }
    }
    sent
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
        assert_eq!(follow_topics(&mut client, &mut subscribed, &topics(&["a", "b"])), 2);
        assert_eq!(follow_topics(&mut client, &mut subscribed, &topics(&["b", "c"])), 1);
        assert_eq!(mock.active_subscriptions(), topics(&["b", "c"]));
        assert_eq!(mock.unsubscriptions(), topics(&["a"]));
        assert_eq!(
            follow_topics(&mut client, &mut subscribed, &topics(&["b", "c"])),
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
        assert_eq!(follow_topics(&mut client, &mut subscribed, &wanted), 0);
        assert!(!all_followed(&subscribed, &wanted), "a refused topic is not followed");
        assert_eq!(follow_topics(&mut client, &mut subscribed, &wanted), 1);
        assert!(all_followed(&subscribed, &wanted));
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
