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

    fn physical_device_count_of(&self, adapter_id: u8) -> usize {
        self.read_inner()
            .physical_devices
            .keys()
            .filter(|(aid, _)| *aid == adapter_id)
            .count()
    }

    fn hydrate_physical_devices_of(
        &self,
        slices: &dyn SliceStore,
        adapter_id: u8,
        sink: &mut HydrateSink<'_>,
    ) {
        for bank in 0..PHYSICAL_DEVICE_BANKS_U8 {
            let _ = hydrate_pd_bank_slice(self, slices, adapter_id, bank, sink);
        }
        if self.physical_device_count_of(adapter_id) > 0 {
            return;
        }
        if matches!(
            hydrate_pd_slice(self, slices, adapter_id, sink),
            HydrateOutcome::Loaded
        ) {
            info!(
                "persistence: PD a{adapter_id} migrated from the whole-adapter slice; \
                 the next flush writes it as banks"
            );
            self.dirty.mark_all_physical_devices_dirty(adapter_id);
        }
    }

    fn hydrate_all(&self, slices: &dyn SliceStore, adapter_count: u8, sink: &mut HydrateSink<'_>) {
        hydrate_adapters_slice(self, slices, sink);
        for adapter_id in 0..adapter_count {
            hydrate_groups_slice(self, slices, adapter_id, sink);
            self.hydrate_physical_devices_of(slices, adapter_id, sink);
            hydrate_vl_slice(self, slices, adapter_id, sink);
            for scene_id in 0..SCENE_COUNT {
                hydrate_scene_slice(self, slices, adapter_id, scene_id, sink);
            }
        }
        for bank in 0..INPUT_DEVICE_BANKS_U8 {
            hydrate_input_devices_slice(self, slices, bank, sink);
        }
        hydrate_hcl_schedules_slice(self, slices, sink);
        hydrate_poller_settings_slice(self, slices, sink);
        hydrate_dali_settings_slice(self, slices, sink);
        hydrate_redundancy_settings_slice(self, slices, sink);
        hydrate_policies_slice(self, slices, sink);
        hydrate_home_assistant_settings_slice(self, slices, sink);
    }

    pub fn flush_dirty_slices(&self, slices: &dyn SliceStore) {
        if !self.dirty.any_dirty() {
            return;
        }
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

    fn flush_physical_device_banks(&self, slices: &dyn SliceStore) {
        let _ = self.dirty.take_physical_devices_dirty();
        let g = self.read_inner();
        let adapter_count = g.adapters.len() as u8;
        drop(g);
        for adapter_id in 0..adapter_count {
            let mask = self.dirty.take_physical_device_banks_dirty(adapter_id);
            if mask == 0 {
                continue;
            }
            for bank in 0..dali2rust_platform::slice_store::SliceKey::PHYSICAL_DEVICE_BANKS {
                if mask & (1u16 << bank) == 0 {
                    continue;
                }
                if let Err(e) = self.flush_physical_device_bank(slices, adapter_id, bank) {
                    warn!("persistence: PD a{adapter_id}/b{bank} flush failed: {e}");
                    self.dirty.mark_physical_device_dirty(
                        adapter_id,
                        bank * dali2rust_platform::slice_store::SliceKey::DEVICES_PER_BANK,
                    );
                    self.persist_counters
                        .flush_error_total
                        .fetch_add(1, Ordering::Relaxed);
                } else {
                    self.persist_counters
                        .flush_success_total
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
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
            warn!("persistence: {name} flush failed: {e}");
            remark();
            self.persist_counters
                .flush_error_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        self.flush_global_slice_if_dirty(
            slices,
            self.dirty.take_hcl_schedules_dirty(),
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
                warn!("persistence: {name} flush failed: {e}");
                self.dirty.mark_input_devices_dirty();
                self.persist_counters
                    .flush_error_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

    fn load_global_slice<T: serde::de::DeserializeOwned>(
        slices: &dyn SliceStore,
        key: SliceKey,
        version: u32,
    ) -> Result<T, StoreError> {
        let bytes = slices.load(key)?;
        decode_versioned_slice::<T>(&bytes, version).map_err(|e| StoreError::Backend(e.to_string()))
    }

    fn load_hcl_schedules(
        slices: &dyn SliceStore,
    ) -> Result<PersistableHclSchedulesSlice, StoreError> {
        Self::load_global_slice(slices, SliceKey::HclSchedules, HCL_SCHEDULES_SLICE_VERSION)
    }

    fn load_poller_settings(
        slices: &dyn SliceStore,
    ) -> Result<PersistablePollerSettingsSlice, StoreError> {
        Self::load_global_slice(slices, SliceKey::PollerSettings, POLLER_SETTINGS_SLICE_VERSION)
    }

    fn load_input_devices_bank(
        slices: &dyn SliceStore,
        bank: u8,
    ) -> Result<PersistableInputDevicesSlice, StoreError> {
        Self::load_global_slice(
            slices,
            SliceKey::InputDevices { bank },
            INPUT_DEVICES_SLICE_VERSION,
        )
    }

    fn load_policies(slices: &dyn SliceStore) -> Result<PersistablePoliciesSlice, StoreError> {
        Self::load_global_slice(slices, SliceKey::Policies, POLICIES_SLICE_VERSION)
    }

    fn load_redundancy_settings(
        slices: &dyn SliceStore,
    ) -> Result<PersistableRedundancySettingsSlice, StoreError> {
        Self::load_global_slice(
            slices,
            SliceKey::RedundancySettings,
            REDUNDANCY_SETTINGS_SLICE_VERSION,
        )
    }

    fn load_dali_settings(slices: &dyn SliceStore) -> Result<PersistableDaliSettingsSlice, StoreError> {
        Self::load_global_slice(slices, SliceKey::DaliSettings, DALI_SETTINGS_SLICE_VERSION)
    }

    fn load_home_assistant_settings(
        slices: &dyn SliceStore,
    ) -> Result<PersistableHomeAssistantSettingsSlice, StoreError> {
        Self::load_global_slice(
            slices,
            SliceKey::HomeAssistantSettings,
            HOME_ASSISTANT_SETTINGS_SLICE_VERSION,
        )
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
                    warn!("persistence: scene a{adapter_id}/s{scene_id} flush failed: {e}");
                    self.dirty.mark_scene_dirty(adapter_id, scene_id);
                    self.persist_counters
                        .flush_error_total
                        .fetch_add(1, Ordering::Relaxed);
                } else {
                    self.persist_counters
                        .flush_success_total
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    fn flush_adapters_if_dirty(&self, slices: &dyn SliceStore) {
        if !self.dirty.adapters.load(Ordering::Acquire) {
            return;
        }
        if let Err(e) = self.flush_adapters(slices) {
            warn!("persistence: adapters flush failed: {e}");
            self.persist_counters
                .flush_error_total
                .fetch_add(1, Ordering::Relaxed);
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
                warn!("persistence: {label} a{adapter_id} flush failed: {e}");
                remark_dirty(self, adapter_id);
                self.persist_counters
                    .flush_error_total
                    .fetch_add(1, Ordering::Relaxed);
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


    fn load_adapters(
        slices: &dyn SliceStore,
    ) -> Result<PersistableAdapterSlice, StoreError> {
        let bytes = slices.load(SliceKey::Adapters)?;
        decode_versioned_slice::<PersistableAdapterSlice>(&bytes, ADAPTERS_SLICE_VERSION)
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    fn load_virtual_lamps(
        slices: &dyn SliceStore,
        adapter_id: u8,
    ) -> Result<PersistableVirtualLampsSlice, StoreError> {
        let bytes = slices.load(SliceKey::VirtualLamps { adapter_id })?;
        decode_versioned_slice::<PersistableVirtualLampsSlice>(&bytes, VIRTUAL_LAMPS_SLICE_VERSION)
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    fn load_groups(
        slices: &dyn SliceStore,
        adapter_id: u8,
    ) -> Result<PersistableGroupsSlice, StoreError> {
        let bytes = slices.load(SliceKey::Groups { adapter_id })?;
        decode_versioned_slice::<PersistableGroupsSlice>(&bytes, GROUPS_SLICE_VERSION)
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    fn load_scene(
        slices: &dyn SliceStore,
        adapter_id: u8,
        scene_id: u8,
    ) -> Result<PersistableSceneSlice, StoreError> {
        let bytes = slices.load(SliceKey::Scene { adapter_id, scene_id })?;
        decode_versioned_slice::<PersistableSceneSlice>(&bytes, SCENES_SLICE_VERSION)
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    fn load_physical_devices(
        slices: &dyn SliceStore,
        adapter_id: u8,
    ) -> Result<PersistablePhysicalDevicesSlice, StoreError> {
        let bytes = slices.load(SliceKey::PhysicalDevices { adapter_id })?;
        decode_versioned_slice::<PersistablePhysicalDevicesSlice>(&bytes, PHYSICAL_DEVICES_SLICE_VERSION)
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    fn load_physical_device_bank(
        slices: &dyn SliceStore,
        adapter_id: u8,
        bank: u8,
    ) -> Result<PersistablePhysicalDevicesSlice, StoreError> {
        let bytes = slices.load(SliceKey::PhysicalDeviceBank { adapter_id, bank })?;
        let slice = decode_versioned_slice::<PersistablePhysicalDevicesSlice>(
            &bytes,
            PHYSICAL_DEVICES_SLICE_VERSION,
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        if let Some(stray) = slice
            .devices
            .iter()
            .find(|d| d.short_address / SliceKey::DEVICES_PER_BANK != bank)
        {
            return Err(StoreError::Backend(format!(
                "a{adapter_id}/b{bank} holds short {} - bank geometry moved under \
                 the stored bytes; falling back to the whole-adapter slice",
                stray.short_address
            )));
        }
        Ok(slice)
    }
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

fn hydrate_slice<S>(
    store: &RegistryStore,
    kind: PersistenceSliceKind,
    label: &str,
    sink: &mut HydrateSink<'_>,
    load: impl FnOnce() -> Result<S, StoreError>,
    apply: impl FnOnce(&mut Inner, &S),
) -> HydrateOutcome {
    match load() {
        Ok(slice) => {
            let mut g = store.write_inner();
            apply(&mut g, &slice);
            drop(g);
            sink.note_loaded(kind);
            bump(&store.persist_counters.hydrate_loaded_total);
            HydrateOutcome::Loaded
        }
        Err(StoreError::Missing) => {
            sink.note_defaulted(kind);
            bump(&store.persist_counters.hydrate_default_total);
            HydrateOutcome::Defaulted
        }
        Err(e) => {
            sink.note_failed(&kind, || e.to_string());
            mark_slice_dirty(store, &kind);
            sink.note_defaulted(kind);
            bump(&store.persist_counters.hydrate_error_total);
            warn!("persistence: failed to load {label}: {e}");
            HydrateOutcome::Failed
        }
    }
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
        PersistenceSliceKind::PhysicalDevices { adapter_id } => {
            store.dirty.mark_physical_devices_dirty(adapter_id)
        }
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id, bank } => store
            .dirty
            .mark_physical_device_dirty(
                adapter_id,
                bank * dali2rust_platform::slice_store::SliceKey::DEVICES_PER_BANK,
            ),
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
        $name:ident($($id:ident),*) => $kind:expr, $label:expr, $load:path, $apply:path;
    )*) => {$(
        fn $name(
            store: &RegistryStore,
            slices: &dyn SliceStore,
            $($id: u8,)*
            sink: &mut HydrateSink<'_>,
        ) -> HydrateOutcome {
            hydrate_slice(
                store,
                $kind,
                &$label,
                sink,
                || $load(slices, $($id),*),
                |inner, slice| $apply(inner, $($id,)* slice),
            )
        }
    )*};
    (global: $(
        $name:ident => $kind:expr, $label:literal, $default_log:literal, $load:path, $apply:path;
    )*) => {$(
        fn $name(store: &RegistryStore, slices: &dyn SliceStore, sink: &mut HydrateSink<'_>) {
            match hydrate_slice(store, $kind, $label, sink, || $load(slices), $apply) {
                HydrateOutcome::Loaded => info!("persistence: loaded {}", $label),
                HydrateOutcome::Defaulted => info!("persistence: {}", $default_log),
                HydrateOutcome::Failed => {}
            }
        }
    )*};
}

hydrate_wrappers! { global:
    hydrate_adapters_slice =>
        PersistenceSliceKind::Adapters,
        "adapters",
        "adapters file not found, using defaults",
        RegistryStore::load_adapters,
        hydrate_adapters_inner;
    hydrate_hcl_schedules_slice =>
        PersistenceSliceKind::HclSchedules,
        "hcl schedules",
        "no hcl schedules stored",
        RegistryStore::load_hcl_schedules,
        crate::runtime::registry::hcl_schedules::hydrate_hcl_schedules_inner;
    hydrate_poller_settings_slice =>
        PersistenceSliceKind::PollerSettings,
        "poller settings",
        "no poller settings stored",
        RegistryStore::load_poller_settings,
        crate::runtime::registry::poller_settings::hydrate_poller_settings_inner;
    hydrate_dali_settings_slice =>
        PersistenceSliceKind::DaliSettings,
        "dali settings",
        "no dali settings stored",
        RegistryStore::load_dali_settings,
        crate::runtime::registry::dali_settings::hydrate_dali_settings_inner;
    hydrate_redundancy_settings_slice =>
        PersistenceSliceKind::RedundancySettings,
        "redundancy settings",
        "no redundancy settings stored",
        RegistryStore::load_redundancy_settings,
        crate::runtime::registry::redundancy_settings::hydrate_redundancy_settings_inner;
    hydrate_policies_slice =>
        PersistenceSliceKind::Policies,
        "policies",
        "no policies stored",
        RegistryStore::load_policies,
        crate::runtime::registry::policies::hydrate_policies_inner;
}

hydrate_wrappers! { per_adapter:
    hydrate_vl_slice(adapter_id) =>
        PersistenceSliceKind::VirtualLamps { adapter_id },
        format!("VL a{adapter_id}"),
        RegistryStore::load_virtual_lamps,
        hydrate_virtual_lamps_inner;
    hydrate_groups_slice(adapter_id) =>
        PersistenceSliceKind::Groups { adapter_id },
        format!("groups a{adapter_id}"),
        RegistryStore::load_groups,
        hydrate_groups_inner;
    hydrate_pd_slice(adapter_id) =>
        PersistenceSliceKind::PhysicalDevices { adapter_id },
        format!("PD a{adapter_id}"),
        RegistryStore::load_physical_devices,
        hydrate_physical_devices_inner;
    hydrate_pd_bank_slice(adapter_id, bank) =>
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id, bank },
        format!("PD a{adapter_id}/b{bank}"),
        RegistryStore::load_physical_device_bank,
        hydrate_physical_device_bank_inner;
    hydrate_scene_slice(adapter_id, scene_id) =>
        PersistenceSliceKind::Scenes { adapter_id, scene_id },
        format!("scene a{adapter_id}/s{scene_id}"),
        RegistryStore::load_scene,
        hydrate_scene_inner;
    hydrate_input_devices_slice(bank) =>
        PersistenceSliceKind::InputDevices { bank },
        format!("input devices b{bank}"),
        RegistryStore::load_input_devices_bank,
        crate::runtime::registry::input_devices::hydrate_input_devices_bank_inner;
}

fn hydrate_home_assistant_settings_slice(
    store: &RegistryStore,
    slices: &dyn SliceStore,
    sink: &mut HydrateSink<'_>,
) {
    let outcome = hydrate_slice(
        store,
        PersistenceSliceKind::HomeAssistantSettings,
        "home assistant settings",
        sink,
        || RegistryStore::load_home_assistant_settings(slices),
        crate::runtime::registry::home_assistant_settings::hydrate_home_assistant_settings_inner,
    );
    match outcome {
        HydrateOutcome::Loaded => info!("persistence: loaded home assistant settings"),
        HydrateOutcome::Defaulted => {
            info!("persistence: no home assistant settings stored, persisting derived defaults");
            store.dirty.mark_home_assistant_settings_dirty();
        }
        HydrateOutcome::Failed => {}
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

fn binding_short_if_physical_exists(
    inner: &super::store::Inner,
    adapter_id: u8,
    binding_short: Option<u8>,
) -> Option<u8> {
    let sa = binding_short?;
    inner
        .physical_devices
        .contains_key(&(adapter_id, sa))
        .then_some(sa)
}

fn hydrate_virtual_lamps_inner(
    inner: &mut super::store::Inner,
    adapter_id: u8,
    slice: &PersistableVirtualLampsSlice,
) {
    for lamp in &slice.lamps {
        let validated = binding_short_if_physical_exists(inner, adapter_id, lamp.binding_short);
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
    _bank: u8,
    slice: &PersistablePhysicalDevicesSlice,
) {
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
    use crate::test_support::CountingStore;
    use super::super::store::RegistryStore;

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
