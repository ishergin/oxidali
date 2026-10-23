mod support;

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusFrame, BusId, PublishResult};
use dali2rust_contracts::msg::{
    GroupChangedEvent, GroupMatrixChangedEvent, HomeAssistantDiscoveryPublishCommand,
    InputDeviceChangedEvent, Origin, PhysicalDeviceChangedEvent, SceneChangedEvent,
    VirtualLampChangedEvent,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{
    CapabilityFlagsView, ColorTemperatureRangeView, HaGroupView, HaInputInstanceView, HaInputView,
    HaLampView, HaSceneView, HomeAssistantSettingsView,
};
use dali2rust_mqtt_runtime::{MqttCounters, PublishedMessage};
use dali2rust_platform::mqtt::MqttQos;
use dali2rust_test_support::remains_false_for;
use support::{
    absence_frame, button_press_frame, enabled_settings, registry_event, runtime_frame,
    spawn_bridge, Harness, StubHaReadPort, StubSettings,
};

const WAIT: Duration = Duration::from_secs(8);
const LAMP_IDS: u8 = 5;
const GROUP_IDS: u8 = 10;
const BOUND_SHORT: u8 = 7;
const PANEL: u8 = 0;
const HIDDEN_PANEL: u8 = 1;
const DOORBELL: u8 = 2;
const MANUAL_RUN: u64 = 77;
const LAST_PLANNED: &str = "homeassistant/light/ctl1/a0_group_9/config";
const DOORBELL_STATE: &str = "dali/ctl1/a0/in/2/0/state";
const PANEL_STATE: &str = "dali/ctl1/a0/in/0/0/state";
const SELECT_STATE: &str = "dali/ctl1/a0/scene_select/state";
const HALL_STATE: &str = "dali/ctl1/a0/group/5/state";
const EMPTY_PAYLOAD: &str = "<empty>";
const RETAINED_GOLDEN: &str = include_str!("golden/ha_wire_retained.txt");
const UNRETAINED_GOLDEN: &str = include_str!("golden/ha_wire_unretained.txt");

#[test]
fn every_publish_keeps_its_topic_payload_qos_retain_and_order_when_everything_is_retained() {
    let settings = HomeAssistantSettingsView {
        expose_input_devices: true,
        ..enabled_settings()
    };
    assert_matches_golden("ha_wire_retained", &record_session(settings), RETAINED_GOLDEN);
}

#[test]
fn every_publish_keeps_its_topic_payload_qos_retain_and_order_when_nothing_is_retained() {
    let settings = HomeAssistantSettingsView {
        expose_input_devices: true,
        retain_discovery: false,
        retain_state: false,
        publish_qos: 0,
        ..enabled_settings()
    };
    assert_matches_golden("ha_wire_unretained", &record_session(settings), UNRETAINED_GOLDEN);
}

fn record_session(settings: HomeAssistantSettingsView) -> String {
    let port = seeded_registry();
    let bridge = spawn_bridge(StubSettings::new(settings), Arc::clone(&port));
    let mut log = WireLog { bridge, port, cursor: 0, text: String::new() };
    log.step("a fresh session announces everything", |_| {}, LAST_PLANNED);
    group_changes(&mut log);
    scene_changes(&mut log);
    lamp_changes(&mut log);
    refused_publishes(&mut log);
    input_changes(&mut log);
    log.step("a manual republish sends everything again", WireLog::manual_republish, LAST_PLANNED);
    log.step("a last press closes the log", |l| l.event(button_press_frame(DOORBELL, 0)), DOORBELL_STATE);
    log.finish()
}

fn group_changes(log: &mut WireLog) {
    log.step("a commit publishes the lamp, its groups and the select state", |l| {
        l.event(runtime_frame(1, 42));
    }, SELECT_STATE);
    log.fenced("a renamed group is announced again", |l| {
        l.port.set_group(group(5, "Hallway", brightness()));
        l.event(registry_event(GroupChangedEvent { adapter_id: 0, group_id: 5 }));
    });
    log.fenced("an unchanged group is not", |l| {
        l.event(registry_event(GroupChangedEvent { adapter_id: 0, group_id: 5 }));
    });
    log.fenced("a deleted group is retracted", |l| {
        l.port.remove_group(6);
        l.event(registry_event(GroupChangedEvent { adapter_id: 0, group_id: 6 }));
    });
    log.fenced("a matrix change retracts only the groups this session announced", |l| {
        l.port.remove_group(8);
        l.event(registry_event(GroupMatrixChangedEvent { adapter_id: 0 }));
    });
}

fn scene_changes(log: &mut WireLog) {
    log.fenced("an added scene announces the select again", |l| {
        l.port.set_scenes(vec![scene(3, "Evening"), scene(4, "Night")]);
        l.event(registry_event(SceneChangedEvent { adapter_id: 0, scene_id: 4 }));
    });
    log.fenced("the last scene removed retracts the select", |l| {
        l.port.set_scenes(Vec::new());
        l.event(registry_event(SceneChangedEvent { adapter_id: 0, scene_id: 3 }));
    });
    log.step("a commit with no scenes publishes no select", |l| {
        l.event(runtime_frame(1, 43));
    }, HALL_STATE);
}

fn lamp_changes(log: &mut WireLog) {
    log.fenced("a gear change announces its lamps again and marks the groups", |l| {
        l.port.set_lamp(lamp(1, "Lamp 1", tunable(), false));
        l.port.set_group(group(5, "Hallway", tunable()));
        l.event(registry_event(PhysicalDeviceChangedEvent { adapter_id: 0, short_address: BOUND_SHORT }));
    });
    log.step("the next commit announces the marked groups again", |l| {
        l.event(runtime_frame(1, 50));
    }, HALL_STATE);
    log.step("an absent gear takes its lamps offline", |l| l.event(absence_frame(BOUND_SHORT)), HALL_STATE);
    log.step("a commit from the gear brings its lamp back online", |l| {
        l.event(runtime_frame(1, 60));
    }, HALL_STATE);
    log.step("a commit for a lamp that neither exists nor was announced sends nothing for it", |l| {
        l.event(runtime_frame(3, 10));
    }, HALL_STATE);
    log.fenced("a removed lamp is retracted with its retained topics", |l| {
        l.port.remove_lamp(2);
        l.event(registry_event(VirtualLampChangedEvent { adapter_id: 0, virtual_lamp_id: 2 }));
    });
}

fn refused_publishes(log: &mut WireLog) {
    log.step("a refused lamp announce is not recorded and holds its state back", |l| {
        l.port.set_lamp(lamp(4, "Desk", brightness(), false));
        l.bridge.mock.fail_next_publishes(1);
        l.event(runtime_frame(4, 10));
    }, HALL_STATE);
    log.step("the next commit announces the lamp", |l| l.event(runtime_frame(4, 11)), HALL_STATE);
    log.fenced("a refused lamp retraction", |l| {
        l.port.remove_lamp(4);
        l.bridge.mock.fail_next_publishes(1);
        l.event(registry_event(VirtualLampChangedEvent { adapter_id: 0, virtual_lamp_id: 4 }));
    });
}

fn input_changes(log: &mut WireLog) {
    log.step("a press publishes its event", |l| l.event(button_press_frame(PANEL, 0)), PANEL_STATE);
    log.fenced("a renamed input device is announced again", |l| {
        l.port.set_input_device(panel("Hall panel", true));
        l.event(registry_event(InputDeviceChangedEvent { adapter_id: 0, short_address: PANEL }));
    });
    log.fenced("an input device no longer exposed is retracted", |l| {
        l.port.set_input_device(panel("Hall panel", false));
        l.event(registry_event(InputDeviceChangedEvent { adapter_id: 0, short_address: PANEL }));
    });
}

fn seeded_registry() -> Arc<StubHaReadPort> {
    let port = StubHaReadPort::with_lamp(1);
    port.limit_ids(LAMP_IDS, GROUP_IDS);
    port.bind_to_short(BOUND_SHORT);
    port.set_lamp(lamp(2, "Porch", tunable(), true));
    for (group_id, name) in [(5, "Hall"), (6, "Kitchen"), (8, "Stairs")] {
        port.set_group(group(group_id, name, brightness()));
    }
    port.set_scenes(vec![scene(3, "Evening")]);
    port.set_input_device(panel("Panel 0", true));
    port.set_input_device(input_device(HIDDEN_PANEL, "Hidden", false, &[(0, Some(1))]));
    port.set_input_device(input_device(DOORBELL, "Doorbell", true, &[(0, Some(1))]));
    port
}

fn panel(name: &str, exposed: bool) -> HaInputView {
    let instances = [(0, Some(1)), (1, Some(3)), (2, Some(2)), (3, Some(4)), (4, None)];
    input_device(PANEL, name, exposed, &instances)
}

fn input_device(short_address: u8, name: &str, exposed: bool, instances: &[(u8, Option<u8>)]) -> HaInputView {
    HaInputView {
        short_address,
        name: Some(name.to_string()),
        ha_expose: exposed,
        instances: instances
            .iter()
            .map(|&(instance_number, instance_type)| HaInputInstanceView { instance_number, instance_type })
            .collect(),
    }
}

fn brightness() -> CapabilityFlagsView {
    CapabilityFlagsView { brightness: true, ..Default::default() }
}

fn tunable() -> CapabilityFlagsView {
    CapabilityFlagsView { brightness: true, cct: true, ..Default::default() }
}

fn lamp(virtual_lamp_id: u8, name: &str, capabilities: CapabilityFlagsView, unreachable: bool) -> HaLampView {
    HaLampView {
        virtual_lamp_id,
        name: name.to_string(),
        ha_entity_enabled: true,
        capabilities,
        color_temperature_range: capabilities
            .cct
            .then_some(ColorTemperatureRangeView { min_kelvin: 2700, max_kelvin: 6500 }),
        unreachable,
    }
}

fn group(group_id: u8, name: &str, capabilities: CapabilityFlagsView) -> HaGroupView {
    HaGroupView {
        group_id,
        name: name.to_string(),
        ha_entity_enabled: true,
        capabilities,
        state: Default::default(),
    }
}

fn scene(scene_id: u8, name: &str) -> HaSceneView {
    HaSceneView { scene_id, name: name.to_string(), ha_select_enabled: true }
}

struct WireLog {
    bridge: Harness,
    port: Arc<StubHaReadPort>,
    cursor: usize,
    text: String,
}

impl WireLog {
    fn event(&self, frame: BusFrame) {
        assert_eq!(self.bridge.publisher.try_publish(BusChannel::Events, frame), PublishResult::Queued);
    }

    fn manual_republish(&mut self) {
        let envelope = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            MANUAL_RUN,
            BusId::default().0,
            Some(Origin::Api),
            HomeAssistantDiscoveryPublishCommand {},
        );
        let queued = self.bridge.publisher.try_publish(BusChannel::Commands, BusFrame::command(envelope));
        assert_eq!(queued, PublishResult::Queued);
    }

    fn fenced(&mut self, name: &str, act: impl FnOnce(&mut Self)) {
        self.step(name, |l| {
            act(l);
            l.event(button_press_frame(DOORBELL, 0));
        }, DOORBELL_STATE);
    }

    fn step(&mut self, name: &str, act: impl FnOnce(&mut Self), barrier: &str) {
        act(self);
        let mock = Arc::clone(&self.bridge.mock);
        let from = self.cursor;
        let reached = !remains_false_for(
            move || mock.published().get(from..).is_some_and(|sent| sent.iter().any(|m| m.topic == barrier)),
            WAIT,
        );
        let published = self.bridge.mock.published();
        let _ = writeln!(self.text, "## {name}");
        for message in &published[self.cursor..] {
            let _ = writeln!(self.text, "{}", wire_line(message));
        }
        self.cursor = published.len();
        assert!(reached, "step '{name}' never published on {barrier}:\n{}", self.text);
    }

    fn finish(mut self) -> String {
        let counters = Arc::clone(&self.bridge.counters);
        let mock = Arc::clone(&self.bridge.mock);
        let settled = !remains_false_for(
            move || MqttCounters::load(&counters.publishes_total) as usize + 1 == mock.published().len(),
            WAIT,
        );
        assert!(settled, "the worker never counted its last publish:\n{}", self.text);
        let c = &self.bridge.counters;
        let _ = writeln!(
            self.text,
            "## counters: publishes {} refused {} discovery published {} discovery failed {}",
            MqttCounters::load(&c.publishes_total),
            MqttCounters::load(&c.publish_failures_total),
            MqttCounters::load(&c.discovery_published_total),
            MqttCounters::load(&c.discovery_failed_total),
        );
        self.text
    }
}

fn wire_line(message: &PublishedMessage) -> String {
    let qos = match message.qos {
        MqttQos::AtMostOnce => "q0",
        MqttQos::AtLeastOnce => "q1",
    };
    let retain = if message.retain { "retained" } else { "volatile" };
    let payload = String::from_utf8_lossy(&message.payload);
    let payload = if payload.is_empty() { EMPTY_PAYLOAD.into() } else { payload };
    format!("{qos} {retain} {} {payload}", message.topic)
}

fn assert_matches_golden(name: &str, actual: &str, golden: &str) {
    if actual == golden {
        return;
    }
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.actual.txt"));
    std::fs::write(&path, actual).expect("the actual recording is written beside the build");
    let line = actual.lines().zip(golden.lines()).position(|(a, g)| a != g);
    let line = line.unwrap_or_else(|| actual.lines().count().min(golden.lines().count()));
    panic!(
        "the publishes differ from tests/golden/{name}.txt at line {}:\n  expected: {:?}\n  actual:   {:?}\nfull recording: {}",
        line + 1,
        golden.lines().nth(line),
        actual.lines().nth(line),
        path.display(),
    );
}
