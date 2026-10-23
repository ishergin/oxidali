use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dali2rust_bus::{BusChannel, BusFrame, BusPublisher, PublishResult};
use dali2rust_contracts::msg::{
    BusEventPayload, FixedText32, GroupChangedEvent, GroupMatrixChangedEvent,
    HclScheduleChangedEvent, HomeAssistantSettingsChangedEvent, InputDeviceChangedEvent,
    PhysicalDeviceChangedEvent, SceneChangedEvent, SceneMatrixChangedEvent,
    VirtualLampChangedEvent, APPLICATION_ACTIVE_UNMOVED,
};
use dali2rust_contracts::CORRELATION_NONE;
use dali2rust_platform::slice_store::SliceKey;
use log::warn;

use super::physical_device_banks::{bank_bit, short_bit};
use super::publish::{
    adapter_settings_frame, dali_settings_changed, policies_changed, poller_settings_changed,
    redundancy_settings_changed, registry_event_frame,
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
    Setting(Setting),
    Schedule(FixedText32),
    Adapter(u8),
    Group { adapter_id: u8, group_id: u8 },
    GroupMatrix(u8),
    Scene { adapter_id: u8, scene_id: u8 },
    SceneMatrix { adapter_id: u8, scene_id: u8 },
    Device { adapter_id: u8, short_address: u8 },
    Lamp { adapter_id: u8, virtual_lamp_id: u8 },
    InputDevice { adapter_id: u8, short_address: u8 },
}

impl Announcement {
    fn frame(&self, store: &RegistryStore, target: u16) -> Option<BusFrame> {
        match self {
            Self::Setting(setting) => Some(setting_frame(store, target, *setting)),
            Self::Schedule(schedule_id) => Some(schedule_frame(store, target, schedule_id)),
            Self::Adapter(adapter_id) => {
                let (name, enabled) = store.adapter_row_name_enabled(*adapter_id)?;
                Some(adapter_settings_frame(*adapter_id, CORRELATION_NONE, name.as_str(), enabled))
            }
            identity => {
                let (adapter_id, payload) = identity.identity_payload()?;
                Some(registry_event_frame(CORRELATION_NONE, u16::from(adapter_id), payload))
            }
        }
    }

    fn identity_payload(&self) -> Option<(u8, BusEventPayload)> {
        self.collection_payload().or_else(|| self.device_payload())
    }

    fn collection_payload(&self) -> Option<(u8, BusEventPayload)> {
        Some(match *self {
            Self::Group { adapter_id, group_id } => {
                (adapter_id, GroupChangedEvent { adapter_id, group_id }.into())
            }
            Self::GroupMatrix(adapter_id) => {
                (adapter_id, GroupMatrixChangedEvent { adapter_id }.into())
            }
            Self::Scene { adapter_id, scene_id } => {
                (adapter_id, SceneChangedEvent { adapter_id, scene_id }.into())
            }
            Self::SceneMatrix { adapter_id, scene_id } => {
                (adapter_id, SceneMatrixChangedEvent { adapter_id, scene_id }.into())
            }
            _ => return None,
        })
    }

    fn device_payload(&self) -> Option<(u8, BusEventPayload)> {
        Some(match *self {
            Self::Device { adapter_id, short_address } => {
                (adapter_id, PhysicalDeviceChangedEvent { adapter_id, short_address }.into())
            }
            Self::Lamp { adapter_id, virtual_lamp_id } => {
                (adapter_id, VirtualLampChangedEvent { adapter_id, virtual_lamp_id }.into())
            }
            Self::InputDevice { adapter_id, short_address } => {
                (adapter_id, InputDeviceChangedEvent { adapter_id, short_address }.into())
            }
            _ => return None,
        })
    }
}

fn schedule_frame(store: &RegistryStore, target: u16, schedule_id: &FixedText32) -> BusFrame {
    let enabled = store.read_inner().hcl_schedules.get(schedule_id).map(|row| row.enabled);
    let (removed, enabled) = (enabled.is_none(), enabled.unwrap_or(false));
    let payload = HclScheduleChangedEvent { schedule_id: schedule_id.clone(), removed, enabled };
    registry_event_frame(CORRELATION_NONE, target, payload)
}

fn setting_frame(store: &RegistryStore, target: u16, setting: Setting) -> BusFrame {
    let inner = store.read_inner();
    let payload: BusEventPayload = match setting {
        Setting::Poller => poller_settings_changed(&inner.poller_settings).into(),
        Setting::Dali => {
            dali_settings_changed(&inner.dali_settings, APPLICATION_ACTIVE_UNMOVED).into()
        }
        Setting::Redundancy => redundancy_settings_changed(&inner.redundancy_settings).into(),
        Setting::Policies => policies_changed(&inner.policies).into(),
        Setting::HomeAssistant => {
            let enabled = inner.home_assistant_settings.enabled;
            HomeAssistantSettingsChangedEvent { enabled }.into()
        }
    };
    drop(inner);
    registry_event_frame(CORRELATION_NONE, target, payload)
}

