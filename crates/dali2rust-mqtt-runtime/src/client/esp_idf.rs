use std::sync::Arc;

use esp_idf_svc::mqtt::client::{
    EspMqttClient, EventPayload, LwtConfiguration, MqttClientConfiguration, QoS,
};
use esp_idf_svc::sys::EspError;

use dali2rust_platform::mqtt::{
    MqttClient, MqttClientBundle, MqttConnectionState, MqttError, MqttIncoming, MqttLink, MqttQos,
    MqttSessionConfig,
};

const BUFFER_BYTES: usize = 1024;
const OUT_BUFFER_BYTES: usize = 2048;
const TASK_STACK_BYTES: usize = 4096;

fn to_qos(qos: MqttQos) -> QoS {
    match qos {
        MqttQos::AtMostOnce => QoS::AtMostOnce,
        MqttQos::AtLeastOnce => QoS::AtLeastOnce,
    }
}

pub struct EspMqttBridgeClient {
    client: Option<EspMqttClient<'static>>,
    link: Arc<MqttLink>,
}

impl EspMqttBridgeClient {
    pub fn bundle() -> MqttClientBundle {
        let (link, incoming) = MqttLink::new();
        MqttClientBundle {
            client: Box::new(Self { client: None, link }),
            incoming,
        }
    }

    fn open(&mut self, config: &MqttSessionConfig) -> Result<EspMqttClient<'static>, EspError> {
        let url = format!("mqtt://{}:{}", config.broker_host, config.broker_port);
        let lwt = config.last_will.as_ref().map(|w| LwtConfiguration {
            topic: &w.topic,
            payload: &w.payload,
            qos: to_qos(w.qos),
            retain: w.retain,
        });
        let mut cfg = MqttClientConfiguration {
            client_id: Some(&config.client_id),
            keep_alive_interval: Some(config.keep_alive),
            buffer_size: BUFFER_BYTES,
            out_buffer_size: OUT_BUFFER_BYTES,
            task_stack: TASK_STACK_BYTES,
            lwt,
            ..Default::default()
        };
        if !config.username.is_empty() {
            cfg.username = Some(&config.username);
            cfg.password = Some(&config.password);
        }
        let link = Arc::clone(&self.link);
        EspMqttClient::new_cb(&url, &cfg, move |event| dispatch(&link, event.payload()))
    }
}

fn dispatch(link: &Arc<MqttLink>, payload: EventPayload<'_, EspError>) {
    dali2rust_bsp::task_registry::observe_current(c"mqtt_task");
    match payload {
        EventPayload::Connected(_) => link.set_state(MqttConnectionState::Connected),
        EventPayload::Disconnected => link.set_state(MqttConnectionState::Disconnected),
        EventPayload::Subscribed(_) => link.note_subscription_acked(),
        EventPayload::Received { topic, data, .. } => {
            if let Some(topic) = topic {
                link.deliver(MqttIncoming {
                    topic: topic.to_string(),
                    payload: data.to_vec(),
                });
            }
        }
        EventPayload::Error(_) => {
            log::warn!("mqtt: transport error (cause not surfaced by esp-idf-svc)");
        }
        _ => {}
    }
}

impl MqttClient for EspMqttBridgeClient {
    fn connect(&mut self, config: &MqttSessionConfig) -> Result<(), MqttError> {
        self.link.set_state(MqttConnectionState::Connecting);
        match self.open(config) {
            Ok(client) => {
                self.client = Some(client);
                Ok(())
            }
            Err(e) => {
                self.link.set_state(MqttConnectionState::Disconnected);
                Err(MqttError::Rejected(e.code()))
            }
        }
    }

    fn disconnect(&mut self) {
        self.client = None;
        self.link.set_state(MqttConnectionState::Disconnected);
    }

    fn publish(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: MqttQos,
        retain: bool,
    ) -> Result<(), MqttError> {
        if payload.len() > OUT_BUFFER_BYTES {
            return Err(MqttError::PayloadTooLarge);
        }
        let client = self.client.as_mut().ok_or(MqttError::NotConnected)?;
        client
            .publish(topic, to_qos(qos), retain, payload)
            .map(|_| ())
            .map_err(|e| MqttError::Rejected(e.code()))
    }

    fn subscribe(&mut self, topic_filter: &str, qos: MqttQos) -> Result<(), MqttError> {
        let client = self.client.as_mut().ok_or(MqttError::NotConnected)?;
        client
            .subscribe(topic_filter, to_qos(qos))
            .map(|_| ())
            .map_err(|e| MqttError::Rejected(e.code()))
    }

    fn link(&self) -> Arc<MqttLink> {
        Arc::clone(&self.link)
    }
}
