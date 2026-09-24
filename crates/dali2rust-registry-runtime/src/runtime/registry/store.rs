use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::RwLock;

use dali2rust_bsp::psram::PsramBox;
use dali2rust_bsp::unix_clock::unix_wall_clock_millis;
use dali2rust_contracts::msg::fixed_text_64;
use dali2rust_domain::registry::RegistryReadPort;

use super::adapters::AdapterRow;
use super::groups::{GroupMembershipRowRecord, GroupRecord};
use super::hcl_schedules::HclScheduleMap;
use super::memory_banks::MemoryBankStaging;
use super::physical_devices::PhysicalDeviceRecord;
use super::scenes::{SceneDesiredRowRecord, SceneRecord};
use super::virtual_lamps::VlRecord;
use dali2rust_contracts::msg::DaliSceneTargetState;

pub(crate) use dali2rust_domain::registry::MAX_SHORT_ADDRESSES;
pub(crate) const MAX_DIRTY_ADAPTERS: usize = 8;

pub fn registry_unix_ms() -> u64 {
    unix_wall_clock_millis()
}

pub(crate) trait Staged {
    fn staged_at_ms(&self) -> u64;
}

pub(crate) const STAGE_MAX_AGE_MS: u64 = 15_000;

pub(crate) fn evict_stale<K, V: Staged>(
    map: &mut std::collections::HashMap<K, V>,
    now_ms: u64,
    max_age_ms: u64,
) where
    K: std::hash::Hash + Eq,
{
    map.retain(|_, staged| now_ms.saturating_sub(staged.staged_at_ms()) <= max_age_ms);
}

macro_rules! global_dirty_flag {
    ($mark:ident, $take:ident, $field:ident) => {
        pub fn $mark(&self) {
            self.$field.store(true, Ordering::Release);
        }

        pub fn $take(&self) -> bool {
            self.$field.swap(false, Ordering::AcqRel)
        }
    };
}

pub(crate) struct DirtyFlags {
    pub adapters: AtomicBool,
    pub groups: AtomicU32,
    pub physical_devices: AtomicU32,
    pub physical_device_banks: [std::sync::atomic::AtomicU16; MAX_DIRTY_ADAPTERS],
    pub virtual_lamps: AtomicU32,
    pub scenes: [std::sync::atomic::AtomicU16; MAX_DIRTY_ADAPTERS],
    pub hcl_schedules: AtomicBool,
    pub poller_settings: AtomicBool,
    pub dali_settings: AtomicBool,
    pub redundancy_settings: AtomicBool,
    pub policies: AtomicBool,
    pub home_assistant_settings: AtomicBool,
    pub input_devices: AtomicBool,
}

impl DirtyFlags {
    pub fn new() -> Self {
        Self {
            adapters: AtomicBool::new(false),
            groups: AtomicU32::new(0),
            physical_devices: AtomicU32::new(0),
            physical_device_banks: std::array::from_fn(|_| std::sync::atomic::AtomicU16::new(0)),
            virtual_lamps: AtomicU32::new(0),
            scenes: std::array::from_fn(|_| std::sync::atomic::AtomicU16::new(0)),
            hcl_schedules: AtomicBool::new(false),
            poller_settings: AtomicBool::new(false),
            dali_settings: AtomicBool::new(false),
            redundancy_settings: AtomicBool::new(false),
            policies: AtomicBool::new(false),
            home_assistant_settings: AtomicBool::new(false),
            input_devices: AtomicBool::new(false),
        }
    }

