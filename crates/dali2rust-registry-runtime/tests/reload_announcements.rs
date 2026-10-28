mod support;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bus::{BusFrame, BusId};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    fixed_text_32, BusEventPayload, ColorMode, DeviceType, OperationWorkerSignal, Origin,
    RegistrySliceReloadCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_registry_runtime::{
    encode_persistence_blob, PersistableGroupRecord, PersistableGroupsSlice,
    PersistablePhysicalDeviceRecord, PersistablePhysicalDevicesSlice,
    PersistablePollerSettingsSlice, PersistableSceneSlice, PersistableVirtualLampsSlice,
    PersistableVlRecord, PersistenceEnvelope, StagedSlice, ANNOUNCEMENT_INBOX_FLOOR,
    GROUPS_SLICE_VERSION, PHYSICAL_DEVICES_SLICE_VERSION, POLLER_SETTINGS_SLICE_VERSION,
    SCENES_SLICE_VERSION, VIRTUAL_LAMPS_SLICE_VERSION,
};
use dali2rust_test_support::temp_slice_store;
use support::{publish_cmd, spawn_registry_stack_with, RegistryStackOptions, RegistryTestStack};

const ADAPTERS: u8 = 2;
const LAMPS_PER_ADAPTER: u8 = 64;
const GROUPS_PER_ADAPTER: u8 = 16;
const DEVICE_BANKS_PER_ADAPTER: u8 = 2;
const SCENES_PER_ADAPTER: u8 = 4;
const IMPORTED_POLLER_INTERVAL_MS: u32 = 8_000;
const IMPORT_WORKFLOW: u64 = 77;
const ANNOUNCED_WITHIN: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(50);

fn blob<T: serde::Serialize>(version: u32, slice: T) -> Vec<u8> {
    encode_persistence_blob(&PersistenceEnvelope::new(version, slice)).expect("encode slice")
}

fn lamps(adapter_id: u8) -> StagedSlice {
    let lamp = |virtual_lamp_id| PersistableVlRecord {
        virtual_lamp_id,
        name: format!("lamp {virtual_lamp_id}"),
        ha_entity_enabled: true,
        binding_short: None,
    };
    let lamps = (0..LAMPS_PER_ADAPTER).map(lamp).collect();
    let slice = PersistableVirtualLampsSlice { adapter_id, lamps };
    let bytes = blob(VIRTUAL_LAMPS_SLICE_VERSION, slice);
    StagedSlice { key: SliceKey::VirtualLamps { adapter_id }, bytes }
}

fn groups(adapter_id: u8) -> StagedSlice {
    let group = |group_id| PersistableGroupRecord {
        group_id,
        name: format!("group {group_id}"),
        ha_entity_enabled: true,
    };
    let groups = (0..GROUPS_PER_ADAPTER).map(group).collect();
    let slice = PersistableGroupsSlice { adapter_id, groups, rows: Vec::new() };
    StagedSlice { key: SliceKey::Groups { adapter_id }, bytes: blob(GROUPS_SLICE_VERSION, slice) }
}

fn device(short_address: u8) -> PersistablePhysicalDeviceRecord {
    PersistablePhysicalDeviceRecord {
        short_address,
        random_address: None,
        name: format!("device {short_address}"),
        notes: None,
        device_type_override: None,
        color_mode_override: None,
        attributes: Default::default(),
        memory_banks: Vec::new(),
        device_type_discovered: DeviceType::Dt6Led,
        color_mode_discovered: ColorMode::Brightness,
        cap_cct: false,
        cap_xy: false,
        cap_rgb: false,
        tc_coolest_mirek: None,
        tc_warmest_mirek: None,
        dt8_auto_activation_repair: None,
        dt8_rgbwaf_control_assert: None,
        cap_rgbwaf: false,
        supported_device_types: None,
    }
}

fn device_bank(adapter_id: u8, bank: u8) -> StagedSlice {
    let first = bank * SliceKey::DEVICES_PER_BANK;
    let devices = (first..first + SliceKey::DEVICES_PER_BANK).map(device).collect();
    let slice = PersistablePhysicalDevicesSlice { adapter_id, devices };
    let key = SliceKey::PhysicalDeviceBank { adapter_id, bank };
    StagedSlice { key, bytes: blob(PHYSICAL_DEVICES_SLICE_VERSION, slice) }
}

fn scene(adapter_id: u8, scene_id: u8) -> StagedSlice {
    let name = format!("scene {scene_id}");
    let rows = Vec::new();
    let slice = PersistableSceneSlice { adapter_id, scene_id, name, ha_select_enabled: true, rows };
    let key = SliceKey::Scene { adapter_id, scene_id };
    StagedSlice { key, bytes: blob(SCENES_SLICE_VERSION, slice) }
}

fn poller_settings() -> StagedSlice {
    let slice = PersistablePollerSettingsSlice {
        enabled: true,
        interval_ms: IMPORTED_POLLER_INTERVAL_MS,
        attribute_groups_mask: 1,
        include_dt8_color: false,
        skip_unbound_virtual_lamps: true,
        include_energy: false,
        include_diagnostics: false,
        power_cycle_watch: false,
    };
    StagedSlice { key: SliceKey::PollerSettings, bytes: blob(POLLER_SETTINGS_SLICE_VERSION, slice) }
}

