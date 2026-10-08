use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::sync::Arc;

use esp_idf_svc::sys::{
    esp, esp_event_base_t, esp_mqtt_client_config_t, esp_mqtt_client_destroy,
    esp_mqtt_client_handle_t, esp_mqtt_client_init, esp_mqtt_client_publish,
    esp_mqtt_client_register_event, esp_mqtt_client_start, esp_mqtt_client_subscribe_single,
    esp_mqtt_client_unsubscribe, esp_mqtt_event_id_t_MQTT_EVENT_ANY,
    esp_mqtt_event_id_t_MQTT_EVENT_CONNECTED, esp_mqtt_event_id_t_MQTT_EVENT_DATA,
    esp_mqtt_event_id_t_MQTT_EVENT_DISCONNECTED, esp_mqtt_event_id_t_MQTT_EVENT_ERROR,
    esp_mqtt_event_id_t_MQTT_EVENT_SUBSCRIBED, esp_mqtt_event_t, EspError, ESP_FAIL,
};

use dali2rust_platform::mqtt::{
    MqttClient, MqttClientBundle, MqttConnectionState, MqttError, MqttLink, MqttQos,
    MqttSessionConfig, MqttSubAck,
};

const BUFFER_BYTES: c_int = 1024;
const OUT_BUFFER_BYTES: usize = 2048;
const TASK_STACK_BYTES: c_int = 4096;
const SUBACK_FAILURE: u8 = 0x80;
const QOS_AT_MOST_ONCE: c_int = 0;
const QOS_AT_LEAST_ONCE: c_int = 1;

fn qos_level(qos: MqttQos) -> c_int {
    match qos {
        MqttQos::AtMostOnce => QOS_AT_MOST_ONCE,
        MqttQos::AtLeastOnce => QOS_AT_LEAST_ONCE,
    }
}

fn rejected(e: EspError) -> MqttError {
    MqttError::Rejected(e.code())
}

fn accepted(rc: c_int) -> Result<u32, MqttError> {
    u32::try_from(rc).map_err(|_| MqttError::Rejected(rc))
}

fn c_string(text: &str) -> Result<CString, MqttError> {
    CString::new(text).map_err(|_| MqttError::ConfigInvalid)
}

fn optional_ptr(text: Option<&CString>) -> *const c_char {
    text.map_or(core::ptr::null(), |text| text.as_ptr())
}

struct RawClient(esp_mqtt_client_handle_t);

// SAFETY: esp-mqtt serialises every call on a client behind the client's own API lock, so the handle may move to the bridge worker's thread.
unsafe impl Send for RawClient {}

impl Drop for RawClient {
    fn drop(&mut self) {
        // SAFETY: the handle came from `esp_mqtt_client_init` and is destroyed once; destroy stops the task and deletes the loop that holds the handler.
        unsafe { esp_mqtt_client_destroy(self.0) };
    }
}

struct SessionStrings {
    uri: CString,
    client_id: CString,
    username: Option<CString>,
    password: Option<CString>,
    will_topic: Option<CString>,
}

impl SessionStrings {
    fn new(config: &MqttSessionConfig) -> Result<Self, MqttError> {
        let credential = |text: &str| (!config.username.is_empty()).then(|| c_string(text)).transpose();
        let will_topic = config.last_will.as_ref().map(|will| c_string(&will.topic));
        Ok(Self {
            uri: c_string(&format!("mqtt://{}:{}", config.broker_host, config.broker_port))?,
            client_id: c_string(&config.client_id)?,
            username: credential(&config.username)?,
            password: credential(&config.password)?,
            will_topic: will_topic.transpose()?,
        })
    }

    fn raw_config(&self, config: &MqttSessionConfig) -> Box<esp_mqtt_client_config_t> {
        let mut raw = Box::<esp_mqtt_client_config_t>::default();
        raw.broker.address.uri = self.uri.as_ptr();
        raw.credentials.client_id = self.client_id.as_ptr();
        raw.credentials.username = optional_ptr(self.username.as_ref());
        raw.credentials.authentication.password = optional_ptr(self.password.as_ref());
        raw.session.keepalive = c_int::try_from(config.keep_alive.as_secs()).unwrap_or(c_int::MAX);
        raw.network.disable_auto_reconnect = true;
        raw.task.stack_size = TASK_STACK_BYTES;
        raw.buffer.size = BUFFER_BYTES;
        raw.buffer.out_size = c_int::try_from(OUT_BUFFER_BYTES).unwrap_or(c_int::MAX);
        if let (Some(will), Some(topic)) = (config.last_will.as_ref(), self.will_topic.as_ref()) {
            raw.session.last_will.topic = topic.as_ptr();
            raw.session.last_will.msg = will.payload.as_ptr().cast::<c_char>();
            raw.session.last_will.msg_len = c_int::try_from(will.payload.len()).unwrap_or(c_int::MAX);
            raw.session.last_will.qos = qos_level(will.qos);
            raw.session.last_will.retain = c_int::from(will.retain);
        }
        raw
    }
}

pub struct EspMqttBridgeClient {
    client: Option<RawClient>,
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