#[cfg(test)]
impl RegistryStore {
    pub(crate) fn announcements_queued(&self) -> usize {
        self.announcements.queue().pending.len()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AnnouncementPacing {
    pub(crate) per_window: usize,
    pub(crate) window: Duration,
}

#[derive(Default)]
struct AnnouncementQueue {
    pending: BTreeSet<Announcement>,
    window_started: Option<Instant>,
    announced_in_window: usize,
    refused_in_window: bool,
    refusing: bool,
}

impl AnnouncementQueue {
    fn budget(&mut self, pacing: AnnouncementPacing) -> usize {
        let now = Instant::now();
        if self.window_started.is_none_or(|started| now.duration_since(started) >= pacing.window) {
            self.window_started = Some(now);
            self.announced_in_window = 0;
            self.refused_in_window = false;
        }
        if self.refused_in_window {
            return 0;
        }
        pacing.per_window.saturating_sub(self.announced_in_window)
    }

    fn refused(&mut self, announcement: Announcement) {
        self.pending.insert(announcement);
        self.refused_in_window = true;
        if !std::mem::replace(&mut self.refusing, true) {
            let waiting = self.pending.len();
            warn!("registry: events ingress full; {waiting} announcements wait for a later window");
        }
    }

    fn sent(&mut self, announced: usize) {
        self.announced_in_window += announced;
        if announced > 0 && !self.refused_in_window {
            self.refusing = false;
        }
    }
}

#[derive(Default)]
pub(crate) struct PendingAnnouncements(Mutex<AnnouncementQueue>);

impl PendingAnnouncements {
    fn queue(&self) -> MutexGuard<'_, AnnouncementQueue> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RegistryStore {
    #[inline(never)]
    pub(crate) fn queue_announcements(&self, replaced: ReplacedEntities) {
        replaced.queue_into(&mut self.announcements.queue().pending);
    }

    #[inline(never)]
    pub(crate) fn announce_queued(
        &self,
        publisher: &BusPublisher,
        target: u16,
        pacing: AnnouncementPacing,
    ) -> usize {
        let budget = self.announcements.queue().budget(pacing);
        let mut announced = 0;
        while announced < budget && self.announce_next(publisher, target) {
            announced += 1;
        }
        self.announcements.queue().sent(announced);
        announced
    }

    fn announce_next(&self, publisher: &BusPublisher, target: u16) -> bool {
        let Some(next) = self.announcements.queue().pending.pop_first() else {
            return false;
        };
        let Some(frame) = next.frame(self, target) else {
            return true;
        };
        let result = publisher.try_publish(BusChannel::Events, frame);
        let refused = result == PublishResult::DroppedIngressFull;
        if refused {
            self.announcements.queue().refused(next);
        }
        !refused
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
        let waiting = waiting_keys(held, adapter_count);
        let inner = self.read_inner();
        let keys = written.iter().chain(&waiting);
        let before = keys.map(|key| (*key, entities_of(&inner, *key))).collect();
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

#[inline(never)]
fn waiting_keys(held: HeldSlices, adapter_count: u8) -> Vec<SliceKey> {
    let waiting = held.keys(adapter_count);
    waiting.filter(|key| !matches!(key, SliceKey::PhysicalDevices { .. })).collect()
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
    use std::time::{Duration, Instant};

    use dali2rust_bus::{BusConfig, BusFrame, BusHost, BusPublisher};
    use dali2rust_contracts::msg::{BusEventPayload, EVENT_VARIANT_NAMES};
    use dali2rust_test_support::{hold_the_events_ingress_full, wait_until};

    use super::{AnnouncementPacing, ReplacedEntities, POLLER_SETTINGS};
    use crate::runtime::registry::RegistryStore;
    use crate::runtime::registry_worker::ANNOUNCEMENT_PACING;

    const INBOX: usize = 128;
    const QUEUED_LAMPS: u8 = 40;
    const REFUSED_LAMPS: u8 = 3;
    const LATER_INTERVAL_MS: u32 = 4_321;
    const WAIT: Duration = Duration::from_secs(2);
    const TARGET: u16 = 1;
    const EVERY_CALL: AnnouncementPacing =
        AnnouncementPacing { window: Duration::ZERO, ..ANNOUNCEMENT_PACING };

    fn observed() -> (BusHost, BusPublisher, Receiver<BusFrame>) {
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_events(INBOX, EVENT_VARIANT_NAMES))
    }

    fn first_heard<T>(
        events: &Receiver<BusFrame>,
        pick: impl Fn(&BusEventPayload) -> Option<T>,
    ) -> Vec<T> {
        let deadline = Instant::now() + WAIT;
        let mut picked = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Ok(BusFrame::Event(event)) = events.recv_timeout(left) else {
                break;
            };
            picked.extend(pick(&event.payload));
            if !picked.is_empty() {
                break;
            }
        }
        picked
    }

    fn lamps_queued(store: &RegistryStore, count: u8) {
        let lamps = (0..count).fold(0u64, |mask, id| mask | (1 << id));
        let lamps = BTreeMap::from([(0, lamps)]);
        store.queue_announcements(ReplacedEntities { lamps, ..ReplacedEntities::default() });
    }

    fn lamp_id(payload: &BusEventPayload) -> Option<u8> {
        match payload {
            BusEventPayload::VirtualLampChangedEvent(ev) => Some(ev.virtual_lamp_id),
            _ => None,
        }
    }

    #[test]
    fn an_announcement_a_full_ingress_refuses_waits_in_the_queue_and_goes_out_later() {
        let (_host, publisher, events) = observed();
        let store = RegistryStore::with_adapter_count(1);
        lamps_queued(&store, REFUSED_LAMPS);
        let held = hold_the_events_ingress_full(&publisher);
        let announced = store.announce_queued(&publisher, TARGET, EVERY_CALL);
        assert_eq!(announced, 0, "a refusal counted as sent");
        let queued = store.announcements_queued();
        assert_eq!(queued, usize::from(REFUSED_LAMPS), "a refusal left the queue");
        held.release();
        let mut sent = 0;
        let mut announce = || sent += store.announce_queued(&publisher, TARGET, EVERY_CALL);
        wait_until(|| { announce(); store.announcements_queued() == 0 }, WAIT);
        assert_eq!(sent, usize::from(REFUSED_LAMPS), "each refused lamp went out once");
        let first = first_heard(&events, lamp_id);
        assert_eq!(first, vec![0], "the lamps never reached a subscriber");
    }

    const TURNS_IN_THE_WINDOW: usize = 5;
    const LONG_WINDOW: Duration = Duration::from_secs(3600);

    #[test]
    fn a_refused_announcement_waits_for_the_next_window_however_many_turns_come_first() {
        let (_host, publisher, _events) = observed();
        let store = RegistryStore::with_adapter_count(1);
        lamps_queued(&store, REFUSED_LAMPS);
        let held = hold_the_events_ingress_full(&publisher);
        let one_window = AnnouncementPacing { window: LONG_WINDOW, ..ANNOUNCEMENT_PACING };
        assert_eq!(store.announce_queued(&publisher, TARGET, one_window), 0);
        let attempts = || publisher.counters_snapshot().events.publish_attempted;
        let refused_at = attempts();
        for _ in 0..TURNS_IN_THE_WINDOW {
            store.announce_queued(&publisher, TARGET, one_window);
        }
        assert_eq!(attempts(), refused_at, "a turn inside the window retried the refusal");
        assert_eq!(store.announcements_queued(), usize::from(REFUSED_LAMPS));
        drop(held);
    }

    #[test]
    fn a_settings_record_goes_out_first_ahead_of_the_lamps_queued_with_it() {
        let (_host, publisher, events) = observed();
        let store = RegistryStore::with_adapter_count(1);
        lamps_queued(&store, QUEUED_LAMPS);
        let poller_settings = ReplacedEntities { records: POLLER_SETTINGS, ..Default::default() };
        store.queue_announcements(poller_settings);
        store.announce_queued(&publisher, TARGET, EVERY_CALL);
        let first = first_heard(&events, |payload| Some(payload.variant_name()));
        let settings = vec!["PollerSettingsChangedEvent"];
        assert_eq!(first, settings, "the settings waited behind the lamps");
    }

    #[test]
    fn a_record_queued_twice_goes_out_once_with_the_state_it_has_then() {
        let (_host, publisher, events) = observed();
        let store = RegistryStore::with_adapter_count(1);
        let poller = || ReplacedEntities { records: POLLER_SETTINGS, ..Default::default() };
        store.queue_announcements(poller());
        store.queue_announcements(poller());
        assert_eq!(store.announcements_queued(), 1, "the record was queued twice");
        store.write_inner().poller_settings.interval_ms = LATER_INTERVAL_MS;
        assert_eq!(store.announce_queued(&publisher, TARGET, EVERY_CALL), 1);
        let interval = |payload: &BusEventPayload| match payload {
            BusEventPayload::PollerSettingsChangedEvent(ev) => Some(ev.interval_ms),
            _ => None,
        };
        assert_eq!(first_heard(&events, interval), vec![LATER_INTERVAL_MS], "read when queued");
    }

    #[test]
    fn a_window_announces_no_more_than_its_budget_and_the_rest_stay_queued() {
        let (_host, publisher, _events) = observed();
        let store = RegistryStore::with_adapter_count(1);
        lamps_queued(&store, QUEUED_LAMPS);
        let budget = ANNOUNCEMENT_PACING.per_window;
        let mut queued = vec![store.announcements_queued()];
        while let Some(left) = queued.last().copied().filter(|left| *left > 0) {
            let announced = store.announce_queued(&publisher, TARGET, EVERY_CALL);
            assert_eq!(announced, budget.min(left), "a window spent more or less than its budget");
            queued.push(store.announcements_queued());
        }
        let expected: Vec<usize> = (0..=usize::from(QUEUED_LAMPS).div_ceil(budget))
            .map(|window| usize::from(QUEUED_LAMPS).saturating_sub(window * budget))
            .collect();
        assert_eq!(queued, expected, "a window spent more or less than its budget");
    }
}
