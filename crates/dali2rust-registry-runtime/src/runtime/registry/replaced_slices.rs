use std::collections::BTreeMap;

use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{FixedText32, APPLICATION_ACTIVE_UNMOVED};
use dali2rust_contracts::CORRELATION_NONE;
use dali2rust_platform::slice_store::SliceKey;

use super::dali_settings::DaliSettingsRecord;
use super::physical_device_banks::{bank_bit, short_bit};
use super::policies::PoliciesRecord;
use super::poller_settings::PollerSettingsRecord;
use super::publish::{
    publish_adapter_settings_changed, publish_dali_settings_changed, publish_group_changed,
    publish_group_matrix_changed, publish_hcl_schedule_changed,
    publish_home_assistant_settings_changed, publish_input_device_changed,
    publish_physical_device_changed, publish_policies_changed, publish_poller_settings_changed,
    publish_redundancy_settings_changed, publish_scene_changed, publish_scene_matrix_changed,
    publish_virtual_lamp_changed,
};
use super::redundancy_settings::RedundancySettingsRecord;
use super::store::{Inner, RegistryStore};
use super::withheld_slices::HeldSlices;

const POLLER_SETTINGS: u8 = 1 << 0;
const DALI_SETTINGS: u8 = 1 << 1;
const REDUNDANCY_SETTINGS: u8 = 1 << 2;
const POLICIES: u8 = 1 << 3;
const HOME_ASSISTANT_SETTINGS: u8 = 1 << 4;
const ID_SPACE: u8 = 64;
const EVERY_DEVICE_BANK: u16 = u16::MAX;

fn bit(id: u8) -> u64 {
    1u64.checked_shl(u32::from(id)).unwrap_or(0)
}

fn ids(mask: u64) -> impl Iterator<Item = u8> {
    (0..ID_SPACE).filter(move |id| mask & bit(*id) != 0)
}

fn ids_of<'a>(keys: impl Iterator<Item = &'a (u8, u8)>, adapter_id: u8) -> u64 {
    keys.filter(|(aid, _)| *aid == adapter_id).fold(0, |mask, (_, id)| mask | bit(*id))
}

fn adapter_ids(inner: &Inner) -> u64 {
    let count = u8::try_from(inner.adapters.len()).unwrap_or(ID_SPACE);
    (0..count).fold(0, |mask, id| mask | bit(id))
}

fn or_into(map: &mut BTreeMap<u8, u64>, adapter_id: u8, mask: u64) {
    if mask != 0 {
        *map.entry(adapter_id).or_default() |= mask;
    }
}

fn merge_maps(into: &mut BTreeMap<u8, u64>, from: BTreeMap<u8, u64>) {
    for (adapter_id, mask) in from {
        or_into(into, adapter_id, mask);
    }
}

fn device_banks(key: SliceKey) -> Option<(u8, u16)> {
    match key {
        SliceKey::PhysicalDevices { adapter_id } => Some((adapter_id, EVERY_DEVICE_BANK)),
        SliceKey::PhysicalDeviceBank { adapter_id, bank } => {
            Some((adapter_id, 1u16.checked_shl(u32::from(bank)).unwrap_or(0)))
        }
        _ => None,
    }
}