    fn open(&self, config: &MqttSessionConfig) -> Result<RawClient, MqttError> {
        let strings = SessionStrings::new(config)?;
        let raw_config = strings.raw_config(config);
        // SAFETY: init copies every string and the will message out of the config before it returns.
        let handle = unsafe { esp_mqtt_client_init(&*raw_config) };
        if handle.is_null() {
            return Err(MqttError::Rejected(ESP_FAIL));
        }
        let client = RawClient(handle);
        let receiver = Arc::as_ptr(&self.link).cast_mut().cast::<c_void>();
        // SAFETY: the link outlives the client, whose destroy removes the handler; registering before start leaves no event unseen.
        esp!(unsafe {
            esp_mqtt_client_register_event(handle, esp_mqtt_event_id_t_MQTT_EVENT_ANY, Some(on_event), receiver)
        })
        .map_err(rejected)?;
        // SAFETY: an initialised client whose handler is registered.
        esp!(unsafe { esp_mqtt_client_start(handle) }).map_err(rejected)?;
        Ok(client)
    }

    fn handle(&self) -> Result<esp_mqtt_client_handle_t, MqttError> {
        self.client.as_ref().map(|client| client.0).ok_or(MqttError::NotConnected)
    }
}

extern "C" fn on_event(receiver: *mut c_void, _base: esp_event_base_t, _id: i32, event: *mut c_void) {
    dali2rust_bsp::task_registry::observe_current(c"mqtt_task");
    // SAFETY: registered with the bridge's `MqttLink`; every esp-mqtt event carries an `esp_mqtt_event_t`.
    let (link, event) = unsafe { (&*receiver.cast::<MqttLink>(), &*event.cast::<esp_mqtt_event_t>()) };
    let id = event.event_id;
    if id == esp_mqtt_event_id_t_MQTT_EVENT_DATA {
        deliver_first_chunk(link, event);
    } else if id == esp_mqtt_event_id_t_MQTT_EVENT_SUBSCRIBED {
        note_suback(link, event);
    } else if id == esp_mqtt_event_id_t_MQTT_EVENT_CONNECTED {
        link.set_state(MqttConnectionState::Connected);
    } else if id == esp_mqtt_event_id_t_MQTT_EVENT_DISCONNECTED {
        link.set_state(MqttConnectionState::Disconnected);
    } else if id == esp_mqtt_event_id_t_MQTT_EVENT_ERROR {
        log_error(event);
    }
}

fn log_error(event: &esp_mqtt_event_t) {
    // SAFETY: esp-mqtt points `error_handle` at the client's error record until the handler returns.
    let Some(codes) = (unsafe { event.error_handle.as_ref() }) else {
        log::warn!("mqtt: error event without a cause");
        return;
    };
    log::warn!(
        "mqtt: error type {} (connect return {}, tls {}, socket errno {})",
        codes.error_type,
        codes.connect_return_code,
        codes.esp_tls_last_esp_err,
        codes.esp_transport_sock_errno
    );
}

fn deliver_first_chunk(link: &MqttLink, event: &esp_mqtt_event_t) {
    if event.current_data_offset != 0 || event.topic.is_null() {
        return;
    }
    // SAFETY: esp-mqtt points both at their stated byte counts of the PUBLISH until the handler returns.
    let (topic, payload) = unsafe {
        (event_bytes(event.topic, event.topic_len), event_bytes(event.data, event.data_len))
    };
    link.deliver(String::from_utf8_lossy(topic).into_owned(), payload.to_vec(), event.retain);
}

fn note_suback(link: &MqttLink, event: &esp_mqtt_event_t) {
    let Ok(message_id) = u32::try_from(event.msg_id) else {
        return;
    };
    // SAFETY: esp-mqtt points `data` at the SUBACK's return codes until the handler returns.
    let codes = unsafe { event_bytes(event.data, event.data_len) };
    let granted = !codes.is_empty() && codes.iter().all(|code| *code < SUBACK_FAILURE);
    link.note_suback(MqttSubAck { message_id, granted });
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
        self.client = None;
        self.link.set_state(MqttConnectionState::Connecting);
        match self.open(config) {
            Ok(client) => {
                self.client = Some(client);
                Ok(())
            }
            Err(e) => {
                self.link.set_state(MqttConnectionState::Disconnected);
                Err(e)
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
        let handle = self.handle()?;
        let topic = c_string(topic)?;
        let data = if payload.is_empty() { core::ptr::null() } else { payload.as_ptr().cast::<c_char>() };
        let len = c_int::try_from(payload.len()).map_err(|_| MqttError::PayloadTooLarge)?;
        // SAFETY: topic and payload outlive the call, which copies them into the outbox or onto the socket.
        let rc = unsafe {
            esp_mqtt_client_publish(handle, topic.as_ptr(), data, len, qos_level(qos), c_int::from(retain))
        };
        accepted(rc).map(drop)
    }

    fn subscribe(&mut self, topic_filter: &str, qos: MqttQos) -> Result<u32, MqttError> {
        let handle = self.handle()?;
        let filter = c_string(topic_filter)?;
        // SAFETY: the filter outlives the call, which copies it into the SUBSCRIBE packet.
        accepted(unsafe { esp_mqtt_client_subscribe_single(handle, filter.as_ptr(), qos_level(qos)) })
    }

    fn unsubscribe(&mut self, topic_filter: &str) -> Result<(), MqttError> {
        let handle = self.handle()?;
        let filter = c_string(topic_filter)?;
        // SAFETY: the filter outlives the call, which copies it into the UNSUBSCRIBE packet.
        accepted(unsafe { esp_mqtt_client_unsubscribe(handle, filter.as_ptr()) }).map(drop)
    }

    fn link(&self) -> Arc<MqttLink> {
        Arc::clone(&self.link)
    }
}
