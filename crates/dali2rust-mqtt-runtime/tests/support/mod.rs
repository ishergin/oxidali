use std::sync::{Arc, Mutex};

use dali2rust_bus::{
    BusConfig, BusFrame, BusHost, BusId, BusPublisher, CorrelationIdAllocator,
};
use dali2rust_contracts::msg::{
    BusEventPayload, LightSetpoint, Origin, RuntimeObservation,
};
use dali2rust_domain::registry::{
    CapabilityFlagsView, HaGroupView, HaInputInstanceView, HaInputView, HaLampView,
    HaPublishReadPort, HaSceneView, HomeAssistantSecretReadPort,
    HomeAssistantSettingsReadPort, HomeAssistantSettingsView,
};
use dali2rust_mqtt_runtime::{
    spawn_mqtt_worker, MockMqttClient, MqttCounters, MqttWorkerPorts,
    MQTT_WORKER_HANDLED_COMMANDS, MQTT_WORKER_HANDLED_EVENTS,
};

pub struct StubSettings(Mutex<HomeAssistantSettingsView>);

impl StubSettings {
    pub fn new(view: HomeAssistantSettingsView) -> Arc<Self> {
        Arc::new(Self(Mutex::new(view)))
    }

    #[allow(dead_code, reason = "one binary per test file; not every file flips settings")]
    pub fn set(&self, view: HomeAssistantSettingsView) {
        *self.0.lock().unwrap() = view;
    }
}

impl HomeAssistantSettingsReadPort for StubSettings {
    fn home_assistant_settings_view(&self) -> HomeAssistantSettingsView {
        self.0.lock().unwrap().clone()
    }
}

struct StubSettingsWatch(std::sync::atomic::AtomicU32);

impl dali2rust_domain::registry::HomeAssistantSettingsApplyWatchPort for StubSettingsWatch {
    fn home_assistant_settings_applied_load(&self) -> u32 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

struct StubSecret;

impl HomeAssistantSecretReadPort for StubSecret {
    fn home_assistant_broker_password(&self) -> String {
        String::new()
    }
}

pub struct StubHaReadPort {
    lamps: Mutex<Vec<HaLampView>>,
    groups: Mutex<Vec<HaGroupView>>,
    scenes: Mutex<Vec<HaSceneView>>,
    binding_short: Mutex<Option<u8>>,
    inputs: Mutex<Vec<HaInputView>>,
}

impl StubHaReadPort {
    pub fn with_lamp(virtual_lamp_id: u8) -> Arc<Self> {
        Arc::new(Self {
            lamps: Mutex::new(vec![HaLampView {
                virtual_lamp_id,
                name: format!("Lamp {virtual_lamp_id}"),
                ha_entity_enabled: true,
                capabilities: CapabilityFlagsView {
                    brightness: true,
                    ..Default::default()
                },
                color_temperature_range: None,
                unreachable: false,
            }]),
            groups: Mutex::new(Vec::new()),
            scenes: Mutex::new(Vec::new()),
            binding_short: Mutex::new(None),
            inputs: Mutex::new(Vec::new()),
        })
    }

    #[allow(dead_code, reason = "one binary per test file")]
    pub fn bind_to_short(&self, short_address: u8) {
        *self.binding_short.lock().unwrap() = Some(short_address);
    }

    #[allow(dead_code, reason = "one binary per test file; not every file uses inputs")]
    pub fn set_button_device(&self, short_address: u8, instances: u8) {
        let mut inputs = self.inputs.lock().unwrap();
        inputs.retain(|v| v.short_address != short_address);
        inputs.push(HaInputView {
            short_address,
            name: Some(format!("Panel {short_address}")),
            ha_expose: true,
            instances: (0..instances)
                .map(|instance_number| HaInputInstanceView {
                    instance_number,
                    instance_type: Some(1),
                })
                .collect(),
        });
    }

    #[allow(dead_code, reason = "one binary per test file; not every file uses groups")]
    pub fn set_group(&self, group: HaGroupView) {
        let mut groups = self.groups.lock().unwrap();
        groups.retain(|g| g.group_id != group.group_id);
        groups.push(group);
    }