fn record_bit(key: SliceKey) -> Option<u8> {
    match key {
        SliceKey::PollerSettings => Some(POLLER_SETTINGS),
        SliceKey::DaliSettings => Some(DALI_SETTINGS),
        SliceKey::RedundancySettings => Some(REDUNDANCY_SETTINGS),
        SliceKey::Policies => Some(POLICIES),
        SliceKey::HomeAssistantSettings => Some(HOME_ASSISTANT_SETTINGS),
        _ => None,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReplacedEntities {
    adapters: u64,
    groups: BTreeMap<u8, u64>,
    group_matrices: u64,
    lamps: BTreeMap<u8, u64>,
    devices: BTreeMap<u8, u64>,
    scenes: BTreeMap<u8, u64>,
    input_devices: Vec<(u8, u8)>,
    schedules: Vec<FixedText32>,
    records: u8,
}

impl ReplacedEntities {
    fn note(&mut self, inner: &Inner, key: SliceKey) {
        if let Some(record) = record_bit(key) {
            self.records |= record;
        } else if let Some((adapter_id, banks)) = device_banks(key) {
            self.note_devices(inner, adapter_id, banks);
        } else {
            self.note_collection(inner, key);
        }
    }

    fn note_collection(&mut self, inner: &Inner, key: SliceKey) {
        let lamps_of = |adapter_id| ids_of(inner.lamps.keys(), adapter_id);
        match key {
            SliceKey::Adapters => self.adapters |= adapter_ids(inner),
            SliceKey::Groups { adapter_id } => self.note_groups(inner, adapter_id),
            SliceKey::VirtualLamps { adapter_id } => {
                or_into(&mut self.lamps, adapter_id, lamps_of(adapter_id));
            }
            SliceKey::Scene { adapter_id, scene_id } => {
                or_into(&mut self.scenes, adapter_id, bit(scene_id));
            }
            SliceKey::InputDevices { .. } => {
                self.input_devices.extend(inner.input_devices.keys().copied());
            }
            SliceKey::HclSchedules => self.schedules.extend(inner.hcl_schedules.keys().cloned()),
            _ => {}
        }
    }

    fn note_groups(&mut self, inner: &Inner, adapter_id: u8) {
        or_into(&mut self.groups, adapter_id, ids_of(inner.groups.keys(), adapter_id));
        self.group_matrices |= bit(adapter_id);
    }

    fn note_devices(&mut self, inner: &Inner, adapter_id: u8, banks: u16) {
        let in_banks = |short: u8| banks & bank_bit(short) != 0;
        let shorts = inner
            .physical_devices
            .keys()
            .filter(|(aid, short)| *aid == adapter_id && in_banks(*short))
            .fold(0, |mask, (_, short)| mask | short_bit(*short));
        let lamps = inner
            .lamps
            .iter()
            .filter(|((aid, _), _)| *aid == adapter_id)
            .filter(|(_, lamp)| lamp.binding_short.is_some_and(in_banks))
            .fold(0, |mask, ((_, id), _)| mask | bit(*id));
        or_into(&mut self.devices, adapter_id, shorts);
        or_into(&mut self.lamps, adapter_id, lamps);
    }

    fn merge(&mut self, other: Self) {
        self.adapters |= other.adapters;
        merge_maps(&mut self.groups, other.groups);
        self.group_matrices |= other.group_matrices;
        merge_maps(&mut self.lamps, other.lamps);
        merge_maps(&mut self.devices, other.devices);
        merge_maps(&mut self.scenes, other.scenes);
        self.input_devices.extend(other.input_devices);
        self.schedules.extend(other.schedules);
        self.records |= other.records;
    }

    pub(crate) fn publish(mut self, publisher: &BusPublisher, store: &RegistryStore, target: u16) {
        self.input_devices.sort_unstable();
        self.input_devices.dedup();
        self.schedules.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        self.schedules.dedup();
        publish_adapters(publisher, store, self.adapters);
        self.publish_collections(publisher);
        self.publish_devices(publisher);
        publish_schedules(publisher, store, target, &self.schedules);
        publish_records(publisher, store, target, self.records);
    }

    fn publish_collections(&self, publisher: &BusPublisher) {
        each_id(&self.groups, |adapter_id, id| {
            publish_group_changed(publisher, CORRELATION_NONE, adapter_id, id);
        });
        for adapter_id in ids(self.group_matrices) {
            publish_group_matrix_changed(publisher, CORRELATION_NONE, adapter_id);
        }
        each_id(&self.scenes, |adapter_id, scene_id| {
            publish_scene_changed(publisher, CORRELATION_NONE, adapter_id, scene_id);
            publish_scene_matrix_changed(publisher, CORRELATION_NONE, adapter_id, scene_id);
        });
    }

    fn publish_devices(&self, publisher: &BusPublisher) {
        each_id(&self.devices, |adapter_id, short| {
            publish_physical_device_changed(publisher, CORRELATION_NONE, adapter_id, short);
        });
        each_id(&self.lamps, |adapter_id, id| {
            publish_virtual_lamp_changed(publisher, CORRELATION_NONE, adapter_id, id);
        });
        for (adapter_id, short) in &self.input_devices {
            publish_input_device_changed(publisher, CORRELATION_NONE, *adapter_id, *short);
        }
    }
}

#[cfg(test)]
impl ReplacedEntities {
    pub(crate) fn lamps_of(&self, adapter_id: u8) -> u64 {
        self.lamps.get(&adapter_id).copied().unwrap_or(0)
    }

    pub(crate) fn devices_of(&self, adapter_id: u8) -> u64 {
        self.devices.get(&adapter_id).copied().unwrap_or(0)
    }
}

fn each_id(map: &BTreeMap<u8, u64>, mut publish: impl FnMut(u8, u8)) {
    for (adapter_id, mask) in map {
        ids(*mask).for_each(|id| publish(*adapter_id, id));
    }
}

fn publish_adapters(publisher: &BusPublisher, store: &RegistryStore, adapters: u64) {
    for adapter_id in ids(adapters) {
        if let Some((name, enabled)) = store.adapter_row_name_enabled(adapter_id) {
            let name = name.as_str();
            publish_adapter_settings_changed(publisher, adapter_id, CORRELATION_NONE, name, enabled);
        }
    }
}

fn publish_schedules(
    publisher: &BusPublisher,
    store: &RegistryStore,
    target: u16,
    schedules: &[FixedText32],
) {
    for schedule_id in schedules {
        let enabled = store.read_inner().hcl_schedules.get(schedule_id).map(|row| row.enabled);
        let (removed, enabled) = (enabled.is_none(), enabled.unwrap_or(false));
        let id = schedule_id.clone();
        publish_hcl_schedule_changed(publisher, CORRELATION_NONE, target, id, removed, enabled);
    }
}

fn publish_records(publisher: &BusPublisher, store: &RegistryStore, target: u16, records: u8) {
    if records != 0 {
        let settings = Settings::of(&store.read_inner());
        settings.publish(publisher, target, records);
    }
}

struct Settings {
    poller: PollerSettingsRecord,
    dali: DaliSettingsRecord,
    redundancy: RedundancySettingsRecord,
    policies: PoliciesRecord,
    home_assistant: bool,
}

impl Settings {
    fn of(inner: &Inner) -> Self {
        Self {
            poller: inner.poller_settings,
            dali: inner.dali_settings,
            redundancy: inner.redundancy_settings.clone(),
            policies: inner.policies,
            home_assistant: inner.home_assistant_settings.enabled,
        }
    }

    fn publish(&self, publisher: &BusPublisher, target: u16, records: u8) {
        let (corr, announced) = (CORRELATION_NONE, |record: u8| records & record != 0);
        if announced(POLLER_SETTINGS) {
            publish_poller_settings_changed(publisher, corr, target, &self.poller);
        }
        if announced(DALI_SETTINGS) {
            let unmoved = APPLICATION_ACTIVE_UNMOVED;
            publish_dali_settings_changed(publisher, corr, target, &self.dali, unmoved);
        }
        if announced(REDUNDANCY_SETTINGS) {
            publish_redundancy_settings_changed(publisher, corr, target, &self.redundancy);
        }
        if announced(POLICIES) {
            publish_policies_changed(publisher, corr, target, &self.policies);
        }
        if announced(HOME_ASSISTANT_SETTINGS) {
            publish_home_assistant_settings_changed(publisher, corr, target, self.home_assistant);
        }
    }
}

pub(crate) struct ReloadWatch {
    held: HeldSlices,
    written: Vec<SliceKey>,
    before: Vec<(SliceKey, ReplacedEntities)>,
}

impl RegistryStore {
    pub(crate) fn watch_reload(&self, written: &[SliceKey], adapter_count: u8) -> ReloadWatch {
        let held = self.dirty.held();
        let waiting = held
            .keys(adapter_count)
            .filter(|key| !matches!(key, SliceKey::PhysicalDevices { .. }));
        let inner = self.read_inner();
        let keys = written.iter().copied().chain(waiting);
        let before = keys.map(|key| (key, entities_of(&inner, key))).collect();
        ReloadWatch { held, written: written.to_vec(), before }
    }

    pub(crate) fn replaced_entities(&self, watch: ReloadWatch) -> ReplacedEntities {
        let still = self.dirty.held();
        let inner = self.read_inner();
        let mut replaced = ReplacedEntities::default();
        for (key, before) in watch.before {
            if watch.written.contains(&key) || (watch.held.reads(key) && !still.reads(key)) {
                replaced.merge(before);
                replaced.note(&inner, key);
            }
        }
        replaced
    }
}

fn entities_of(inner: &Inner, key: SliceKey) -> ReplacedEntities {
    let mut entities = ReplacedEntities::default();
    entities.note(inner, key);
    entities
}
