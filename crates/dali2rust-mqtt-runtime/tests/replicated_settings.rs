use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, CorrelationIdAllocator};
use dali2rust_contracts::msg::{fixed_text_32, RegistrySliceReloadCommand};
use dali2rust_domain::registry::HomeAssistantSettingsReadPort;
use dali2rust_mqtt_runtime::{
    spawn_mqtt_worker, MockMqttClient, MqttCounters, MqttWorkerPorts,
    MQTT_WORKER_HANDLED_COMMANDS, MQTT_WORKER_HANDLED_EVENTS,
};
use dali2rust_platform::liveness::LivenessBeat;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_registry_runtime::{
    encode_persistence_blob, spawn_registry_worker, PersistableHomeAssistantSettingsSlice,
    PersistenceEnvelope, RegistryApplyWatch, RegistryStore, RegistryWorkerCounters,
    HOME_ASSISTANT_SETTINGS_SLICE_VERSION, REGISTRY_EVENTS_HANDLED_EVENTS,
    REGISTRY_WORKER_HANDLED_COMMANDS,
};
use dali2rust_test_support::{publish_queued, temp_slice_store, wait_until, write_slice};

const ADAPTERS: u8 = 1;
const WAIT: Duration = Duration::from_secs(4);
const LIVENESS_BUDGET_MS: u32 = 60_000;
const REPLICATED_BROKER: &str = "broker.replicated";

struct Stack {
    mock: Arc<MockMqttClient>,
    publisher: BusPublisher,
    slices: Arc<dyn SliceStore>,
    store: Arc<RegistryStore>,
    _host: BusHost,
}

fn spawn_stack() -> Stack {
    let (host, publisher, (registry_rx, bridge_rx)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands_and_events(
                64,
                REGISTRY_WORKER_HANDLED_COMMANDS,
                REGISTRY_EVENTS_HANDLED_EVENTS,
            ),
            reg.subscribe_commands_and_events(
                64,
                MQTT_WORKER_HANDLED_COMMANDS,
                MQTT_WORKER_HANDLED_EVENTS,
            ),
        )
    });
    let slices: Arc<dyn SliceStore> = Arc::new(temp_slice_store("ha-replicated"));
    let store = Arc::new(RegistryStore::with_adapter_count(ADAPTERS));
    let counters = Arc::new(RegistryWorkerCounters::default());
    spawn_registry_worker(
        registry_rx,
        publisher.clone(),
        BusId::default(),
        ADAPTERS,
        Arc::clone(&store),
        Arc::clone(&counters),
        Some(Arc::clone(&slices)),
        Arc::new(LivenessBeat::new("test", LIVENESS_BUDGET_MS)),
    );
    let (mock, bundle) = MockMqttClient::bundle();
    let link = bundle.client.link();
    spawn_mqtt_worker(
        bridge_rx,
        bundle.client,
        bundle.incoming,
        link,
        MqttWorkerPorts {
            publisher: publisher.clone(),
            bus_id: BusId::default(),
            correlation: Arc::new(CorrelationIdAllocator::new()),
            read_port: Arc::clone(&store) as _,
            settings: Arc::clone(&store) as _,
            settings_watch: Arc::new(RegistryApplyWatch::new(counters)),
            secret: Arc::clone(&store) as _,
            counters: Arc::new(MqttCounters::default()),
            version: "test",
            adapter_count: ADAPTERS,
            role: Arc::clone(&store) as _,
        },
    );
    Stack { mock, publisher, slices, store, _host: host }
}

fn replicated_settings() -> PersistableHomeAssistantSettingsSlice {
    PersistableHomeAssistantSettingsSlice {
        enabled: true,
        broker_host: REPLICATED_BROKER.to_string(),
        broker_port: 1883,
        broker_username: String::new(),
        broker_password: String::new(),
        discovery_prefix: "homeassistant".to_string(),
        state_topic_prefix: "dali".to_string(),
        controller_id: "ctl1".to_string(),
        publish_qos: 1,
        retain_state: true,
        retain_discovery: true,
        expose_input_devices: false,
    }
}

fn replicate(stack: &Stack) {
    let envelope = PersistenceEnvelope::new(HOME_ASSISTANT_SETTINGS_SLICE_VERSION, replicated_settings());
    let bytes = encode_persistence_blob(&envelope).expect("encode home assistant slice");
    write_slice(stack.slices.as_ref(), SliceKey::HomeAssistantSettings, &bytes);
    let reload = dali2rust_contracts::bus::command_envelope(
        0,
        1,
        BusId::default().0,
        None,
        RegistrySliceReloadCommand { slice_name: fixed_text_32("home_assistant_settings") },
    );
    publish_queued(&stack.publisher, BusChannel::Commands, BusFrame::command(reload));
}

#[test]
fn replicated_settings_reach_the_bridge_on_the_slice_reload_issue163() {
    let stack = spawn_stack();
    replicate(&stack);
    wait_until(
        || stack.store.home_assistant_settings_view().broker_host == REPLICATED_BROKER,
        WAIT,
    );
    wait_until(
        || {
            stack
                .mock
                .session_config()
                .is_some_and(|config| config.broker_host == REPLICATED_BROKER)
        },
        WAIT,
    );
}
