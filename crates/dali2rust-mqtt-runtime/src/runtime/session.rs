use std::time::Duration;

use dali2rust_api::ha::HaTopics;
use dali2rust_domain::registry::HomeAssistantSettingsView;
use dali2rust_platform::mqtt::{
    MqttClient, MqttError, MqttLastWill, MqttQos, MqttSessionConfig,
};

pub const PAYLOAD_ONLINE: &[u8] = b"online";
pub const PAYLOAD_OFFLINE: &[u8] = b"offline";

pub const KEEP_ALIVE: Duration = Duration::from_secs(30);

pub fn topics_of(settings: &HomeAssistantSettingsView) -> HaTopics {
    HaTopics::new(
        &settings.discovery_prefix,
        &settings.state_topic_prefix,
        &settings.controller_id,
    )
}

pub fn qos_of(settings: &HomeAssistantSettingsView) -> MqttQos {
    if settings.publish_qos == 0 {
        MqttQos::AtMostOnce
    } else {
        MqttQos::AtLeastOnce
    }
}

pub fn session_config(
    settings: &HomeAssistantSettingsView,
    password: String,
    topics: &HaTopics,
) -> MqttSessionConfig {
    MqttSessionConfig {
        broker_host: settings.broker_host.clone(),
        broker_port: settings.broker_port,
        client_id: settings.controller_id.clone(),
        username: settings.broker_username.clone(),
        password,
        keep_alive: KEEP_ALIVE,
        last_will: Some(MqttLastWill {
            topic: topics.availability_topic(),
            payload: PAYLOAD_OFFLINE.to_vec(),
            qos: MqttQos::AtLeastOnce,
            retain: true,
        }),
    }
}

pub fn announce_and_subscribe(
    client: &mut dyn MqttClient,
    topics: &HaTopics,
) -> Result<(), MqttError> {
    client.publish(
        &topics.availability_topic(),
        PAYLOAD_ONLINE,
        MqttQos::AtLeastOnce,
        true,
    )?;
    for filter in topics.command_subscriptions() {
        client.subscribe(&filter, MqttQos::AtLeastOnce)?;
    }
    Ok(())
}

pub fn announce_offline(client: &mut dyn MqttClient, topics: &HaTopics) {
    let _ = client.publish(
        &topics.availability_topic(),
        PAYLOAD_OFFLINE,
        MqttQos::AtLeastOnce,
        true,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{MockMqttClient, MockMqttHandle};
    use std::sync::Arc;

    fn settings() -> HomeAssistantSettingsView {
        HomeAssistantSettingsView {
            enabled: true,
            broker_host: "broker".to_string(),
            broker_port: 1883,
            controller_id: "ctl1".to_string(),
            discovery_prefix: "homeassistant".to_string(),
            state_topic_prefix: "dali".to_string(),
            publish_qos: 1,
            ..Default::default()
        }
    }

    #[test]
    fn the_will_is_part_of_the_session_and_is_always_retained() {
        let s = settings();
        let topics = topics_of(&s);
        let cfg = session_config(&s, "secret".to_string(), &topics);
        let will = cfg.last_will.expect("a session without a will lies on reset");
        assert_eq!(will.topic, "dali/ctl1/availability");
        assert_eq!(will.payload, PAYLOAD_OFFLINE);
        assert!(will.retain);
        assert_eq!(cfg.keep_alive, KEEP_ALIVE);
        assert_eq!(cfg.client_id, "ctl1");
        assert_eq!(cfg.password, "secret");
    }

    #[test]
    fn the_birth_message_is_published_before_any_subscription() {
        let (mock, _rx) = MockMqttClient::new();
        let mut client = MockMqttHandle::new(Arc::clone(&mock));
        let s = settings();
        let topics = topics_of(&s);
        client.connect(&session_config(&s, String::new(), &topics)).unwrap();
        announce_and_subscribe(&mut client, &topics).unwrap();

        let birth = mock.published_on("dali/ctl1/availability");
        assert_eq!(birth.len(), 1);
        assert_eq!(birth[0].payload, PAYLOAD_ONLINE);
        assert!(birth[0].retain, "a non-retained birth is invisible to a later client");
        assert_eq!(mock.subscriptions().len(), 3, "three wildcards, not one per entity");
    }

    #[test]
    fn a_deliberate_disable_says_offline_rather_than_relying_on_the_will() {
        let (mock, _rx) = MockMqttClient::new();
        let mut client = MockMqttHandle::new(Arc::clone(&mock));
        let s = settings();
        let topics = topics_of(&s);
        client.connect(&session_config(&s, String::new(), &topics)).unwrap();
        announce_offline(&mut client, &topics);
        let last = mock
            .published_on("dali/ctl1/availability")
            .pop()
            .expect("an availability publish");
        assert_eq!(last.payload, PAYLOAD_OFFLINE);
    }

    #[test]
    fn publish_qos_zero_is_honoured_rather_than_silently_upgraded() {
        let mut s = settings();
        s.publish_qos = 0;
        assert_eq!(qos_of(&s), MqttQos::AtMostOnce);
        s.publish_qos = 1;
        assert_eq!(qos_of(&s), MqttQos::AtLeastOnce);
    }
}