    pub fn any_dirty(&self) -> bool {
        self.adapters.load(Ordering::Acquire)
            || self.groups.load(Ordering::Acquire) != 0
            || self.physical_devices.load(Ordering::Acquire) != 0
            || self
                .physical_device_banks
                .iter()
                .any(|mask| mask.load(Ordering::Acquire) != 0)
            || self.virtual_lamps.load(Ordering::Acquire) != 0
            || self.scenes.iter().any(|mask| mask.load(Ordering::Acquire) != 0)
            || self.hcl_schedules.load(Ordering::Acquire)
            || self.poller_settings.load(Ordering::Acquire)
            || self.dali_settings.load(Ordering::Acquire)
            || self.redundancy_settings.load(Ordering::Acquire)
            || self.policies.load(Ordering::Acquire)
            || self.home_assistant_settings.load(Ordering::Acquire)
            || self.input_devices.load(Ordering::Acquire)
    }

    pub fn mark_scene_dirty(&self, adapter_id: u8, scene_id: u8) {
        if let Some(mask) = self.scenes.get(adapter_id as usize) {
            mask.fetch_or(1u16 << scene_id, Ordering::Release);
        }
    }

    pub fn take_scenes_dirty(&self, adapter_id: u8) -> u16 {
        self.scenes
            .get(adapter_id as usize)
            .map(|mask| mask.swap(0, Ordering::AcqRel))
            .unwrap_or(0)
    }

    global_dirty_flag!(mark_hcl_schedules_dirty, take_hcl_schedules_dirty, hcl_schedules);
    global_dirty_flag!(mark_dali_settings_dirty, take_dali_settings_dirty, dali_settings);
    global_dirty_flag!(
        mark_redundancy_settings_dirty,
        take_redundancy_settings_dirty,
        redundancy_settings
    );
    global_dirty_flag!(mark_policies_dirty, take_policies_dirty, policies);
    global_dirty_flag!(
        mark_poller_settings_dirty,
        take_poller_settings_dirty,
        poller_settings
    );
    global_dirty_flag!(
        mark_home_assistant_settings_dirty,
        take_home_assistant_settings_dirty,
        home_assistant_settings
    );
    global_dirty_flag!(mark_input_devices_dirty, take_input_devices_dirty, input_devices);

    pub fn mark_groups_dirty(&self, adapter_id: u8) {
        self.groups.fetch_or(1u32 << adapter_id, Ordering::Release);
    }

    pub fn mark_physical_device_dirty(&self, adapter_id: u8, short_address: u8) {
        let Some(mask) = self.physical_device_banks.get(adapter_id as usize) else {
            return;
        };
        let bank = short_address / dali2rust_platform::slice_store::SliceKey::DEVICES_PER_BANK;
        if bank < dali2rust_platform::slice_store::SliceKey::PHYSICAL_DEVICE_BANKS {
            mask.fetch_or(1u16 << bank, Ordering::Release);
        }
    }

    pub fn mark_all_physical_devices_dirty(&self, adapter_id: u8) {
        if let Some(mask) = self.physical_device_banks.get(adapter_id as usize) {
            mask.fetch_or(u16::MAX, Ordering::Release);
        }
        self.physical_devices
            .fetch_or(1u32 << adapter_id, Ordering::Release);
    }

    pub fn mark_physical_devices_dirty(&self, adapter_id: u8) {
        self.mark_all_physical_devices_dirty(adapter_id);
    }

    pub fn mark_virtual_lamps_dirty(&self, adapter_id: u8) {
        self.virtual_lamps
            .fetch_or(1u32 << adapter_id, Ordering::Release);
    }

    pub fn take_physical_devices_dirty(&self) -> u32 {
        self.physical_devices.swap(0, Ordering::AcqRel)
    }

    #[cfg(test)]
    pub fn take_physical_devices_dirty_for(&self, adapter_id: u8) -> bool {
        let banks = self.take_physical_device_banks_dirty(adapter_id);
        let coarse = self
            .physical_devices
            .fetch_and(!(1u32 << adapter_id), Ordering::AcqRel)
            & (1u32 << adapter_id);
        banks != 0 || coarse != 0
    }

