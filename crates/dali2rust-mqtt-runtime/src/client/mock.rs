use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_platform::mqtt::{
    MqttClient, MqttConnectionState, MqttError, MqttIncoming, MqttLastWill, MqttLink, MqttQos,
    MqttSessionConfig,
};

use crate::runtime::rule_topics::{topic_filter_matches, RuleTopicsReadPort};

const REFUSAL_REPORT_LATENCY: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: MqttQos,
    pub retain: bool,
}

#[derive(Debug, Default)]
struct MockInner {
    published: Vec<PublishedMessage>,
    subscriptions: Vec<String>,
    unsubscriptions: Vec<String>,
    active: Vec<String>,
    session: Option<MqttSessionConfig>,
    connect_calls: u32,
    disconnect_calls: u32,
    fail_publishes: u32,
    fail_subscribes: u32,
    hold_subacks: bool,
    held_subacks: u32,
    refuse_subacks: u32,
    fail_connects: u32,
    stall_connects: u32,
    connected: bool,
}

#[derive(Debug)]
pub struct MockMqttClient {
    inner: Mutex<MockInner>,
    link: Arc<MqttLink>,
}

impl MockMqttClient {
    pub fn bundle() -> (Arc<Self>, dali2rust_platform::mqtt::MqttClientBundle) {
        let (mock, incoming) = Self::new();
        let bundle = dali2rust_platform::mqtt::MqttClientBundle {
            client: Box::new(MockMqttHandle::new(Arc::clone(&mock))),
            incoming,
        };
        (mock, bundle)
    }