    #[allow(dead_code, reason = "one binary per test file; not every file uses scenes")]
    pub fn set_scenes(&self, scenes: Vec<HaSceneView>) {
        *self.scenes.lock().unwrap() = scenes;
    }
}

impl HaPublishReadPort for StubHaReadPort {
    fn ha_input_views(&self, adapter_id: u8) -> Vec<HaInputView> {
        if adapter_id == 0 {
            self.inputs.lock().unwrap().clone()
        } else {
            Vec::new()
        }
    }

    fn ha_input_view(&self, adapter_id: u8, short_address: u8) -> Option<HaInputView> {
        self.ha_input_views(adapter_id)
            .into_iter()
            .find(|v| v.short_address == short_address)
    }

    fn ha_lamp_views(&self, adapter_id: u8) -> Vec<HaLampView> {
        if adapter_id == 0 {
            self.lamps.lock().unwrap().clone()
        } else {
            Vec::new()
        }
    }

    fn ha_group_views(&self, adapter_id: u8) -> Vec<HaGroupView> {
        if adapter_id == 0 {
            self.groups.lock().unwrap().clone()
        } else {
            Vec::new()
        }
    }

    fn ha_group_view(&self, adapter_id: u8, group_id: u8) -> Option<HaGroupView> {
        self.ha_group_views(adapter_id)
            .into_iter()
            .find(|g| g.group_id == group_id)
    }

    fn ha_scene_views(&self, adapter_id: u8) -> Vec<HaSceneView> {
        if adapter_id == 0 {
            self.scenes.lock().unwrap().clone()
        } else {
            Vec::new()
        }
    }

    fn ha_lamp_view(&self, adapter_id: u8, virtual_lamp_id: u8) -> Option<HaLampView> {
        self.ha_lamp_views(adapter_id)
            .into_iter()
            .find(|l| l.virtual_lamp_id == virtual_lamp_id)
    }

    fn ha_lamp_ids_for_short(&self, adapter_id: u8, short_address: u8) -> Vec<u8> {
        if *self.binding_short.lock().unwrap() != Some(short_address) {
            return Vec::new();
        }
        self.ha_lamp_views(adapter_id)
            .into_iter()
            .map(|l| l.virtual_lamp_id)
            .collect()
    }

    fn ha_retracted_lamp_ids(&self, adapter_id: u8) -> Vec<u8> {
        let exposed: Vec<u8> = self
            .ha_lamp_views(adapter_id)
            .into_iter()
            .map(|l| l.virtual_lamp_id)
            .collect();
        (0..64).filter(|id| !exposed.contains(id)).collect()
    }

    fn ha_retracted_group_ids(&self, adapter_id: u8) -> Vec<u8> {
        let exposed: Vec<u8> = self
            .ha_group_views(adapter_id)
            .into_iter()
            .map(|g| g.group_id)
            .collect();
        (0..16).filter(|id| !exposed.contains(id)).collect()
    }