    pub fn take_physical_device_banks_dirty(&self, adapter_id: u8) -> u16 {
        self.physical_device_banks
            .get(adapter_id as usize)
            .map(|mask| mask.swap(0, Ordering::AcqRel))
            .unwrap_or(0)
    }

    pub fn take_groups_dirty(&self) -> u32 {
        self.groups.swap(0, Ordering::AcqRel)
    }

    pub fn take_virtual_lamps_dirty(&self) -> u32 {
        self.virtual_lamps.swap(0, Ordering::AcqRel)
    }
}

#[derive(Default)]
pub(crate) struct Inner {
    pub(crate) adapters: Vec<AdapterRow>,
    pub(crate) groups: HashMap<(u8, u8), GroupRecord>,
    pub(crate) group_matrix: HashMap<(u8, u8), GroupMembershipRowRecord>,
    pub(crate) scenes: HashMap<(u8, u8), SceneRecord>,
    pub(crate) scene_matrix: HashMap<(u8, u8, u8), SceneDesiredRowRecord>,
    pub(crate) scene_applied_echo: HashMap<(u8, u8, u8), DaliSceneTargetState>,
    pub(crate) physical_devices: HashMap<(u8, u8), PsramBox<PhysicalDeviceRecord>>,
    pub(crate) lamps: HashMap<(u8, u8), VlRecord>,
    pub(crate) memory_bank_stage: HashMap<u64, MemoryBankStaging>,
    pub(crate) hcl_schedules: HclScheduleMap,
    pub(crate) hcl_schedule_stage: super::hcl_schedules::HclScheduleStageMap,
    pub(crate) config_write_stage: HashMap<
        super::config_write_stage::ConfigWriteStageKey,
        super::config_write_stage::ConfigWriteStage,
    >,
    pub(crate) input_devices:
        HashMap<(u8, u8), super::input_devices::InputDeviceRecord>,
    pub(crate) poller_settings: super::poller_settings::PollerSettingsRecord,
    pub(crate) dali_settings: super::dali_settings::DaliSettingsRecord,
    pub(crate) redundancy_settings: super::redundancy_settings::RedundancySettingsRecord,
    pub(crate) policies: super::policies::PoliciesRecord,
    pub(crate) home_assistant_settings:
        super::home_assistant_settings::HomeAssistantSettingsRecord,
    pub(crate) active_scene: HashMap<u8, u8>,
    pub(crate) group_commanded: std::collections::HashSet<(u8, u8)>,
    pub(crate) adapters_revision: u32,
    pub(crate) groups_metadata_revision: u32,
    pub(crate) group_matrix_revision: u32,
    pub(crate) scenes_metadata_revision: u32,
    pub(crate) scenes_matrix_desired_revision: u32,
    pub(crate) virtual_lamps_metadata_revision: u32,
    pub(crate) physical_devices_revision: u32,
    pub(crate) hcl_schedules_revision: u32,
    pub(crate) input_devices_revision: u32,
}

mod psram_capacity {
    pub(super) const BY_ADAPTER_AND_ID: usize = 256;
    pub(super) const SCENE_ROWS: usize = 1024;
    pub(super) const SMALL_ROWS: usize = 4096;
    pub(super) const SCHEDULES: usize = 32;
    pub(super) const STAGING: usize = 512;
}

impl Inner {
    pub(crate) fn preallocated() -> Self {
        use psram_capacity as cap;
        Self {
            groups: HashMap::with_capacity(cap::BY_ADAPTER_AND_ID),
            group_matrix: HashMap::with_capacity(cap::SMALL_ROWS),
            scenes: HashMap::with_capacity(cap::BY_ADAPTER_AND_ID),
            scene_matrix: HashMap::with_capacity(cap::SCENE_ROWS),
            scene_applied_echo: HashMap::with_capacity(cap::SCENE_ROWS),
            physical_devices: HashMap::with_capacity(cap::SMALL_ROWS),
            lamps: HashMap::with_capacity(cap::BY_ADAPTER_AND_ID),
            memory_bank_stage: HashMap::with_capacity(cap::STAGING),
            hcl_schedules: HashMap::with_capacity(cap::SCHEDULES),
            hcl_schedule_stage: HashMap::with_capacity(cap::SCHEDULES),
            input_devices: HashMap::with_capacity(cap::SMALL_ROWS),
            config_write_stage: HashMap::with_capacity(
                super::config_write_stage::MAX_CONFIG_WRITE_STAGES,
            ),
            ..Default::default()
        }
    }