    pub fn new() -> (Arc<Self>, std::sync::mpsc::Receiver<MqttIncoming>) {
        let (link, rx) = MqttLink::new();
        (
            Arc::new(Self {
                inner: Mutex::new(MockInner::default()),
                link,
            }),
            rx,
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn published(&self) -> Vec<PublishedMessage> {
        self.lock().published.clone()
    }

    pub fn published_on(&self, topic: &str) -> Vec<PublishedMessage> {
        self.lock()
            .published
            .iter()
            .filter(|m| m.topic == topic)
            .cloned()
            .collect()
    }

    pub fn hold_subacks(&self) {
        self.lock().hold_subacks = true;
    }

    pub fn release_subacks(&self) {
        let held = {
            let mut g = self.lock();
            g.hold_subacks = false;
            core::mem::take(&mut g.held_subacks)
        };
        for _ in 0..held {
            self.link.note_subscription_acked();
        }
    }

    pub fn subscriptions(&self) -> Vec<String> {
        self.lock().subscriptions.clone()
    }

    pub fn unsubscriptions(&self) -> Vec<String> {
        self.lock().unsubscriptions.clone()
    }

    pub fn active_subscriptions(&self) -> Vec<String> {
        self.lock().active.clone()
    }

    pub fn covers(&self, topic: &str) -> bool {
        self.matching_subscriptions(topic) > 0
    }

    pub fn broker_publish(&self, topic: &str, payload: &[u8]) {
        for _ in 0..self.matching_subscriptions(topic) {
            self.deliver(topic, payload);
        }
    }

    fn matching_subscriptions(&self, topic: &str) -> usize {
        let g = self.lock();
        g.active.iter().filter(|filter| topic_filter_matches(filter, topic)).count()
    }

    pub fn session_config(&self) -> Option<MqttSessionConfig> {
        self.lock().session.clone()
    }

    pub fn last_will(&self) -> Option<MqttLastWill> {
        self.lock().session.as_ref().and_then(|s| s.last_will.clone())
    }

    pub fn connect_calls(&self) -> u32 {
        self.lock().connect_calls
    }

    pub fn disconnect_calls(&self) -> u32 {
        self.lock().disconnect_calls
    }

    pub fn deliver(&self, topic: &str, payload: &[u8]) {
        self.deliver_flagged(topic, payload, false);
    }

    pub fn deliver_retained(&self, topic: &str, payload: &[u8]) {
        self.deliver_flagged(topic, payload, true);
    }

    fn deliver_flagged(&self, topic: &str, payload: &[u8], retained: bool) {
        self.link.deliver(MqttIncoming {
            topic: topic.to_string(),
            payload: payload.to_vec(),
            retained,
        });
    }

    pub fn set_connected(&self, up: bool) {
        {
            let mut g = self.lock();
            g.connected = up;
            g.active.clear();
        }
        self.link.set_state(if up {
            MqttConnectionState::Connected
        } else {
            MqttConnectionState::Disconnected
        });
    }

    pub fn fail_next_publishes(&self, n: u32) {
        self.lock().fail_publishes = n;
    }

    pub fn fail_next_subscribes(&self, n: u32) {
        self.lock().fail_subscribes = n;
    }

    pub fn refuse_next_subacks(&self, n: u32) {
        self.lock().refuse_subacks = n;
    }

    pub fn fail_next_connects(&self, n: u32) {
        self.lock().fail_connects = n;
    }

    pub fn stall_next_connects(&self, n: u32) {
        self.lock().stall_connects = n;
    }

    pub fn clear(&self) {
        let mut g = self.lock();
        g.published.clear();
        g.subscriptions.clear();
        g.unsubscriptions.clear();
    }
}

#[derive(Debug)]
pub struct MockMqttHandle(Arc<MockMqttClient>);

impl MockMqttHandle {
    pub fn new(mock: Arc<MockMqttClient>) -> Self {
        Self(mock)
    }
}

impl MqttClient for MockMqttHandle {
    fn connect(&mut self, config: &MqttSessionConfig) -> Result<(), MqttError> {
        {
            let mut g = self.0.lock();
            g.connect_calls += 1;
            if g.fail_connects > 0 {
                g.fail_connects -= 1;
                drop(g);
                self.0.link.set_state(MqttConnectionState::Connecting);
                let link = Arc::clone(&self.0.link);
                std::thread::spawn(move || {
                    // sleep-ok: models the transport's own error latency
                    std::thread::sleep(REFUSAL_REPORT_LATENCY);
                    link.set_state(MqttConnectionState::Disconnected);
                });
                return Ok(());
            }
            if g.stall_connects > 0 {
                g.stall_connects -= 1;
                drop(g);
                self.0.link.set_state(MqttConnectionState::Connecting);
                return Ok(());
            }
            g.session = Some(config.clone());
            g.connected = true;
            g.active.clear();
        }
        self.0.link.set_state(MqttConnectionState::Connected);
        Ok(())
    }

    fn disconnect(&mut self) {
        {
            let mut g = self.0.lock();
            g.disconnect_calls += 1;
            g.connected = false;
            g.active.clear();
        }
        self.0.link.set_state(MqttConnectionState::Disconnected);
    }

    fn publish(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: MqttQos,
        retain: bool,
    ) -> Result<(), MqttError> {
        let mut g = self.0.lock();
        if !g.connected {
            return Err(MqttError::NotConnected);
        }
        if g.fail_publishes > 0 {
            g.fail_publishes -= 1;
            return Err(MqttError::Rejected(-1));
        }
        g.published.push(PublishedMessage {
            topic: topic.to_string(),
            payload: payload.to_vec(),
            qos,
            retain,
        });
        Ok(())
    }

    fn subscribe(&mut self, topic_filter: &str, qos: MqttQos) -> Result<(), MqttError> {
        let mut g = self.0.lock();
        if !g.connected {
            return Err(MqttError::NotConnected);
        }
        if g.fail_subscribes > 0 {
            g.fail_subscribes -= 1;
            return Err(MqttError::Rejected(-1));
        }
        let _ = qos;
        g.subscriptions.push(topic_filter.to_string());
        if g.refuse_subacks > 0 {
            g.refuse_subacks -= 1;
            self.0.link.note_subscription_refused();
            return Ok(());
        }
        if !g.active.iter().any(|t| t == topic_filter) {
            g.active.push(topic_filter.to_string());
        }
        if g.hold_subacks {
            g.held_subacks += 1;
        } else {
            self.0.link.note_subscription_acked();
        }
        Ok(())
    }

    fn unsubscribe(&mut self, topic_filter: &str) -> Result<(), MqttError> {
        let mut g = self.0.lock();
        if !g.connected {
            return Err(MqttError::NotConnected);
        }
        g.unsubscriptions.push(topic_filter.to_string());
        g.active.retain(|t| t != topic_filter);
        Ok(())
    }

    fn link(&self) -> Arc<MqttLink> {
        Arc::clone(&self.0.link)
    }
}

#[derive(Debug, Default)]
pub struct MockRuleTopics {
    generation: AtomicU32,
    topics: Mutex<Vec<String>>,
}

impl MockRuleTopics {
    pub fn set(&self, topics: &[&str]) {
        *self.topics.lock().unwrap_or_else(|e| e.into_inner()) =
            topics.iter().map(|topic| (*topic).to_string()).collect();
        self.generation.fetch_add(1, Ordering::Release);
    }
}

impl RuleTopicsReadPort for MockRuleTopics {
    fn rule_topics_generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    fn rule_topics(&self) -> Vec<String> {
        self.topics.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> MqttSessionConfig {
        MqttSessionConfig {
            broker_host: "broker".to_string(),
            broker_port: 1883,
            client_id: "ctl1".to_string(),
            username: String::new(),
            password: String::new(),
            keep_alive: std::time::Duration::from_secs(30),
            last_will: Some(MqttLastWill {
                topic: "dali/ctl1/availability".to_string(),
                payload: b"offline".to_vec(),
                qos: MqttQos::AtLeastOnce,
                retain: true,
            }),
        }
    }

    #[test]
    fn the_last_will_is_observable_so_a_test_can_prove_it_was_registered() {
        let (mock, _rx) = MockMqttClient::new();
        let mut client = MockMqttHandle::new(Arc::clone(&mock));
        client.connect(&session()).unwrap();
        let will = mock.last_will().expect("will registered at connect");
        assert_eq!(will.topic, "dali/ctl1/availability");
        assert_eq!(will.payload, b"offline");
        assert!(will.retain);
    }

    #[test]
    fn publishing_without_a_session_fails_rather_than_being_recorded() {
        let (mock, _rx) = MockMqttClient::new();
        let mut client = MockMqttHandle::new(Arc::clone(&mock));
        assert_eq!(
            client.publish("t", b"x", MqttQos::AtMostOnce, false),
            Err(MqttError::NotConnected)
        );
        assert!(mock.published().is_empty());
    }

    #[test]
    fn a_delivered_message_arrives_through_the_production_link() {
        let (mock, rx) = MockMqttClient::new();
        mock.deliver("dali/ctl1/a0/vl/12/set", br#"{"state":"ON"}"#);
        let got = rx.try_recv().expect("message queued");
        assert_eq!(got.topic, "dali/ctl1/a0/vl/12/set");
        assert_eq!(got.payload, br#"{"state":"ON"}"#);
    }
}