    fn ha_active_scene(&self, _adapter_id: u8) -> Option<u8> {
        None
    }
}

pub fn enabled_settings() -> HomeAssistantSettingsView {
    HomeAssistantSettingsView {
        enabled: true,
        broker_host: "broker.test".to_string(),
        broker_port: 1883,
        discovery_prefix: "homeassistant".to_string(),
        state_topic_prefix: "dali".to_string(),
        controller_id: "ctl1".to_string(),
        publish_qos: 1,
        retain_state: true,
        retain_discovery: true,
        ..Default::default()
    }
}

pub struct Harness {
    pub mock: Arc<MockMqttClient>,
    pub publisher: BusPublisher,
    pub counters: Arc<MqttCounters>,
    #[allow(dead_code, reason = "holds the settings port so a test can flip it live")]
    pub settings: Arc<StubSettings>,
    #[allow(dead_code, reason = "not every test file asserts on published commands")]
    pub cmd_rx: std::sync::mpsc::Receiver<BusFrame>,
    #[allow(dead_code, reason = "not every test file asserts on published events")]
    pub ev_obs_rx: std::sync::mpsc::Receiver<BusFrame>,
    _host: BusHost,
    _worker: std::thread::JoinHandle<()>,
}

pub fn spawn_bridge(settings: Arc<StubSettings>, read_port: Arc<StubHaReadPort>) -> Harness {
    spawn_bridge_with_config(settings, read_port, BusConfig::default())
}

#[allow(dead_code, reason = "one binary per test file; not every file shapes the bus")]
pub fn spawn_bridge_with_config(
    settings: Arc<StubSettings>,
    read_port: Arc<StubHaReadPort>,
    config: BusConfig,
) -> Harness {
    let (host, publisher, (ev_rx, cmd_rx, ev_obs_rx)) = BusHost::spawn(config, |reg| {
        (
            reg.subscribe_commands_and_events(
                64,
                MQTT_WORKER_HANDLED_COMMANDS,
                MQTT_WORKER_HANDLED_EVENTS,
            ),
            reg.subscribe_commands(64, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
            reg.subscribe_events(64, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
        )
    });
    let (mock, bundle) = MockMqttClient::bundle();
    let link = bundle.client.link();
    let counters = Arc::new(MqttCounters::default());
    let worker = spawn_mqtt_worker(
        ev_rx,
        bundle.client,
        bundle.incoming,
        link,
        MqttWorkerPorts {
            publisher: publisher.clone(),
            bus_id: BusId::default(),
            correlation: Arc::new(CorrelationIdAllocator::new()),
            read_port,
            settings: Arc::clone(&settings) as Arc<dyn HaSettingsPortAlias>,
            settings_watch: Arc::new(StubSettingsWatch(std::sync::atomic::AtomicU32::new(0))),
            secret: Arc::new(StubSecret),
            counters: Arc::clone(&counters),
            version: "test",
            adapter_count: 1,
            role: Arc::new(ActiveRole),
        },
    );
    Harness {
        mock,
        publisher,
        counters,
        settings,
        cmd_rx,
        ev_obs_rx,
        _host: host,
        _worker: worker,
    }
}

#[allow(dead_code, reason = "one binary per test file; not every file feeds events")]
pub fn registry_event(payload: impl Into<BusEventPayload>) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        0,
        0,
        BusId::default().0,
        Some(Origin::Internal),
        payload,
    ))
}

use dali2rust_domain::registry::HomeAssistantSettingsReadPort as HaSettingsPortAlias;

#[allow(dead_code, reason = "one binary per test file; not every file feeds presses")]
pub fn button_press_frame(short_address: u8, instance_number: u8) -> BusFrame {
    registry_event(dali2rust_contracts::msg::DaliInputEventObservedEvent {
        registry_adapter_id: 0,
        scheme: 2,
        short_address: Some(short_address),
        device_group: None,
        instance_group: None,
        instance_number: Some(instance_number),
        instance_type: Some(1),
        event_info: 0x002,
        typed: dali2rust_contracts::msg::InputEventKind::Button,
        typed_value: 0x002,
        observed_at_ms: 1,
        observed_at_mono_ms: 1,
    })
}

#[allow(dead_code, reason = "one binary per test file; not every file feeds commits")]
pub fn runtime_frame(virtual_lamp_id: u8, level: u8) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        0,
        u64::from(level),
        BusId::default().0,
        Some(Origin::Internal),
        dali2rust_contracts::msg::RuntimeStateChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: Some(virtual_lamp_id),
            short_address: Some(virtual_lamp_id),
            state_setpoint: LightSetpoint::from_level(level, None),
            state_observation: RuntimeObservation::default(),
            commit_source: dali2rust_contracts::msg::RuntimeSource::Api,
            commit_dimensions: LightSetpoint::from_level(level, None).dimensions(),
        },
    ))
}

#[allow(dead_code, reason = "one binary per test file")]
pub fn absence_frame(short_address: u8) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        0,
        900,
        BusId::default().0,
        Some(Origin::Internal),
        dali2rust_contracts::msg::RuntimeStateChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: None,
            short_address: Some(short_address),
            commit_dimensions: LightSetpoint::default().dimensions(),
            state_setpoint: LightSetpoint::default(),
            state_observation: RuntimeObservation::device_absent(
                dali2rust_contracts::msg::RuntimeSource::Poller,
            ),
            commit_source: dali2rust_contracts::msg::RuntimeSource::Poller,
        },
    ))
}

pub struct ActiveRole;

impl dali2rust_domain::registry::DaliSettingsReadPort for ActiveRole {
    fn dali_settings_view(&self) -> dali2rust_domain::registry::DaliSettingsView {
        dali2rust_domain::registry::DaliSettingsView {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: true,
            device_short_address: None,
        }
    }
}
