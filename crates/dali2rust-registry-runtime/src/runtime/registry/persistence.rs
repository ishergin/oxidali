use std::sync::atomic::Ordering;

use dali2rust_contracts::msg::{
    fixed_text_64, PersistenceSliceError, PersistenceSliceErrorList, PersistenceSliceKind,
    PersistenceSliceList,
};
use dali2rust_domain::registry::MemoryBankSummaryView;
use dali2rust_platform::slice_store::{SliceKey, SliceStore, StoreError};
use log::{info, warn};

use crate::runtime::registry::input_devices::PersistableInputDevicesSlice;

use super::groups::{GroupMembershipRowRecord, GroupRecord};
use super::persistence_slices::{
    PersistableAdapterSlice, PersistableGroupsSlice, PersistableHclSchedulesSlice,
    PersistableHomeAssistantSettingsSlice, PersistablePhysicalDevicesSlice,
    PersistableDaliSettingsSlice, PersistablePollerSettingsSlice,
    PersistablePoliciesSlice, PersistableRedundancySettingsSlice, PersistableSceneSlice,
    PersistableVirtualLampsSlice,
    PersistenceEnvelope, decode_versioned_slice, ADAPTERS_SLICE_VERSION, GROUPS_SLICE_VERSION,
    HCL_SCHEDULES_SLICE_VERSION, HOME_ASSISTANT_SETTINGS_SLICE_VERSION,
    DALI_SETTINGS_SLICE_VERSION, INPUT_DEVICES_SLICE_VERSION, PHYSICAL_DEVICES_SLICE_VERSION,
    POLLER_SETTINGS_SLICE_VERSION,
    POLICIES_SLICE_VERSION,
    REDUNDANCY_SETTINGS_SLICE_VERSION,
    SCENES_SLICE_VERSION,
    VIRTUAL_LAMPS_SLICE_VERSION,
};
use super::physical_device_banks::{bank_bit, banks_of, short_bit, BankOutcomes, SlotOutcome};
use super::persistence_stream::{
    write_persistence_streaming, AdaptersSliceStream, GroupsSliceStream,
    PhysicalDevicesSliceStream, SceneSliceStream, VirtualLampsSliceStream,
};
use super::scenes::{SceneDesiredRowRecord, SceneRecord, SCENE_COUNT};
use super::physical_devices::{MemoryBankRangeRecord, MemoryBankRecord, PhysicalDeviceRecord};
use super::store::{Inner, RegistryStore};

pub struct PersistenceHydrateReport {
    pub loaded_slices: PersistenceSliceList,
    pub default_slices: PersistenceSliceList,
    pub errors: PersistenceSliceErrorList,
}

impl PersistenceHydrateReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

impl crate::runtime::registry::store::RegistryStore {
    pub fn hydrate_from_store(
        &self,
        slices: &dyn SliceStore,
        adapter_count: u8,
    ) -> PersistenceHydrateReport {
        let mut loaded_slices = PersistenceSliceList::new();
        let mut default_slices = PersistenceSliceList::new();
        let mut errors = PersistenceSliceErrorList::new();
        let mut sink = HydrateSink::Collecting {
            loaded: &mut loaded_slices,
            defaults: &mut default_slices,
            errors: &mut errors,
        };
        self.hydrate_all(slices, adapter_count, &mut sink);
        PersistenceHydrateReport {
            loaded_slices,
            default_slices,
            errors,
        }
    }

    pub fn hydrate_counts_from_store(
        &self,
        slices: &dyn SliceStore,
        adapter_count: u8,
    ) -> HydrateCounts {
        let mut counts = HydrateCounts::default();
        let mut sink = HydrateSink::Counting(&mut counts);
        self.hydrate_all(slices, adapter_count, &mut sink);
        counts
    }

    fn hydrate_physical_devices_of(
        &self,
        slices: &dyn SliceStore,
        adapter_id: u8,
        sink: &mut HydrateSink<'_>,
    ) {
        let banks = self.hydrate_physical_device_banks(slices, adapter_id, sink);
        let old_slot = (banks.open() != 0)
            .then(|| self.fall_back_to_whole_adapter_slot(slices, adapter_id, banks.open(), sink));
        let decision = banks.decide(old_slot);
        self.dirty.mark_physical_device_banks_dirty(adapter_id, decision.rewrite);
        self.dirty
            .settle_physical_device_banks(adapter_id, decision.settled, decision.waiting);
    }

    fn hydrate_physical_device_banks(
        &self,
        slices: &dyn SliceStore,
        adapter_id: u8,
        sink: &mut HydrateSink<'_>,
    ) -> BankOutcomes {
        let mut banks = BankOutcomes::default();
        for bank in 0..PHYSICAL_DEVICE_BANKS_U8 {
            banks.note(bank, hydrate_pd_bank_slice(self, slices, adapter_id, bank, sink));
        }
        banks
    }

    fn fall_back_to_whole_adapter_slot(
        &self,
        slices: &dyn SliceStore,
        adapter_id: u8,
        open: u16,
        sink: &mut HydrateSink<'_>,
    ) -> SlotOutcome {
        let held = self.shorts_held_in_ram(adapter_id);
        let live = banks_of(held) | self.dirty.pending_physical_device_banks(adapter_id);
        let takeable = open & (self.dirty.withheld_physical_device_banks(adapter_id) | !live);
        let stored = slices.load(SliceKey::PhysicalDevices { adapter_id });
        let outcome = hydrate_pd_slice(self, stored, adapter_id, takeable, held, sink);
        let taken = (self.shorts_held_in_ram(adapter_id) & !held).count_ones();
        if taken > 0 {
            info!(
                "persistence: PD a{adapter_id} took {taken} devices from the whole-adapter \
                 slice for banks {takeable:#06x} that did not load; the next flush writes them"
            );
        }
        outcome
    }

    fn shorts_held_in_ram(&self, adapter_id: u8) -> u64 {
        self.read_inner()
            .physical_devices
            .keys()
            .filter(|(aid, _)| *aid == adapter_id)
            .fold(0, |shorts, (_, short_address)| shorts | short_bit(*short_address))
    }

    fn hydrate_all(&self, slices: &dyn SliceStore, adapter_count: u8, sink: &mut HydrateSink<'_>) {
        hydrate_adapters_slice(self, slices, sink);
        for adapter_id in 0..adapter_count {
            self.hydrate_adapter(slices, adapter_id, sink);
        }
        hydrate_input_device_banks(self, slices, sink);
        hydrate_hcl_schedules_slice(self, slices, sink);
        hydrate_poller_settings_slice(self, slices, sink);
        hydrate_dali_settings_slice(self, slices, sink);
        hydrate_redundancy_settings_slice(self, slices, sink);
        hydrate_policies_slice(self, slices, sink);
        hydrate_home_assistant_settings_slice(self, slices, sink);
        self.persist_counters
            .unread_slices
            .store(self.unread_slices(adapter_count), Ordering::Relaxed);
    }

