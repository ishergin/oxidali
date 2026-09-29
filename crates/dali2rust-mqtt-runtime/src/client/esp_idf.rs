use core::ffi::{c_char, c_int, c_void};
use std::sync::Arc;

use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::mqtt::client::{
    EspMqttClient, EventPayload, LwtConfiguration, MqttClientConfiguration, QoS,
};
use esp_idf_svc::sys::{
    esp, esp_event_base_t, esp_mqtt_client_register_event, esp_mqtt_event_id_t_MQTT_EVENT_DATA,
    esp_mqtt_event_t, EspError,
};

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
        let client = EspMqttClient::new_cb(&url, &cfg, move |event| dispatch(&link, event.payload()))?;
        receive_data_events(&client, &self.link)?;
        Ok(client)
    }
}

fn dispatch(link: &Arc<MqttLink>, payload: EventPayload<'_, EspError>) {
    dali2rust_bsp::task_registry::observe_current(c"mqtt_task");
    match payload {
        EventPayload::Connected(_) => link.set_state(MqttConnectionState::Connected),
        EventPayload::Disconnected => link.set_state(MqttConnectionState::Disconnected),
        EventPayload::Subscribed(_) => link.note_subscription_acked(),
        EventPayload::Error(_) => {
            log::warn!("mqtt: transport error (cause not surfaced by esp-idf-svc)");
        }
        _ => {}
    }
}

fn receive_data_events(client: &EspMqttClient<'static>, link: &Arc<MqttLink>) -> Result<(), EspError> {
    let receiver = Arc::as_ptr(link).cast_mut().cast::<c_void>();
    // SAFETY: the link outlives `client`, which drops first and unregisters the handler as it is destroyed.
    esp!(unsafe {
        esp_mqtt_client_register_event(
            client.handle(),
            esp_mqtt_event_id_t_MQTT_EVENT_DATA,
            Some(on_data),
            receiver,
        )
    })
}

extern "C" fn on_data(receiver: *mut c_void, _base: esp_event_base_t, _id: i32, event: *mut c_void) {
    // SAFETY: registered with the bridge's `MqttLink` for MQTT_EVENT_DATA, whose data is an `esp_mqtt_event_t`.
    let (link, event) = unsafe { (&*receiver.cast::<MqttLink>(), &*event.cast::<esp_mqtt_event_t>()) };
    if event.current_data_offset != 0 || event.topic.is_null() {
        return;
    }
    // SAFETY: esp-mqtt points both at their stated byte counts of the PUBLISH until this handler returns.
    let (topic, payload) = unsafe {
        (event_bytes(event.topic, event.topic_len), event_bytes(event.data, event.data_len))
    };
    let Ok(topic) = core::str::from_utf8(topic) else {
        return;
    };
    link.deliver(MqttIncoming {
        topic: topic.to_string(),
        payload: payload.to_vec(),
        retained: event.retain,
    });
}

unsafe fn event_bytes<'a>(start: *const c_char, len: c_int) -> &'a [u8] {
    match usize::try_from(len) {
        Ok(len) if len > 0 && !start.is_null() => {
            // SAFETY: the caller vouches for `len` readable bytes at `start` for the lifetime it picks.
            unsafe { core::slice::from_raw_parts(start.cast::<u8>(), len) }
        }
        _ => &[],
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

    fn unsubscribe(&mut self, topic_filter: &str) -> Result<(), MqttError> {
        let client = self.client.as_mut().ok_or(MqttError::NotConnected)?;
        client
            .unsubscribe(topic_filter)
            .map(|_| ())
            .map_err(|e| MqttError::Rejected(e.code()))
    }

    fn link(&self) -> Arc<MqttLink> {
        Arc::clone(&self.link)
    }
}
