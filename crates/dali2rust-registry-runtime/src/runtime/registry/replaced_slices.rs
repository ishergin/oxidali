use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{FixedText32, APPLICATION_ACTIVE_UNMOVED};
use dali2rust_contracts::CORRELATION_NONE;
use dali2rust_platform::slice_store::SliceKey;

use super::physical_device_banks::{bank_bit, short_bit};
use super::publish::{
    publish_adapter_settings_changed, publish_dali_settings_changed, publish_group_changed,
    publish_group_matrix_changed, publish_hcl_schedule_changed,
    publish_home_assistant_settings_changed, publish_input_device_changed,
    publish_physical_device_changed, publish_policies_changed, publish_poller_settings_changed,
    publish_redundancy_settings_changed, publish_scene_changed, publish_scene_matrix_changed,
    publish_virtual_lamp_changed,
};
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

fn each<'a>(
    map: &'a BTreeMap<u8, u64>,
    make: impl Fn(u8, u8) -> Announcement + Copy + 'a,
) -> impl Iterator<Item = Announcement> + 'a {
    map.iter().flat_map(move |(adapter_id, mask)| ids(*mask).map(move |id| make(*adapter_id, id)))
}

const SETTINGS: [(u8, Setting); 5] = [
    (POLLER_SETTINGS, Setting::Poller),
    (DALI_SETTINGS, Setting::Dali),
    (REDUNDANCY_SETTINGS, Setting::Redundancy),
    (POLICIES, Setting::Policies),
    (HOME_ASSISTANT_SETTINGS, Setting::HomeAssistant),
];

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

    fn queue_into(self, queue: &mut BTreeSet<Announcement>) {
        queue.extend(ids(self.adapters).map(Announcement::Adapter));
        queue.extend(ids(self.group_matrices).map(Announcement::GroupMatrix));
        queue.extend(each(&self.groups, |adapter_id, group_id| Announcement::Group {
            adapter_id,
            group_id,
        }));
        queue.extend(each(&self.scenes, |adapter_id, scene_id| Announcement::Scene {
            adapter_id,
            scene_id,
        }));
        queue.extend(each(&self.scenes, |adapter_id, scene_id| Announcement::SceneMatrix {
            adapter_id,
            scene_id,
        }));
        self.queue_records_into(queue);
    }

    fn queue_records_into(self, queue: &mut BTreeSet<Announcement>) {
        let device = |adapter_id, short_address| Announcement::Device { adapter_id, short_address };
        let lamp = |adapter_id, virtual_lamp_id| Announcement::Lamp { adapter_id, virtual_lamp_id };
        queue.extend(each(&self.devices, device));
        queue.extend(each(&self.lamps, lamp));
        let inputs = self.input_devices.into_iter();
        queue.extend(inputs.map(|(adapter_id, short_address)| Announcement::InputDevice {
            adapter_id,
            short_address,
        }));
        queue.extend(self.schedules.into_iter().map(Announcement::Schedule));
        let records = SETTINGS.into_iter().filter(|(bit, _)| self.records & bit != 0);
        queue.extend(records.map(|(_, setting)| Announcement::Setting(setting)));
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Setting {
    Poller,
    Dali,
    Redundancy,
    Policies,
    HomeAssistant,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Announcement {
    Adapter(u8),
    Group { adapter_id: u8, group_id: u8 },
    GroupMatrix(u8),
    Scene { adapter_id: u8, scene_id: u8 },
    SceneMatrix { adapter_id: u8, scene_id: u8 },
    Device { adapter_id: u8, short_address: u8 },
    Lamp { adapter_id: u8, virtual_lamp_id: u8 },
    InputDevice { adapter_id: u8, short_address: u8 },
    Schedule(FixedText32),
    Setting(Setting),
}

impl Announcement {
    fn publish(&self, publisher: &BusPublisher, store: &RegistryStore, target: u16) {
        match self {
            Self::Adapter(adapter_id) => publish_adapter(publisher, store, *adapter_id),
            Self::Schedule(schedule_id) => publish_schedule(publisher, store, target, schedule_id),
            Self::Setting(setting) => publish_setting(publisher, store, target, *setting),
            Self::InputDevice { adapter_id: a, short_address: s } => {
                publish_input_device_changed(publisher, CORRELATION_NONE, *a, *s);
            }
            identity => identity.publish_identity(publisher),
        }
    }

    fn publish_identity(&self, p: &BusPublisher) {
        let c = CORRELATION_NONE;
        match self {
            Self::Group { adapter_id: a, group_id: g } => publish_group_changed(p, c, *a, *g),
            Self::GroupMatrix(a) => publish_group_matrix_changed(p, c, *a),
            Self::Scene { adapter_id: a, scene_id: s } => publish_scene_changed(p, c, *a, *s),
            Self::SceneMatrix { adapter_id: a, scene_id: s } => {
                publish_scene_matrix_changed(p, c, *a, *s);
            }
            Self::Device { adapter_id: a, short_address: s } => {
                publish_physical_device_changed(p, c, *a, *s);
            }
            Self::Lamp { adapter_id: a, virtual_lamp_id: l } => {
                publish_virtual_lamp_changed(p, c, *a, *l);
            }
            _ => {}
        }
    }
}

fn publish_adapter(publisher: &BusPublisher, store: &RegistryStore, adapter_id: u8) {
    if let Some((name, enabled)) = store.adapter_row_name_enabled(adapter_id) {
        let name = name.as_str();
        publish_adapter_settings_changed(publisher, adapter_id, CORRELATION_NONE, name, enabled);
    }
}

fn publish_schedule(
    publisher: &BusPublisher,
    store: &RegistryStore,
    target: u16,
    schedule_id: &FixedText32,
) {
    let enabled = store.read_inner().hcl_schedules.get(schedule_id).map(|row| row.enabled);
    let (removed, enabled) = (enabled.is_none(), enabled.unwrap_or(false));
    let id = schedule_id.clone();
    publish_hcl_schedule_changed(publisher, CORRELATION_NONE, target, id, removed, enabled);
}

fn publish_setting(publisher: &BusPublisher, store: &RegistryStore, target: u16, setting: Setting) {
    let corr = CORRELATION_NONE;
    match setting {
        Setting::Poller => {
            let poller = store.read_inner().poller_settings;
            publish_poller_settings_changed(publisher, corr, target, &poller);
        }
        Setting::Dali => {
            let dali = store.read_inner().dali_settings;
            let unmoved = APPLICATION_ACTIVE_UNMOVED;
            publish_dali_settings_changed(publisher, corr, target, &dali, unmoved);
        }
        Setting::Redundancy => {
            let redundancy = store.read_inner().redundancy_settings.clone();
            publish_redundancy_settings_changed(publisher, corr, target, &redundancy);
        }
        policy => publish_policy_setting(publisher, store, target, policy),
    }
}

fn publish_policy_setting(
    publisher: &BusPublisher,
    store: &RegistryStore,
    target: u16,
    setting: Setting,
) {
    let corr = CORRELATION_NONE;
    if setting == Setting::Policies {
        let policies = store.read_inner().policies;
        publish_policies_changed(publisher, corr, target, &policies);
    } else {
        let enabled = store.read_inner().home_assistant_settings.enabled;
        publish_home_assistant_settings_changed(publisher, corr, target, enabled);
    }
}

#[derive(Default)]
pub(crate) struct PendingAnnouncements(Mutex<BTreeSet<Announcement>>);

impl PendingAnnouncements {
    fn queue(&self) -> MutexGuard<'_, BTreeSet<Announcement>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RegistryStore {
    #[inline(never)]
    pub(crate) fn queue_announcements(&self, replaced: ReplacedEntities) {
        replaced.queue_into(&mut self.announcements.queue());
    }

    #[inline(never)]
    pub(crate) fn announce_queued(&self, publisher: &BusPublisher, target: u16, budget: usize) {
        let batch: Vec<Announcement> = {
            let mut queue = self.announcements.queue();
            (0..budget).map_while(|_| queue.pop_first()).collect()
        };
        for announcement in &batch {
            announcement.publish(publisher, self, target);
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use dali2rust_bus::{BusConfig, BusFrame, BusHost, BusPublisher};
    use dali2rust_contracts::msg::{BusEventPayload, EVENT_VARIANT_NAMES};

    use super::{ReplacedEntities, POLLER_SETTINGS};
    use crate::runtime::registry::RegistryStore;

    const INBOX: usize = 128;
    const BUDGET: usize = 16;
    const QUEUED_LAMPS: u8 = 40;
    const LATER_INTERVAL_MS: u32 = 4_321;
    const WAIT: Duration = Duration::from_secs(2);
    const QUIET: Duration = Duration::from_millis(200);
    const TARGET: u16 = 1;

    fn observed() -> (BusHost, BusPublisher, Receiver<BusFrame>) {
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_events(INBOX, EVENT_VARIANT_NAMES))
    }

    fn heard_until_quiet(events: &Receiver<BusFrame>) -> Vec<BusEventPayload> {
        let mut heard = Vec::new();
        let mut wait = WAIT;
        while let Ok(BusFrame::Event(event)) = events.recv_timeout(wait) {
            heard.push(event.payload.clone());
            wait = QUIET;
        }
        heard
    }

    #[test]
    fn a_record_queued_twice_goes_out_once_with_the_state_it_has_then() {
        let (_host, publisher, events) = observed();
        let store = RegistryStore::with_adapter_count(1);
        let poller = || ReplacedEntities { records: POLLER_SETTINGS, ..Default::default() };
        store.queue_announcements(poller());
        store.queue_announcements(poller());
        store.write_inner().poller_settings.interval_ms = LATER_INTERVAL_MS;
        store.announce_queued(&publisher, TARGET, BUDGET);
        let heard = heard_until_quiet(&events);
        let intervals: Vec<u32> = heard
            .iter()
            .filter_map(|payload| match payload {
                BusEventPayload::PollerSettingsChangedEvent(ev) => Some(ev.interval_ms),
                _ => None,
            })
            .collect();
        assert_eq!(intervals, vec![LATER_INTERVAL_MS], "one announcement, read when it went out");
    }

    #[test]
    fn a_turn_announces_no_more_than_its_budget_and_the_rest_wait_for_the_next() {
        let (_host, publisher, events) = observed();
        let store = RegistryStore::with_adapter_count(1);
        let lamps = (0..QUEUED_LAMPS).fold(0u64, |mask, id| mask | (1 << id));
        let lamps = BTreeMap::from([(0, lamps)]);
        let queued = ReplacedEntities { lamps, ..ReplacedEntities::default() };
        store.queue_announcements(queued);
        let mut turns = Vec::new();
        while turns.iter().sum::<usize>() < usize::from(QUEUED_LAMPS) && turns.len() < 4 {
            store.announce_queued(&publisher, TARGET, BUDGET);
            turns.push(heard_until_quiet(&events).len());
        }
        assert_eq!(turns, vec![BUDGET, BUDGET, usize::from(QUEUED_LAMPS) - 2 * BUDGET]);
    }
}
