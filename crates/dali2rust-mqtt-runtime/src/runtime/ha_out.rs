use dali2rust_api::ha::{HaDevice, HaTopics};
use dali2rust_bus::worker_counters::bump;
use dali2rust_domain::registry::HomeAssistantSettingsView;
use dali2rust_platform::mqtt::{MqttClient, MqttQos};

use crate::runtime::session::{qos_of, Session};
use crate::runtime::worker::MqttWorkerPorts;

pub(super) struct HaOut<'a> {
    pub(super) client: &'a mut dyn MqttClient,
    pub(super) session: &'a mut Session,
    pub(super) topics: &'a HaTopics,
    pub(super) settings: &'a HomeAssistantSettingsView,
    pub(super) ports: &'a MqttWorkerPorts,
}

impl<'a> HaOut<'a> {
    pub(super) fn publish(&mut self, topic: &str, payload: &str, retain: bool) -> bool {
        self.send(topic, payload, qos_of(self.settings), retain)
    }

    pub(super) fn send(&mut self, topic: &str, payload: &str, qos: MqttQos, retain: bool) -> bool {
        match self.client.publish(topic, payload.as_bytes(), qos, retain) {
            Ok(()) => {
                bump(&self.ports.counters.publishes_total);
                true
            }
            Err(e) => {
                log::warn!("mqtt: publish to {topic} refused: {e:?}");
                bump(&self.ports.counters.publish_failures_total);
                false
            }
        }
    }

    pub(super) fn device(&self) -> HaDevice<'a> {
        HaDevice {
            controller_id: &self.settings.controller_id,
            version: self.ports.version,
        }
    }
}