    pub(crate) fn adapter_exists(&self, adapter_id: u8) -> bool {
        self.adapters.get(adapter_id as usize).is_some()
    }

    pub(crate) fn vl_capability_view(
        &self,
        adapter_id: u8,
        virtual_lamp_id: u8,
    ) -> dali2rust_domain::registry::VirtualLampCapabilityView {
        let binding_short = self
            .lamps
            .get(&(adapter_id, virtual_lamp_id))
            .and_then(|lamp| lamp.binding_short);
        let capabilities = binding_short
            .and_then(|short| self.physical_devices.get(&(adapter_id, short)))
            .map_or_else(super::conversions::unbound_capabilities_view, |boxed| {
                let rec = boxed.as_ref();
                super::views::effective_capabilities_view(rec, rec.effective_color_mode())
            });
        dali2rust_domain::registry::VirtualLampCapabilityView {
            virtual_lamp_id,
            binding_short,
            capabilities,
        }
    }

    pub(crate) fn vl_capabilities(
        &self,
        adapter_id: u8,
        virtual_lamp_id: u8,
    ) -> dali2rust_domain::registry::CapabilityFlagsView {
        self.vl_capability_view(adapter_id, virtual_lamp_id)
            .capabilities
    }
}

pub struct RegistryStore {
    pub(crate) inner: RwLock<Inner>,
    pub(crate) dirty: DirtyFlags,
    pub(crate) persist_counters: PersistenceCounters,
    pub(crate) flush_buf: std::sync::Mutex<Vec<u8>>,
}

impl RegistryStore {
    pub fn persistence_counters(&self) -> &PersistenceCounters {
        &self.persist_counters
    }

    pub fn display_sample_parts(&self, adapter_id: u8) -> DisplaySampleParts {
        let g = self.read_inner();
        DisplaySampleParts {
            counts: count_for_display(&g, adapter_id),
            poller_enabled: g.poller_settings.enabled,
            poller_interval_ms: g.poller_settings.interval_ms,
            adapter_enabled: g
                .adapters
                .get(adapter_id as usize)
                .is_some_and(|r| r.enabled),
            redundancy_enabled: g.redundancy_settings.enabled,
            controller_active: g.dali_settings.application_active,
        }
    }

    pub fn display_counts(&self, adapter_id: u8) -> DisplayCounts {
        count_for_display(&self.read_inner(), adapter_id)
    }
}