    fn hydrate_adapter(&self, slices: &dyn SliceStore, adapter_id: u8, sink: &mut HydrateSink<'_>) {
        hydrate_groups_slice(self, slices, adapter_id, sink);
        self.hydrate_physical_devices_of(slices, adapter_id, sink);
        hydrate_vl_slice(self, slices, adapter_id, sink);
        for scene_id in 0..SCENE_COUNT {
            hydrate_scene_slice(self, slices, adapter_id, scene_id, sink);
        }
    }

    pub(crate) fn unread_slices(&self, adapter_count: u8) -> u32 {
        let banks: u32 = (0..adapter_count)
            .map(|adapter_id| self.dirty.withheld_physical_device_banks(adapter_id).count_ones())
            .sum();
        banks + self.dirty.withheld.count()
    }

    pub fn flush_dirty_slices(&self, slices: &dyn SliceStore) {
        if !self.dirty.any_dirty() {
            return;
        }
        self.note_held_back_slices();
        self.flush_adapters_if_dirty(slices);
        self.flush_masked_slice(
            slices,
            self.dirty.take_groups_dirty(),
            "groups",
            Self::flush_groups,
            |s, id| s.dirty.mark_groups_dirty(id),
        );
        self.flush_masked_slice(
            slices,
            self.dirty.take_virtual_lamps_dirty(),
            "VL",
            Self::flush_virtual_lamps,
            |s, id| s.dirty.mark_virtual_lamps_dirty(id),
        );
        self.flush_physical_device_banks(slices);
        self.flush_scenes_if_dirty(slices);
        self.flush_hcl_schedules_if_dirty(slices);
        self.flush_poller_settings_if_dirty(slices);
        self.flush_dali_settings_if_dirty(slices);
        self.flush_redundancy_settings_if_dirty(slices);
        self.flush_policies_if_dirty(slices);
        self.flush_home_assistant_settings_if_dirty(slices);
        self.flush_input_devices_if_dirty(slices);
    }

    fn note_flush_failure(&self, what: core::fmt::Arguments<'_>, e: &StoreError) {
        if matches!(e, StoreError::Deferred) {
            return;
        }
        warn!("persistence: {what} flush failed: {e}");
        self.persist_counters
            .flush_error_total
            .fetch_add(1, Ordering::Relaxed);
    }