fn of_adapter(adapter_id: u8) -> Vec<StagedSlice> {
    let banks = (0..DEVICE_BANKS_PER_ADAPTER).map(|bank| device_bank(adapter_id, bank));
    let scenes = (0..SCENES_PER_ADAPTER).map(|scene_id| scene(adapter_id, scene_id));
    [lamps(adapter_id), groups(adapter_id)].into_iter().chain(banks).chain(scenes).collect()
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum Heard {
    Lamp(u8, u8),
    Group(u8, u8),
    GroupMatrix(u8),
    Device(u8, u8),
    Scene(u8, u8),
    SceneMatrix(u8, u8),
    PollerSettings(u32),
}

fn heard(payload: &BusEventPayload) -> Option<Heard> {
    Some(match payload {
        BusEventPayload::VirtualLampChangedEvent(e) => Heard::Lamp(e.adapter_id, e.virtual_lamp_id),
        BusEventPayload::GroupChangedEvent(e) => Heard::Group(e.adapter_id, e.group_id),
        BusEventPayload::GroupMatrixChangedEvent(e) => Heard::GroupMatrix(e.adapter_id),
        BusEventPayload::PhysicalDeviceChangedEvent(e) => {
            Heard::Device(e.adapter_id, e.short_address)
        }
        BusEventPayload::SceneChangedEvent(e) => Heard::Scene(e.adapter_id, e.scene_id),
        BusEventPayload::SceneMatrixChangedEvent(e) => Heard::SceneMatrix(e.adapter_id, e.scene_id),
        BusEventPayload::PollerSettingsChangedEvent(e) => Heard::PollerSettings(e.interval_ms),
        _ => return None,
    })
}

fn expected_of(adapter_id: u8) -> impl Iterator<Item = Heard> {
    let lamps = (0..LAMPS_PER_ADAPTER).map(move |id| Heard::Lamp(adapter_id, id));
    let groups = (0..GROUPS_PER_ADAPTER).map(move |id| Heard::Group(adapter_id, id));
    let shorts = 0..DEVICE_BANKS_PER_ADAPTER * SliceKey::DEVICES_PER_BANK;
    let devices = shorts.map(move |short| Heard::Device(adapter_id, short));
    let scenes = (0..SCENES_PER_ADAPTER)
        .flat_map(move |id| [Heard::Scene(adapter_id, id), Heard::SceneMatrix(adapter_id, id)]);
    let matrix = std::iter::once(Heard::GroupMatrix(adapter_id));
    lamps.chain(groups).chain(devices).chain(scenes).chain(matrix)
}

fn expected() -> HashMap<Heard, usize> {
    let records = (0..ADAPTERS).flat_map(expected_of);
    let settings = std::iter::once(Heard::PollerSettings(IMPORTED_POLLER_INTERVAL_MS));
    records.chain(settings).map(|record| (record, 1)).collect()
}

fn commit(stack: &RegistryTestStack) {
    let slices = (0..ADAPTERS).flat_map(of_adapter).chain(std::iter::once(poller_settings()));
    stack.store.stage_import(IMPORT_WORKFLOW, slices.collect()).expect("stage");
    let reload = RegistrySliceReloadCommand { slice_name: fixed_text_32("virtual_lamps") };
    let (target, origin) = (BusId::default().0, Some(Origin::Api));
    let envelope = command_envelope(SOURCE_ID_UNSPECIFIED, IMPORT_WORKFLOW, target, origin, reload);
    publish_cmd(&stack.publisher, envelope);
}

fn listen(stack: &RegistryTestStack) -> (HashMap<Heard, usize>, bool) {
    let (mut announced, mut succeeded) = (HashMap::new(), false);
    let deadline = Instant::now() + ANNOUNCED_WITHIN;
    while Instant::now() < deadline && !(succeeded && announced == expected()) {
        let Ok(BusFrame::Event(event)) = stack.ev_rx.recv_timeout(POLL) else {
            continue;
        };
        if let BusEventPayload::OperationWorkerSignalEvent(signal) = &event.payload {
            succeeded |= signal.signal == OperationWorkerSignal::WorkerSucceeded;
        }
        if let Some(record) = heard(&event.payload) {
            *announced.entry(record).or_default() += 1;
        }
    }
    (announced, succeeded)
}

#[test]
fn an_import_larger_than_the_events_ingress_announces_each_record_once_without_an_overflow() {
    let slices: Arc<dyn SliceStore> = Arc::new(temp_slice_store("reload-announcements"));
    let stack = spawn_registry_stack_with(RegistryStackOptions {
        adapter_count: ADAPTERS,
        event_observer_capacity: ANNOUNCEMENT_INBOX_FLOOR,
        slices: Some(slices),
        production_subscription: true,
        ..RegistryStackOptions::default()
    });
    commit(&stack);
    let (announced, succeeded) = listen(&stack);
    assert!(succeeded, "the import did not succeed");
    let expected = expected();
    let missing: Vec<_> = expected.keys().filter(|key| !announced.contains_key(*key)).collect();
    assert!(missing.is_empty(), "records never announced: {missing:?}");
    let repeated: Vec<_> = announced.iter().filter(|(_, count)| **count > 1).collect();
    assert!(repeated.is_empty(), "records announced more than once: {repeated:?}");
    let counters = stack.publisher.counters_snapshot();
    let ingress = counters.events.ingress_overflow;
    assert_eq!(ingress, 0, "the announcements overflowed the events ingress");
    let observer = counters.event_subscribers.iter().map(|s| s.receiver_overflow).sum::<u32>();
    assert_eq!(observer, 0, "an inbox of the floor's size overflowed");
}