fn count_for_display(g: &Inner, adapter_id: u8) -> DisplayCounts {
    {
        let mut counts = DisplayCounts::default();
        for ((aid, _), rec) in g.physical_devices.iter() {
            if *aid != adapter_id {
                continue;
            }
            counts.gear_known = counts.gear_known.saturating_add(1);
            if rec
                .runtime
                .error
                .as_ref()
                .is_some_and(dali2rust_contracts::msg::CompactErrorPayload::is_device_absent)
            {
                counts.gear_unreachable = counts.gear_unreachable.saturating_add(1);
            }
            if rec
                .runtime
                .status_flags
                .as_ref()
                .is_some_and(|f| f.lamp_failure || f.gear_failure)
            {
                counts.gear_faulted = counts.gear_faulted.saturating_add(1);
            }
        }
        for ((aid, _), rec) in g.input_devices.iter() {
            if *aid != adapter_id {
                continue;
            }
            counts.input_known = counts.input_known.saturating_add(1);
            if rec.present {
                counts.input_present = counts.input_present.saturating_add(1);
            }
        }
        counts
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DisplaySampleParts {
    pub counts: DisplayCounts,
    pub poller_enabled: bool,
    pub poller_interval_ms: u32,
    pub adapter_enabled: bool,
    pub redundancy_enabled: bool,
    pub controller_active: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayCounts {
    pub gear_known: u8,
    pub gear_unreachable: u8,
    pub gear_faulted: u8,
    pub input_known: u8,
    pub input_present: u8,
}

pub struct PersistenceCounters {
    pub flush_success_total: std::sync::atomic::AtomicU32,
    pub flush_error_total: std::sync::atomic::AtomicU32,
    pub no_space_total: std::sync::atomic::AtomicU32,
    pub hydrate_loaded_total: std::sync::atomic::AtomicU32,
    pub hydrate_default_total: std::sync::atomic::AtomicU32,
    pub hydrate_error_total: std::sync::atomic::AtomicU32,
}

impl PersistenceCounters {
    pub fn new() -> Self {
        Self {
            flush_success_total: std::sync::atomic::AtomicU32::new(0),
            flush_error_total: std::sync::atomic::AtomicU32::new(0),
            no_space_total: std::sync::atomic::AtomicU32::new(0),
            hydrate_loaded_total: std::sync::atomic::AtomicU32::new(0),
            hydrate_default_total: std::sync::atomic::AtomicU32::new(0),
            hydrate_error_total: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

impl Default for PersistenceCounters {
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryStore {
    pub fn new() -> Self {
        Self::with_adapter_count(1)
    }

    pub fn with_adapter_count(adapter_count: u8) -> Self {
        let n = adapter_count.max(1) as usize;
        let mut adapters = Vec::with_capacity(n);
        for i in 0..n {
            let name = if i == 0 {
                fixed_text_64("Main DALI")
            } else if i == 1 {
                fixed_text_64("Aux DALI")
            } else {
                fixed_text_64(&format!("Adapter {i}"))
            };
            adapters.push(AdapterRow {
                name,
                enabled: true,
                commands: 0,
                timeouts: 0,
                errors: 0,
            });
        }
        Self {
            inner: RwLock::new(Inner {
                adapters,
                ..Inner::preallocated()
            }),
            dirty: DirtyFlags::new(),
            persist_counters: PersistenceCounters::new(),
            flush_buf: std::sync::Mutex::new(Vec::with_capacity(
                super::persistence_stream::FLUSH_CHUNK_BYTES,
            )),
        }
    }

    pub(crate) fn read_inner(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| {
            log::error!("registry lock poisoned by a previous panic; recovering");
            self.inner.clear_poison();
            e.into_inner()
        })
    }

    pub(crate) fn write_inner(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| {
            log::error!("registry lock poisoned by a previous panic; recovering");
            self.inner.clear_poison();
            e.into_inner()
        })
    }

    pub fn adapters_revision(&self) -> u32 {
        self.read_inner().adapters_revision
    }

    pub fn groups_metadata_revision(&self) -> u32 {
        self.read_inner()
            .groups_metadata_revision
    }

    pub fn scenes_metadata_revision(&self) -> u32 {
        self.read_inner()
            .scenes_metadata_revision
    }

    pub fn scenes_matrix_desired_revision(&self) -> u32 {
        self.read_inner()
            .scenes_matrix_desired_revision
    }

    pub fn group_matrix_revision(&self) -> u32 {
        self.read_inner().group_matrix_revision
    }

    pub fn virtual_lamps_metadata_revision(&self) -> u32 {
        self.read_inner()
            .virtual_lamps_metadata_revision
    }

    pub fn physical_devices_revision(&self) -> u32 {
        self.read_inner()
            .physical_devices_revision
    }
}

impl Default for RegistryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryReadPort for RegistryStore {
    fn application_controller_active(&self) -> bool {
        self.read_inner().dali_settings.application_active
    }

    fn virtual_lamp_snapshot(
        &self,
        adapter_id: u8,
        virtual_lamp_id: u8,
    ) -> dali2rust_domain::registry::VirtualLampSnapshot {
        self.internal_virtual_lamp_snapshot(adapter_id, virtual_lamp_id)
    }

    fn virtual_lamp_binding_short(&self, adapter_id: u8, virtual_lamp_id: u8) -> Option<u8> {
        self.internal_virtual_lamp_binding_short(adapter_id, virtual_lamp_id)
    }

    fn adapter_snapshot(&self, adapter_id: u8) -> dali2rust_domain::registry::AdapterSnapshot {
        self.internal_adapter_snapshot(adapter_id)
    }

    fn known_physical_short_addresses(&self, adapter_id: u8) -> Vec<u8> {
        self.internal_known_physical_short_addresses(adapter_id)
    }

    fn first_free_short_address(&self, adapter_id: u8) -> Option<u8> {
        let g = self.read_inner();
        g.adapters.get(adapter_id as usize)?;

        let mut used = [false; MAX_SHORT_ADDRESSES];
        for (aid, sa) in g.physical_devices.keys() {
            if *aid == adapter_id && (*sa as usize) < MAX_SHORT_ADDRESSES {
                used[*sa as usize] = true;
            }
        }

        used.iter().position(|taken| !*taken).map(|idx| idx as u8)
    }

    fn physical_dt8_gear_features(&self, adapter_id: u8, short_address: u8) -> Option<u8> {
        self.read_inner()
            .physical_devices
            .get(&(adapter_id, short_address))?
            .attributes
            .dt8_color
            .gear_features
            .as_ref()
            .map(|observed| observed.value)
    }

    fn physical_dt8_auto_activation_repair_allowed(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> bool {
        self.dt8_permission_allowed(
            adapter_id,
            short_address,
            |s| s.dt8_auto_activation_repair,
            |rec| rec.dt8_auto_activation_repair,
        )
    }

    fn apply_on_discovery_armed(&self, _adapter_id: u8) -> bool {
        let view = <Self as dali2rust_domain::registry::PoliciesReadPort>::policies_view(self);
        view.apply_on_discovery && view.manages_anything()
    }

    fn physical_dt8_rgbwaf_control_assert_allowed(&self, adapter_id: u8, short_address: u8) -> bool {
        self.dt8_permission_allowed(
            adapter_id,
            short_address,
            |s| s.dt8_rgbwaf_control_assert,
            |rec| rec.dt8_rgbwaf_control_assert,
        )
    }
}

impl RegistryStore {
    fn dt8_permission_allowed(
        &self,
        adapter_id: u8,
        short_address: u8,
        global: impl FnOnce(&super::dali_settings::DaliSettingsRecord) -> bool,
        per_device: impl FnOnce(&super::physical_devices::PhysicalDeviceRecord) -> Option<bool>,
    ) -> bool {
        let g = self.read_inner();
        if !global(&g.dali_settings) {
            return false;
        }
        g.physical_devices
            .get(&(adapter_id, short_address))
            .and_then(|rec| per_device(rec))
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn read_inner_recovers_and_clears_poison() {
        let store = Arc::new(RegistryStore::new());
        let bumped = {
            let mut g = store.write_inner();
            g.adapters_revision += 1;
            g.adapters_revision
        };

        let poisoner = Arc::clone(&store);
        let handle = std::thread::spawn(move || {
            let _g = poisoner.inner.write().expect("registry lock");
            panic!("poison the registry lock");
        });
        assert!(handle.join().is_err(), "writer thread must panic");
        assert!(store.inner.read().is_err(), "lock must be poisoned");

        assert_eq!(store.read_inner().adapters_revision, bumped);
        assert!(store.inner.read().is_ok(), "clear_poison must heal the lock");
    }
}