    fn flush_physical_device_banks(&self, slices: &dyn SliceStore) {
        let _ = self.dirty.take_physical_devices_dirty();
        let g = self.read_inner();
        let adapter_count = g.adapters.len() as u8;
        drop(g);
        for adapter_id in 0..adapter_count {
            self.note_held_back_banks(adapter_id);
            let mask = self.dirty.take_physical_device_banks_dirty(adapter_id);
            if mask == 0 {
                continue;
            }
            for bank in 0..dali2rust_platform::slice_store::SliceKey::PHYSICAL_DEVICE_BANKS {
                if mask & (1u16 << bank) == 0 {
                    continue;
                }
                if let Err(e) = self.flush_physical_device_bank(slices, adapter_id, bank) {
                    self.note_flush_failure(format_args!("PD a{adapter_id}/b{bank}"), &e);
                    self.dirty.mark_physical_device_dirty(
                        adapter_id,
                        bank * dali2rust_platform::slice_store::SliceKey::DEVICES_PER_BANK,
                    );
                } else {
                    self.persist_counters
                        .flush_success_total
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    fn note_held_back_slices(&self) {
        let adapter_count = self.read_inner().adapters.len() as u8;
        for key in self.dirty.held_back(adapter_count) {
            warn!(
                "persistence: {} waits for its stored copy; its changes are not saved until a \
                 boot or reload can read it",
                key.label()
            );
        }
    }

    fn note_held_back_banks(&self, adapter_id: u8) {
        let held = self.dirty.pending_physical_device_banks(adapter_id)
            & self.dirty.withheld_physical_device_banks(adapter_id);
        if held != 0 {
            log::warn!(
                "persistence: PD a{adapter_id} banks {held:#06x} wait for their stored copy; \
                 their changes are not saved until a boot or reload can read it"
            );
        }
    }

    fn flush_global_slice_if_dirty<T: serde::Serialize>(
        &self,
        slices: &dyn SliceStore,
        taken: bool,
        remark: impl FnOnce(),
        key: SliceKey,
        version: u32,
        name: &str,
        snapshot: impl FnOnce(&Inner) -> T,
    ) {
        if !taken {
            return;
        }
        if let Err(e) = self.flush_global_slice(slices, key, version, name, snapshot) {
            self.note_flush_failure(format_args!("{name}"), &e);
            remark();
            return;
        }
        self.persist_counters
            .flush_success_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn flush_global_slice<T: serde::Serialize>(
        &self,
        slices: &dyn SliceStore,
        key: SliceKey,
        version: u32,
        name: &str,
        snapshot: impl FnOnce(&Inner) -> T,
    ) -> Result<(), StoreError> {
        let snapshot = {
            let g = self.read_inner();
            snapshot(&g)
        };
        let mut buf = self.flush_buf.lock().expect("flush buf lock");
        let envelope = PersistenceEnvelope::new(version, snapshot);
        write_persistence_streaming(slices, key, &mut buf, &envelope, name)
    }

    fn flush_hcl_schedules_if_dirty(&self, slices: &dyn SliceStore) {
        let taken = self.dirty.take_hcl_schedules_dirty();
        if taken {
            self.dirty.take_hcl_switches_waiting();
        }
        self.flush_global_slice_if_dirty(
            slices,
            taken,
            || self.dirty.mark_hcl_schedules_dirty(),
            SliceKey::HclSchedules,
            HCL_SCHEDULES_SLICE_VERSION,
            "hcl_schedules",
            crate::runtime::registry::hcl_schedules::persistable_snapshot,
        );
    }

    fn flush_input_devices_if_dirty(&self, slices: &dyn SliceStore) {
        if !self.dirty.take_input_devices_dirty() {
            return;
        }
        for bank in 0..INPUT_DEVICE_BANKS_U8 {
            let name = format!("input_devices_b{bank}");
            if let Err(e) = self.flush_global_slice(
                slices,
                SliceKey::InputDevices { bank },
                INPUT_DEVICES_SLICE_VERSION,
                &name,
                |inner| {
                    crate::runtime::registry::input_devices::persistable_snapshot_bank(inner, bank)
                },
            ) {
                self.note_flush_failure(format_args!("{name}"), &e);
                self.dirty.mark_input_devices_dirty();
                return;
            }
            self.persist_counters
                .flush_success_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn flush_poller_settings_if_dirty(&self, slices: &dyn SliceStore) {
        self.flush_global_slice_if_dirty(
            slices,
            self.dirty.take_poller_settings_dirty(),
            || self.dirty.mark_poller_settings_dirty(),
            SliceKey::PollerSettings,
            POLLER_SETTINGS_SLICE_VERSION,
            "poller_settings",
            crate::runtime::registry::poller_settings::persistable_snapshot,
        );
    }

    fn flush_dali_settings_if_dirty(&self, slices: &dyn SliceStore) {
        self.flush_global_slice_if_dirty(
            slices,
            self.dirty.take_dali_settings_dirty(),
            || self.dirty.mark_dali_settings_dirty(),
            SliceKey::DaliSettings,
            DALI_SETTINGS_SLICE_VERSION,
            "dali_settings",
            crate::runtime::registry::dali_settings::persistable_snapshot,
        );
    }

    fn flush_redundancy_settings_if_dirty(&self, slices: &dyn SliceStore) {
        self.flush_global_slice_if_dirty(
            slices,
            self.dirty.take_redundancy_settings_dirty(),
            || self.dirty.mark_redundancy_settings_dirty(),
            SliceKey::RedundancySettings,
            REDUNDANCY_SETTINGS_SLICE_VERSION,
            "redundancy_settings",
            crate::runtime::registry::redundancy_settings::persistable_snapshot,
        );
    }

    fn flush_policies_if_dirty(&self, slices: &dyn SliceStore) {
        self.flush_global_slice_if_dirty(
            slices,
            self.dirty.take_policies_dirty(),
            || self.dirty.mark_policies_dirty(),
            SliceKey::Policies,
            POLICIES_SLICE_VERSION,
            "policies",
            crate::runtime::registry::policies::persistable_snapshot,
        );
    }

    fn flush_home_assistant_settings_if_dirty(&self, slices: &dyn SliceStore) {
        self.flush_global_slice_if_dirty(
            slices,
            self.dirty.take_home_assistant_settings_dirty(),
            || self.dirty.mark_home_assistant_settings_dirty(),
            SliceKey::HomeAssistantSettings,
            HOME_ASSISTANT_SETTINGS_SLICE_VERSION,
            "home_assistant_settings",
            crate::runtime::registry::home_assistant_settings::persistable_snapshot,
        );
    }

    fn flush_scenes_if_dirty(&self, slices: &dyn SliceStore) {
        let g = self.read_inner();
        let adapter_count = g.adapters.len() as u8;
        drop(g);
        for adapter_id in 0..adapter_count {
            let dirty_mask = self.dirty.take_scenes_dirty(adapter_id);
            for scene_id in 0..SCENE_COUNT {
                if dirty_mask & (1u16 << scene_id) == 0 {
                    continue;
                }
                if let Err(e) = self.flush_scene(slices, adapter_id, scene_id) {
                    self.note_flush_failure(format_args!("scene a{adapter_id}/s{scene_id}"), &e);
                    self.dirty.mark_scene_dirty(adapter_id, scene_id);
                } else {
                    self.persist_counters
                        .flush_success_total
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    fn flush_adapters_if_dirty(&self, slices: &dyn SliceStore) {
        if !self.dirty.adapters_writable() {
            return;
        }
        if let Err(e) = self.flush_adapters(slices) {
            self.note_flush_failure(format_args!("adapters"), &e);
        } else {
            self.dirty.adapters.store(false, Ordering::Release);
            self.persist_counters
                .flush_success_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush_masked_slice(
        &self,
        slices: &dyn SliceStore,
        dirty_mask: u32,
        label: &str,
        flush_one: fn(&Self, &dyn SliceStore, u8) -> Result<(), StoreError>,
        remark_dirty: fn(&Self, u8),
    ) {
        if dirty_mask == 0 {
            return;
        }
        let g = self.read_inner();
        let adapter_count = g.adapters.len() as u8;
        drop(g);
        for adapter_id in 0..adapter_count {
            if dirty_mask & (1u32 << adapter_id) == 0 {
                continue;
            }
            if let Err(e) = flush_one(self, slices, adapter_id) {
                self.note_flush_failure(format_args!("{label} a{adapter_id}"), &e);
                remark_dirty(self, adapter_id);
            } else {
                self.persist_counters
                    .flush_success_total
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn flush_adapters(
        &self,
        slices: &dyn SliceStore,
    ) -> Result<(), StoreError> {
        let g = self.read_inner();
        let mut buf = self.flush_buf.lock().expect("flush buf lock");
        let envelope = PersistenceEnvelope::new(ADAPTERS_SLICE_VERSION, AdaptersSliceStream { inner: &g });
        write_persistence_streaming(slices, SliceKey::Adapters, &mut buf, &envelope, "adapters")
    }

    fn flush_virtual_lamps(
        &self,
        slices: &dyn SliceStore,
        adapter_id: u8,
    ) -> Result<(), StoreError> {
        let g = self.read_inner();
        let mut buf = self.flush_buf.lock().expect("flush buf lock");
        let envelope = PersistenceEnvelope::new(VIRTUAL_LAMPS_SLICE_VERSION, VirtualLampsSliceStream {
            inner: &g,
            adapter_id,
        });
        write_persistence_streaming(
            slices,
            SliceKey::VirtualLamps { adapter_id },
            &mut buf,
            &envelope,
            "virtual_lamps",
        )
    }

    fn flush_groups(
        &self,
        slices: &dyn SliceStore,
        adapter_id: u8,
    ) -> Result<(), StoreError> {
        let g = self.read_inner();
        let mut buf = self.flush_buf.lock().expect("flush buf lock");
        let envelope = PersistenceEnvelope::new(GROUPS_SLICE_VERSION, GroupsSliceStream {
            inner: &g,
            adapter_id,
        });
        write_persistence_streaming(slices, SliceKey::Groups { adapter_id }, &mut buf, &envelope, "groups")
    }

    fn flush_scene(
        &self,
        slices: &dyn SliceStore,
        adapter_id: u8,
        scene_id: u8,
    ) -> Result<(), StoreError> {
        let g = self.read_inner();
        let mut buf = self.flush_buf.lock().expect("flush buf lock");
        let envelope = PersistenceEnvelope::new(SCENES_SLICE_VERSION, SceneSliceStream {
            inner: &g,
            adapter_id,
            scene_id,
        });
        write_persistence_streaming(
            slices,
            SliceKey::Scene { adapter_id, scene_id },
            &mut buf,
            &envelope,
            "scene",
        )
    }

    fn flush_physical_device_bank(
        &self,
        slices: &dyn SliceStore,
        adapter_id: u8,
        bank: u8,
    ) -> Result<(), StoreError> {
        let g = self.read_inner();
        let mut buf = self.flush_buf.lock().expect("flush buf lock");
        let envelope = PersistenceEnvelope::new(
            PHYSICAL_DEVICES_SLICE_VERSION,
            PhysicalDevicesSliceStream {
                inner: &g,
                adapter_id,
                bank: Some(bank),
            },
        );
        write_persistence_streaming(
            slices,
            SliceKey::PhysicalDeviceBank { adapter_id, bank },
            &mut buf,
            &envelope,
            "physical_device_bank",
        )
    }


    fn decode_physical_devices(
        bytes: &[u8],
    ) -> Result<PersistablePhysicalDevicesSlice, StoreError> {
        decode_versioned_slice::<PersistablePhysicalDevicesSlice>(bytes, PHYSICAL_DEVICES_SLICE_VERSION)
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    fn decode_physical_device_bank(
        bytes: &[u8],
        adapter_id: u8,
        bank: u8,
    ) -> Result<PersistablePhysicalDevicesSlice, StoreError> {
        let slice = Self::decode_physical_devices(bytes)?;
        if let Some(stray) = slice
            .devices
            .iter()
            .find(|d| d.short_address / SliceKey::DEVICES_PER_BANK != bank)
        {
            return Err(StoreError::Backend(format!(
                "a{adapter_id}/b{bank} holds short {} - bank geometry moved under \
                 the stored bytes; the bank is not loaded",
                stray.short_address
            )));
        }
        Ok(slice)
    }
}

pub(crate) fn validate_registry_slice(key: SliceKey, bytes: &[u8]) -> Option<Result<(), StoreError>> {
    let checked = match key {
        SliceKey::Adapters => decode_slice::<PersistableAdapterSlice>(bytes, ADAPTERS_SLICE_VERSION).map(drop),
        SliceKey::Groups { .. } => decode_slice::<PersistableGroupsSlice>(bytes, GROUPS_SLICE_VERSION).map(drop),
        SliceKey::VirtualLamps { .. } => {
            decode_slice::<PersistableVirtualLampsSlice>(bytes, VIRTUAL_LAMPS_SLICE_VERSION).map(drop)
        }
        SliceKey::Scene { .. } => decode_slice::<PersistableSceneSlice>(bytes, SCENES_SLICE_VERSION).map(drop),
        SliceKey::PhysicalDevices { .. } => RegistryStore::decode_physical_devices(bytes).map(drop),
        SliceKey::PhysicalDeviceBank { adapter_id, bank } => {
            RegistryStore::decode_physical_device_bank(bytes, adapter_id, bank).map(drop)
        }
        SliceKey::InputDevices { .. } => {
            decode_slice::<PersistableInputDevicesSlice>(bytes, INPUT_DEVICES_SLICE_VERSION).map(drop)
        }
        _ => return validate_registry_setting(key, bytes),
    };
    Some(checked)
}

fn validate_registry_setting(key: SliceKey, bytes: &[u8]) -> Option<Result<(), StoreError>> {
    let checked = match key {
        SliceKey::HclSchedules => {
            decode_slice::<PersistableHclSchedulesSlice>(bytes, HCL_SCHEDULES_SLICE_VERSION).map(drop)
        }
        SliceKey::PollerSettings => {
            decode_slice::<PersistablePollerSettingsSlice>(bytes, POLLER_SETTINGS_SLICE_VERSION).map(drop)
        }
        SliceKey::DaliSettings => {
            decode_slice::<PersistableDaliSettingsSlice>(bytes, DALI_SETTINGS_SLICE_VERSION).map(drop)
        }
        SliceKey::RedundancySettings => decode_slice::<PersistableRedundancySettingsSlice>(
            bytes,
            REDUNDANCY_SETTINGS_SLICE_VERSION,
        )
        .map(drop),
        SliceKey::Policies => decode_slice::<PersistablePoliciesSlice>(bytes, POLICIES_SLICE_VERSION).map(drop),
        SliceKey::HomeAssistantSettings => decode_slice::<PersistableHomeAssistantSettingsSlice>(
            bytes,
            HOME_ASSISTANT_SETTINGS_SLICE_VERSION,
        )
        .map(drop),
        _ => return None,
    };
    Some(checked)
}

enum HydrateSink<'a> {
    Collecting {
        loaded: &'a mut PersistenceSliceList,
        defaults: &'a mut PersistenceSliceList,
        errors: &'a mut PersistenceSliceErrorList,
    },
    Counting(&'a mut HydrateCounts),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HydrateCounts {
    pub loaded: u16,
    pub defaulted: u16,
    pub errors: u16,
}

impl HydrateSink<'_> {
    fn note_loaded(&mut self, kind: PersistenceSliceKind) {
        match self {
            Self::Collecting { loaded, .. } => push_slice(loaded, kind),
            Self::Counting(counts) => counts.loaded = counts.loaded.saturating_add(1),
        }
    }

    fn note_defaulted(&mut self, kind: PersistenceSliceKind) {
        match self {
            Self::Collecting { defaults, .. } => push_slice(defaults, kind),
            Self::Counting(counts) => counts.defaulted = counts.defaulted.saturating_add(1),
        }
    }

    fn note_failed(&mut self, kind: &PersistenceSliceKind, error: impl FnOnce() -> String) {
        match self {
            Self::Collecting { errors, .. } => push_error(errors, kind, &error()),
            Self::Counting(counts) => counts.errors = counts.errors.saturating_add(1),
        }
    }
}

enum HydrateOutcome {
    Loaded,
    Defaulted,
    Failed,
}

fn hydrate_stored_slice<S>(
    store: &RegistryStore,
    slices: &dyn SliceStore,
    kind: PersistenceSliceKind,
    sink: &mut HydrateSink<'_>,
    decode: impl FnOnce(&[u8]) -> Result<S, StoreError>,
    apply: impl FnOnce(&mut Inner, &S),
) -> SlotOutcome {
    let key = key_of(&kind);
    let stored = slices.load(key);
    let outcome = hydrate_read_slot(store, kind.clone(), &key.label(), sink, stored, decode, apply);
    if outcome == SlotOutcome::Rejected {
        mark_slice_dirty(store, &kind);
    }
    store.dirty.withheld.settle(key, outcome == SlotOutcome::Unread);
    outcome
}

fn decode_slice<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    version: u32,
) -> Result<T, StoreError> {
    decode_versioned_slice::<T>(bytes, version).map_err(|e| StoreError::Backend(e.to_string()))
}

fn key_of(kind: &PersistenceSliceKind) -> SliceKey {
    match *kind {
        PersistenceSliceKind::Adapters => SliceKey::Adapters,
        PersistenceSliceKind::Groups { adapter_id } => SliceKey::Groups { adapter_id },
        PersistenceSliceKind::VirtualLamps { adapter_id } => SliceKey::VirtualLamps { adapter_id },
        PersistenceSliceKind::PhysicalDevices { adapter_id } => {
            SliceKey::PhysicalDevices { adapter_id }
        }
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id, bank } => {
            SliceKey::PhysicalDeviceBank { adapter_id, bank }
        }
        PersistenceSliceKind::Scenes { adapter_id, scene_id } => {
            SliceKey::Scene { adapter_id, scene_id }
        }
        PersistenceSliceKind::HclSchedules => SliceKey::HclSchedules,
        PersistenceSliceKind::PollerSettings => SliceKey::PollerSettings,
        PersistenceSliceKind::HomeAssistantSettings => SliceKey::HomeAssistantSettings,
        PersistenceSliceKind::DaliSettings => SliceKey::DaliSettings,
        PersistenceSliceKind::RedundancySettings => SliceKey::RedundancySettings,
        PersistenceSliceKind::Policies => SliceKey::Policies,
        PersistenceSliceKind::InputDevices { bank } => SliceKey::InputDevices { bank },
    }
}

fn hydrate_slice_unmarked<S>(
    store: &RegistryStore,
    kind: PersistenceSliceKind,
    label: &str,
    sink: &mut HydrateSink<'_>,
    load: impl FnOnce() -> Result<S, StoreError>,
    apply: impl FnOnce(&mut Inner, &S),
) -> HydrateOutcome {
    match load() {
        Ok(slice) => apply_loaded_slice(store, kind, sink, |inner| apply(inner, &slice)),
        Err(StoreError::Missing) => {
            note_missing_slice(store, kind, sink);
            HydrateOutcome::Defaulted
        }
        Err(e) => {
            note_unread_slice(store, kind, label, sink, &e);
            HydrateOutcome::Failed
        }
    }
}

fn hydrate_read_slot<S>(
    store: &RegistryStore,
    kind: PersistenceSliceKind,
    label: &str,
    sink: &mut HydrateSink<'_>,
    stored: Result<Vec<u8>, StoreError>,
    decode: impl FnOnce(&[u8]) -> Result<S, StoreError>,
    apply: impl FnOnce(&mut Inner, &S),
) -> SlotOutcome {
    let bytes = match stored {
        Ok(bytes) => bytes,
        Err(e) => return note_unloaded_slot(store, kind, label, sink, &e),
    };
    match hydrate_slice_unmarked(store, kind, label, sink, || decode(&bytes), apply) {
        HydrateOutcome::Loaded => SlotOutcome::Loaded,
        HydrateOutcome::Defaulted | HydrateOutcome::Failed => SlotOutcome::Rejected,
    }
}

fn note_unloaded_slot(
    store: &RegistryStore,
    kind: PersistenceSliceKind,
    label: &str,
    sink: &mut HydrateSink<'_>,
    e: &StoreError,
) -> SlotOutcome {
    if let StoreError::Missing = e {
        note_missing_slice(store, kind, sink);
        return SlotOutcome::Missing;
    }
    note_unread_slice(store, kind, label, sink, e);
    SlotOutcome::Unread
}

fn apply_loaded_slice(
    store: &RegistryStore,
    kind: PersistenceSliceKind,
    sink: &mut HydrateSink<'_>,
    apply: impl FnOnce(&mut Inner),
) -> HydrateOutcome {
    apply(&mut store.write_inner());
    sink.note_loaded(kind);
    bump(&store.persist_counters.hydrate_loaded_total);
    HydrateOutcome::Loaded
}

fn note_missing_slice(store: &RegistryStore, kind: PersistenceSliceKind, sink: &mut HydrateSink<'_>) {
    sink.note_defaulted(kind);
    bump(&store.persist_counters.hydrate_default_total);
}

fn note_unread_slice(
    store: &RegistryStore,
    kind: PersistenceSliceKind,
    label: &str,
    sink: &mut HydrateSink<'_>,
    e: &StoreError,
) {
    sink.note_failed(&kind, || e.to_string());
    sink.note_defaulted(kind);
    bump(&store.persist_counters.hydrate_error_total);
    warn!("persistence: failed to load {label}: {e}");
}

const PHYSICAL_DEVICE_BANKS_U8: u8 =
    dali2rust_platform::slice_store::SliceKey::PHYSICAL_DEVICE_BANKS;

const INPUT_DEVICE_BANKS_U8: u8 = 4;

fn mark_slice_dirty(store: &RegistryStore, kind: &PersistenceSliceKind) {
    match *kind {
        PersistenceSliceKind::Adapters => store.dirty.adapters.store(true, Ordering::Release),
        PersistenceSliceKind::Groups { adapter_id } => store.dirty.mark_groups_dirty(adapter_id),
        PersistenceSliceKind::VirtualLamps { adapter_id } => {
            store.dirty.mark_virtual_lamps_dirty(adapter_id)
        }
        PersistenceSliceKind::PhysicalDevices { .. }
        | PersistenceSliceKind::PhysicalDeviceBank { .. } => {}
        PersistenceSliceKind::Scenes {
            adapter_id,
            scene_id,
        } => store.dirty.mark_scene_dirty(adapter_id, scene_id),
        PersistenceSliceKind::HclSchedules => store.dirty.mark_hcl_schedules_dirty(),
        PersistenceSliceKind::PollerSettings => store.dirty.mark_poller_settings_dirty(),
        PersistenceSliceKind::HomeAssistantSettings => {
            store.dirty.mark_home_assistant_settings_dirty()
        }
        PersistenceSliceKind::DaliSettings => store.dirty.mark_dali_settings_dirty(),
        PersistenceSliceKind::RedundancySettings => store.dirty.mark_redundancy_settings_dirty(),
        PersistenceSliceKind::Policies => store.dirty.mark_policies_dirty(),
        PersistenceSliceKind::InputDevices { .. } => store.dirty.mark_input_devices_dirty(),
    }
}

fn bump(counter: &std::sync::atomic::AtomicU32) {
    counter.fetch_add(1, Ordering::Relaxed);
}

macro_rules! hydrate_wrappers {
    (per_adapter: $(
        $name:ident($($id:ident),*) => $kind:expr, $slice:ty, $version:path, $apply:path;
    )*) => {$(
        fn $name(
            store: &RegistryStore,
            slices: &dyn SliceStore,
            $($id: u8,)*
            sink: &mut HydrateSink<'_>,
        ) -> SlotOutcome {
            hydrate_stored_slice(
                store,
                slices,
                $kind,
                sink,
                |bytes| decode_slice::<$slice>(bytes, $version),
                |inner, slice| $apply(inner, $($id,)* slice),
            )
        }
    )*};
    (global: $(
        $name:ident => $kind:expr, $label:literal, $default_log:literal, $slice:ty, $version:path,
            $apply:path;
    )*) => {$(
        fn $name(store: &RegistryStore, slices: &dyn SliceStore, sink: &mut HydrateSink<'_>) {
            let decode = |bytes: &[u8]| decode_slice::<$slice>(bytes, $version);
            match hydrate_stored_slice(store, slices, $kind, sink, decode, $apply) {
                SlotOutcome::Loaded => info!("persistence: loaded {}", $label),
                SlotOutcome::Missing => info!("persistence: {}", $default_log),
                SlotOutcome::Rejected | SlotOutcome::Unread => {}
            }
        }
    )*};
}

hydrate_wrappers! { global:
    hydrate_adapters_slice =>
        PersistenceSliceKind::Adapters,
        "adapters",
        "adapters file not found, using defaults",
        PersistableAdapterSlice,
        ADAPTERS_SLICE_VERSION,
        hydrate_adapters_inner;
    hydrate_hcl_schedules_slice =>
        PersistenceSliceKind::HclSchedules,
        "hcl schedules",
        "no hcl schedules stored",
        PersistableHclSchedulesSlice,
        HCL_SCHEDULES_SLICE_VERSION,
        crate::runtime::registry::hcl_schedules::hydrate_hcl_schedules_inner;
    hydrate_poller_settings_slice =>
        PersistenceSliceKind::PollerSettings,
        "poller settings",
        "no poller settings stored",
        PersistablePollerSettingsSlice,
        POLLER_SETTINGS_SLICE_VERSION,
        crate::runtime::registry::poller_settings::hydrate_poller_settings_inner;
    hydrate_dali_settings_slice =>
        PersistenceSliceKind::DaliSettings,
        "dali settings",
        "no dali settings stored",
        PersistableDaliSettingsSlice,
        DALI_SETTINGS_SLICE_VERSION,
        crate::runtime::registry::dali_settings::hydrate_dali_settings_inner;
    hydrate_redundancy_settings_slice =>
        PersistenceSliceKind::RedundancySettings,
        "redundancy settings",
        "no redundancy settings stored",
        PersistableRedundancySettingsSlice,
        REDUNDANCY_SETTINGS_SLICE_VERSION,
        crate::runtime::registry::redundancy_settings::hydrate_redundancy_settings_inner;
    hydrate_policies_slice =>
        PersistenceSliceKind::Policies,
        "policies",
        "no policies stored",
        PersistablePoliciesSlice,
        POLICIES_SLICE_VERSION,
        crate::runtime::registry::policies::hydrate_policies_inner;
}

hydrate_wrappers! { per_adapter:
    hydrate_groups_slice(adapter_id) =>
        PersistenceSliceKind::Groups { adapter_id },
        PersistableGroupsSlice,
        GROUPS_SLICE_VERSION,
        hydrate_groups_inner;
    hydrate_scene_slice(adapter_id, scene_id) =>
        PersistenceSliceKind::Scenes { adapter_id, scene_id },
        PersistableSceneSlice,
        SCENES_SLICE_VERSION,
        hydrate_scene_inner;
}

fn hydrate_input_device_banks(store: &RegistryStore, slices: &dyn SliceStore, sink: &mut HydrateSink<'_>) {
    let mut stored = std::collections::HashSet::new();
    let (mut whole, mut loaded) = (true, false);
    for bank in 0..INPUT_DEVICE_BANKS_U8 {
        let outcome = hydrate_stored_slice(
            store,
            slices,
            PersistenceSliceKind::InputDevices { bank },
            sink,
            |bytes| decode_slice::<PersistableInputDevicesSlice>(bytes, INPUT_DEVICES_SLICE_VERSION),
            |inner, slice| {
                stored.extend(slice.devices.iter().map(|d| (d.adapter_id, d.short_address)));
                crate::runtime::registry::input_devices::hydrate_input_devices_bank_inner(inner, bank, slice);
            },
        );
        whole &= matches!(outcome, SlotOutcome::Loaded | SlotOutcome::Missing);
        loaded |= outcome == SlotOutcome::Loaded;
    }
    if whole && loaded {
        store.write_inner().input_devices.retain(|key, _| stored.contains(key));
    }
}

fn hydrate_pd_bank_slice(
    store: &RegistryStore,
    slices: &dyn SliceStore,
    adapter_id: u8,
    bank: u8,
    sink: &mut HydrateSink<'_>,
) -> SlotOutcome {
    hydrate_read_slot(
        store,
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id, bank },
        &format!("PD a{adapter_id}/b{bank}"),
        sink,
        slices.load(SliceKey::PhysicalDeviceBank { adapter_id, bank }),
        |bytes| RegistryStore::decode_physical_device_bank(bytes, adapter_id, bank),
        |inner, slice| hydrate_physical_device_bank_inner(inner, adapter_id, bank, slice),
    )
}

fn hydrate_pd_slice(
    store: &RegistryStore,
    stored: Result<Vec<u8>, StoreError>,
    adapter_id: u8,
    banks: u16,
    held: u64,
    sink: &mut HydrateSink<'_>,
) -> SlotOutcome {
    hydrate_read_slot(
        store,
        PersistenceSliceKind::PhysicalDevices { adapter_id },
        &format!("PD a{adapter_id}"),
        sink,
        stored,
        |bytes| devices_of_banks(bytes, banks, held),
        |inner, slice| hydrate_physical_devices_inner(inner, adapter_id, slice),
    )
}

fn devices_of_banks(
    bytes: &[u8],
    banks: u16,
    held: u64,
) -> Result<PersistablePhysicalDevicesSlice, StoreError> {
    let mut slice = RegistryStore::decode_physical_devices(bytes)?;
    slice.devices.retain(|dev| {
        banks & bank_bit(dev.short_address) != 0 && held & short_bit(dev.short_address) == 0
    });
    Ok(slice)
}

fn hydrate_vl_slice(
    store: &RegistryStore,
    slices: &dyn SliceStore,
    adapter_id: u8,
    sink: &mut HydrateSink<'_>,
) -> SlotOutcome {
    let waiting = store.dirty.withheld_physical_device_banks(adapter_id);
    hydrate_stored_slice(
        store,
        slices,
        PersistenceSliceKind::VirtualLamps { adapter_id },
        sink,
        |bytes| decode_slice::<PersistableVirtualLampsSlice>(bytes, VIRTUAL_LAMPS_SLICE_VERSION),
        |inner, slice| hydrate_virtual_lamps_inner(inner, adapter_id, waiting, slice),
    )
}

fn hydrate_home_assistant_settings_slice(
    store: &RegistryStore,
    slices: &dyn SliceStore,
    sink: &mut HydrateSink<'_>,
) {
    let outcome = hydrate_stored_slice(
        store,
        slices,
        PersistenceSliceKind::HomeAssistantSettings,
        sink,
        |bytes| {
            decode_slice::<PersistableHomeAssistantSettingsSlice>(
                bytes,
                HOME_ASSISTANT_SETTINGS_SLICE_VERSION,
            )
        },
        crate::runtime::registry::home_assistant_settings::hydrate_home_assistant_settings_inner,
    );
    match outcome {
        SlotOutcome::Loaded => info!("persistence: loaded home assistant settings"),
        SlotOutcome::Missing => {
            info!("persistence: no home assistant settings stored, persisting derived defaults");
            store.dirty.mark_home_assistant_settings_dirty();
        }
        SlotOutcome::Rejected | SlotOutcome::Unread => {}
    }
}

fn push_slice(list: &mut PersistenceSliceList, kind: PersistenceSliceKind) {
    list.push(kind)
        .expect("persistence slice list capacity must cover all configured adapters");
}

fn push_error(errors: &mut PersistenceSliceErrorList, kind: &PersistenceSliceKind, error: &str) {
    errors
        .push(PersistenceSliceError {
            kind: kind.clone(),
            error: fixed_text_64(error),
        })
        .expect("persistence error list capacity must cover all hydrate failures");
}

fn hydrate_adapters_inner(inner: &mut super::store::Inner, slice: &PersistableAdapterSlice) {
    for row in &slice.adapters {
        if let Some(existing) = inner.adapters.get_mut(row.adapter_id as usize) {
            existing.name = fixed_text_64(&row.name);
            existing.enabled = row.enabled;
        }
    }
}

fn binding_short_to_keep(
    inner: &super::store::Inner,
    adapter_id: u8,
    waiting_banks: u16,
    binding_short: Option<u8>,
) -> Option<u8> {
    let sa = binding_short?;
    let waiting = waiting_banks & bank_bit(sa) != 0;
    (waiting || inner.physical_devices.contains_key(&(adapter_id, sa))).then_some(sa)
}

fn hydrate_virtual_lamps_inner(
    inner: &mut super::store::Inner,
    adapter_id: u8,
    waiting_banks: u16,
    slice: &PersistableVirtualLampsSlice,
) {
    inner.lamps.retain(|&(aid, id), _| {
        aid != adapter_id || slice.lamps.iter().any(|lamp| lamp.virtual_lamp_id == id)
    });
    for lamp in &slice.lamps {
        let validated =
            binding_short_to_keep(inner, adapter_id, waiting_banks, lamp.binding_short);
        if lamp.binding_short.is_some() && validated.is_none() {
            warn!(
                "persistence: dropped orphan VL binding a{adapter_id} vl{} -> short {:?}",
                lamp.virtual_lamp_id, lamp.binding_short
            );
        }
        let entry = inner
            .lamps
            .entry((adapter_id, lamp.virtual_lamp_id))
            .or_default();
        entry.name = fixed_text_64(&lamp.name);
        entry.ha_entity_enabled = lamp.ha_entity_enabled;
        entry.binding_short = validated;
    }
}

fn hydrate_groups_inner(
    inner: &mut super::store::Inner,
    adapter_id: u8,
    slice: &PersistableGroupsSlice,
) {
    for group in &slice.groups {
        inner.groups.insert(
            (adapter_id, group.group_id),
            GroupRecord {
                name: fixed_text_64(&group.name),
                ha_entity_enabled: group.ha_entity_enabled,
            },
        );
    }
    for row in &slice.rows {
        inner.group_matrix.insert(
            (adapter_id, row.virtual_lamp_id),
            GroupMembershipRowRecord {
                desired_groups_mask: row.desired_groups_mask,
                desired_seeded: row.desired_seeded,
                desired_from_operator: row.desired_from_operator,
            },
        );
    }
}

fn hydrate_scene_inner(
    inner: &mut super::store::Inner,
    adapter_id: u8,
    scene_id: u8,
    slice: &PersistableSceneSlice,
) {
    inner.scene_matrix.retain(|&(aid, sid, _), _| aid != adapter_id || sid != scene_id);
    inner.scenes.insert(
        (adapter_id, scene_id),
        SceneRecord {
            name: fixed_text_64(&slice.name),
            ha_select_enabled: slice.ha_select_enabled,
        },
    );
    for row in &slice.rows {
        let target = row.included.then_some(dali2rust_contracts::msg::DaliSceneTargetState {
            power: row.power,
            level: row.level,
            color: row.color,
        });
        let record = SceneDesiredRowRecord {
            included: row.included,
            target,
            desired_seeded: row.desired_seeded,
            desired_from_operator: row.desired_from_operator,
        };
        if row.included || row.desired_seeded {
            inner
                .scene_matrix
                .insert((adapter_id, scene_id, row.virtual_lamp_id), record);
        }
    }
}

fn hydrate_physical_device_bank_inner(
    inner: &mut super::store::Inner,
    adapter_id: u8,
    bank: u8,
    slice: &PersistablePhysicalDevicesSlice,
) {
    inner.physical_devices.retain(|&(aid, short), _| {
        aid != adapter_id
            || short / SliceKey::DEVICES_PER_BANK != bank
            || slice.devices.iter().any(|dev| dev.short_address == short)
    });
    hydrate_physical_devices_inner(inner, adapter_id, slice);
}

fn hydrate_physical_devices_inner(
    inner: &mut super::store::Inner,
    adapter_id: u8,
    slice: &PersistablePhysicalDevicesSlice,
) {
    for dev in &slice.devices {
        let entry = inner
            .physical_devices
            .entry((adapter_id, dev.short_address))
            .or_insert_with(PhysicalDeviceRecord::new_boxed);
        entry.random_address = dev.random_address;
        entry.name = fixed_text_64(&dev.name);
        entry.notes = dev.notes.as_ref().map(|n| fixed_text_64(n));
        entry.device_type_override = dev.device_type_override;
        entry.color_mode_override = dev.color_mode_override;
        entry.device_type_discovered = dev.device_type_discovered;
        entry.color_mode_discovered = dev.color_mode_discovered;
        entry.cap_cct = dev.cap_cct;
        entry.cap_xy = dev.cap_xy;
        entry.cap_rgb = dev.cap_rgb;
        entry.tc_coolest_mirek = dev.tc_coolest_mirek;
        entry.tc_warmest_mirek = dev.tc_warmest_mirek;
        entry.dt8_auto_activation_repair = dev.dt8_auto_activation_repair;
        entry.dt8_rgbwaf_control_assert = dev.dt8_rgbwaf_control_assert;
        entry.cap_rgbwaf = dev.cap_rgbwaf;
        entry.supported_device_types = dev.supported_device_types;
        entry.attributes = dev.attributes.clone();
        entry.memory_banks = hydrate_memory_bank_records(&dev.memory_banks);
    }
}

fn hydrate_memory_bank_records(banks: &[MemoryBankSummaryView]) -> Vec<MemoryBankRecord> {
    banks
        .iter()
        .map(|bank| MemoryBankRecord {
            bank: bank.bank,
            total_bytes_read: bank.total_bytes_read,
            last_read_ms: bank.last_read_ms,
            ranges: bank
                .ranges
                .iter()
                .map(|range| MemoryBankRangeRecord {
                    start: range.start,
                    length: range.length,
                })
                .collect(),
            bytes: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use dali2rust_platform::slice_store::{SliceKey, SliceStore};

    use crate::test_support::{CountingStore, ProbeStore};
    use super::super::input_devices::{InputDeviceRecord, INPUT_DEVICES_PER_BANK};
    use super::super::scenes::{SceneDesiredRowRecord, SCENE_COUNT};
    use super::super::store::RegistryStore;
    use super::super::virtual_lamps::VlRecord;

    const ADAPTER_COUNT: u8 = 2;

    fn registry_slices() -> Vec<SliceKey> {
        SliceKey::every(ADAPTER_COUNT)
            .filter(|key| {
                !matches!(
                    key,
                    SliceKey::PhysicalDevices { .. }
                        | SliceKey::PhysicalDeviceBank { .. }
                        | SliceKey::ControllerSettings
                        | SliceKey::Rules { .. }
                )
            })
            .collect()
    }

    fn held_together(key: SliceKey, unread: SliceKey) -> bool {
        key == unread
            || matches!((key, unread), (SliceKey::InputDevices { .. }, SliceKey::InputDevices { .. }))
    }

    fn mark_every_slice_dirty(store: &RegistryStore) {
        let dirty = &store.dirty;
        dirty.adapters.store(true, Ordering::Release);
        for adapter_id in 0..ADAPTER_COUNT {
            dirty.mark_groups_dirty(adapter_id);
            dirty.mark_virtual_lamps_dirty(adapter_id);
            (0..SCENE_COUNT).for_each(|scene_id| dirty.mark_scene_dirty(adapter_id, scene_id));
        }
        dirty.mark_hcl_schedules_dirty();
        dirty.mark_poller_settings_dirty();
        dirty.mark_dali_settings_dirty();
        dirty.mark_redundancy_settings_dirty();
        dirty.mark_policies_dirty();
        dirty.mark_home_assistant_settings_dirty();
        dirty.mark_input_devices_dirty();
    }

    fn installed() -> ProbeStore {
        let slices = ProbeStore::default();
        let store = RegistryStore::with_adapter_count(ADAPTER_COUNT);
        mark_every_slice_dirty(&store);
        store.flush_dirty_slices(&slices);
        for key in registry_slices() {
            assert!(slices.inner.load(key).is_ok(), "{} was not installed", key.label());
        }
        slices
    }

    fn boot_with_unread(slices: &ProbeStore, unread: SliceKey) -> RegistryStore {
        slices.fail_reads_of(Some(unread));
        let store = RegistryStore::with_adapter_count(ADAPTER_COUNT);
        store.hydrate_from_store(slices, ADAPTER_COUNT);
        store
    }

    fn flush_everything(store: &RegistryStore, slices: &ProbeStore) -> Vec<SliceKey> {
        let before = slices.written().len();
        mark_every_slice_dirty(store);
        store.flush_dirty_slices(slices);
        slices.written().split_off(before)
    }

    fn unread_slices(store: &RegistryStore) -> u32 {
        store.persist_counters.unread_slices.load(Ordering::Acquire)
    }

    #[test]
    fn a_slice_whose_read_fails_is_never_written_over() {
        for unread in registry_slices() {
            let slices = installed();
            let store = boot_with_unread(&slices, unread);
            let label = unread.label();
            assert!(store.dirty.held_back(ADAPTER_COUNT).is_empty(), "{label} marked for rewrite");
            assert_eq!(unread_slices(&store), 1, "{label}");

            let written = flush_everything(&store, &slices);
            assert!(!store.dirty.any_dirty(), "{label} still counts as work");
            for key in registry_slices() {
                let expected = !held_together(key, unread);
                assert_eq!(written.contains(&key), expected, "{} with {label} unread", key.label());
            }
        }
    }

    #[test]
    fn input_device_banks_wait_together_because_a_bank_is_a_position_in_the_whole_list() {
        let installed_devices = u8::try_from(INPUT_DEVICES_PER_BANK + INPUT_DEVICES_PER_BANK / 2)
            .expect("a short address");
        let slices = ProbeStore::default();
        let seeded = RegistryStore::with_adapter_count(1);
        for short in 0..installed_devices {
            seeded.write_inner().input_devices.insert((0, short), InputDeviceRecord::empty(0, short));
        }
        seeded.dirty.mark_input_devices_dirty();
        seeded.flush_dirty_slices(&slices);

        let store = boot_with_unread(&slices, SliceKey::InputDevices { bank: 0 });
        flush_everything(&store, &slices);

        slices.fail_reads_of(None);
        let rebooted = RegistryStore::with_adapter_count(1);
        rebooted.hydrate_from_store(&slices, 1);
        assert_eq!(rebooted.read_inner().input_devices.len(), usize::from(installed_devices));
    }

    #[test]
    fn a_withheld_settings_slice_keeps_the_flush_debounce() {
        let slices = installed();
        let store = boot_with_unread(&slices, SliceKey::PollerSettings);
        store.dirty.mark_poller_settings_dirty();
        assert!(!store.dirty.deliberate_config_write(), "nothing deliberate can be written");
        store.dirty.mark_hcl_schedules_dirty();
        assert!(store.dirty.deliberate_config_write());
    }

    #[test]
    fn a_reload_that_reads_the_slice_lets_the_next_flush_write_it() {
        let unread = SliceKey::VirtualLamps { adapter_id: 0 };
        let slices = installed();
        let store = boot_with_unread(&slices, unread);

        slices.fail_reads_of(None);
        store.hydrate_from_store(&slices, ADAPTER_COUNT);
        assert_eq!(unread_slices(&store), 0);
        assert!(flush_everything(&store, &slices).contains(&unread));
    }

    fn hold_records_the_store_lacks(store: &RegistryStore) {
        let mut inner = store.write_inner();
        inner.lamps.insert((0, STRAY_ID), VlRecord::default());
        inner.scene_matrix.insert((0, 0, STRAY_ID), SceneDesiredRowRecord::default());
        inner.input_devices.insert((0, STRAY_ID), InputDeviceRecord::empty(0, STRAY_ID));
    }

    fn stray_records(store: &RegistryStore) -> [bool; 3] {
        let inner = store.read_inner();
        [
            inner.lamps.contains_key(&(0, STRAY_ID)),
            inner.scene_matrix.contains_key(&(0, 0, STRAY_ID)),
            inner.input_devices.contains_key(&(0, STRAY_ID)),
        ]
    }

    const STRAY_ID: u8 = 9;

    #[test]
    fn a_reload_leaves_only_the_records_each_loaded_slice_holds() {
        let slices = installed();
        let store = RegistryStore::with_adapter_count(ADAPTER_COUNT);
        store.hydrate_from_store(&slices, ADAPTER_COUNT);
        hold_records_the_store_lacks(&store);

        store.hydrate_from_store(&slices, ADAPTER_COUNT);
        assert_eq!(stray_records(&store), [false; 3], "lamp, scene row, input device");
    }

    #[test]
    fn a_reload_keeps_the_records_of_a_slice_it_could_not_read() {
        let slices = installed();
        let store = RegistryStore::with_adapter_count(ADAPTER_COUNT);
        store.hydrate_from_store(&slices, ADAPTER_COUNT);
        hold_records_the_store_lacks(&store);

        slices.fail_reads_of(Some(SliceKey::VirtualLamps { adapter_id: 0 }));
        store.hydrate_from_store(&slices, ADAPTER_COUNT);
        assert_eq!(stray_records(&store), [true, false, false]);
        slices.fail_reads_of(Some(SliceKey::InputDevices { bank: 2 }));
        hold_records_the_store_lacks(&store);
        store.hydrate_from_store(&slices, ADAPTER_COUNT);
        assert!(stray_records(&store)[2], "one unread bank holds the whole list");
    }

    #[test]
    fn a_flush_writes_only_the_dirty_slices() {
        let slices = CountingStore::default();
        let store = RegistryStore::with_adapter_count(1);

        store.dirty.mark_physical_device_dirty(0, 3);
        store.flush_dirty_slices(&slices);
        assert_eq!(slices.write_count(), 1);
        assert_eq!(slices.inner.labels(), vec!["a0/physical_devices_b0"]);

        store.dirty.mark_physical_device_dirty(0, 40);
        store.flush_dirty_slices(&slices);
        assert_eq!(slices.write_count(), 2);
        assert!(slices.inner.labels().contains(&"a0/physical_devices_b10".to_string()));

        store.dirty.mark_all_physical_devices_dirty(0);
        store.flush_dirty_slices(&slices);
        assert_eq!(slices.write_count(), 18);

        store.dirty.mark_virtual_lamps_dirty(0);
        store.flush_dirty_slices(&slices);
        assert_eq!(slices.write_count(), 19);

        store.flush_dirty_slices(&slices);
        assert_eq!(slices.write_count(), 19);
    }
}
