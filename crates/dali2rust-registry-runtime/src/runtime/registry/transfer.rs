use std::sync::atomic::Ordering;

use dali2rust_platform::slice_store::{SliceKey, SliceStore, StoreError};

use super::import_stage::StagedSlice;
use super::persistence::validate_registry_slice;
use super::store::RegistryStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceManifestRow {
    pub key: SliceKey,
    pub name: String,
    pub bytes: Option<usize>,
    pub crc32: u32,
}

#[must_use]
pub fn transferable_slices(adapter_count: u8) -> Vec<SliceKey> {
    let mut keys = vec![
        SliceKey::ControllerSettings,
        SliceKey::HclSchedules,
        SliceKey::PollerSettings,
    ];
    keys.push(SliceKey::Policies);
    keys.push(SliceKey::HomeAssistantSettings);
    for bank in 0..4u8 {
        keys.push(SliceKey::InputDevices { bank });
    }
    for bank in 0..4u8 {
        keys.push(SliceKey::Rules { bank });
    }
    for adapter_id in 0..adapter_count {
        for bank in 0..SliceKey::PHYSICAL_DEVICE_BANKS {
            keys.push(SliceKey::PhysicalDeviceBank { adapter_id, bank });
        }
        keys.push(SliceKey::PhysicalDevices { adapter_id });
        keys.push(SliceKey::Groups { adapter_id });
        keys.push(SliceKey::VirtualLamps { adapter_id });
        for scene_id in 0..16u8 {
            keys.push(SliceKey::Scene {
                adapter_id,
                scene_id,
            });
        }
    }
    keys
}

#[must_use]
pub fn slice_key_from_name(name: &str, adapter_count: u8) -> Option<SliceKey> {
    transferable_slices(adapter_count)
        .into_iter()
        .find(|key| key.label() == name)
}

impl RegistryStore {
    pub fn slice_manifest(&self, slices: &dyn SliceStore, adapter_count: u8) -> Vec<SliceManifestRow> {
        transferable_slices(adapter_count)
            .into_iter()
            .map(|key| {
                let blob = slices.load(key).ok();
                SliceManifestRow {
                    key,
                    name: key.label(),
                    bytes: blob.as_ref().map(Vec::len),
                    crc32: blob
                        .as_deref()
                        .map_or(0, dali2rust_platform::http_fetch::crc32),
                }
            })
            .collect()
    }

    pub fn export_slice(&self, slices: &dyn SliceStore, key: SliceKey) -> Option<Vec<u8>> {
        slices.load(key).ok()
    }
}

pub trait ForeignSliceOwner: Send + Sync {
    fn validate(&self, key: SliceKey, bytes: &[u8]) -> Result<(), String>;
    fn imported(&self, key: SliceKey);
}

pub struct RegistryOwnedOnly;

impl ForeignSliceOwner for RegistryOwnedOnly {
    fn validate(&self, key: SliceKey, _bytes: &[u8]) -> Result<(), String> {
        Err(format!("no owner reads {}", key.label()))
    }

    fn imported(&self, _key: SliceKey) {}
}

#[derive(Debug)]
pub enum ImportWriteFailure {
    FirmwareWriteOpen,
    Store { written: usize, error: StoreError },
}

impl RegistryStore {
    pub fn keep_decodable(
        &self,
        staged: Vec<StagedSlice>,
        foreign: &dyn ForeignSliceOwner,
    ) -> (Vec<StagedSlice>, u32) {
        let broken: Vec<SliceKey> = staged
            .iter()
            .filter(|slice| !decodes(slice, foreign))
            .map(|slice| slice.key)
            .collect();
        let (refused, decodable): (Vec<_>, Vec<_>) = staged
            .into_iter()
            .partition(|slice| broken.iter().any(|key| same_family(*key, slice.key)));
        let refused = u32::try_from(refused.len()).unwrap_or(u32::MAX);
        self.persist_counters.hydrate_error_total.fetch_add(refused, Ordering::Relaxed);
        (decodable, refused)
    }

    pub fn write_staged(
        &self,
        slices: &dyn SliceStore,
        staged: &[StagedSlice],
    ) -> Result<(), ImportWriteFailure> {
        if dali2rust_platform::flash_gate::firmware_write_open() {
            return Err(ImportWriteFailure::FirmwareWriteOpen);
        }
        for (written, slice) in staged.iter().enumerate() {
            write_slice_bytes(slices, slice).map_err(|error| {
                log::warn!("registry: writing imported {} failed: {error}", slice.key.label());
                ImportWriteFailure::Store { written, error }
            })?;
        }
        Ok(())
    }

    pub fn withhold_until_read(&self, keys: &[SliceKey], adapter_count: u8) {
        for key in keys {
            match *key {
                SliceKey::PhysicalDeviceBank { adapter_id, bank } => {
                    self.dirty.settle_physical_device_banks(adapter_id, 0, 1 << bank);
                }
                other => self.dirty.withheld.settle(other, true),
            }
        }
        self.persist_counters.hydrate_error_total.fetch_add(1, Ordering::Relaxed);
        self.persist_counters
            .unread_slices
            .store(self.unread_slices(adapter_count), Ordering::Relaxed);
    }
}

fn same_family(a: SliceKey, b: SliceKey) -> bool {
    a == b
        || matches!(
            (a, b),
            (SliceKey::Rules { .. }, SliceKey::Rules { .. })
                | (SliceKey::InputDevices { .. }, SliceKey::InputDevices { .. })
        )
}

fn decodes(slice: &StagedSlice, foreign: &dyn ForeignSliceOwner) -> bool {
    let checked = match validate_registry_slice(slice.key, &slice.bytes) {
        Some(checked) => checked.map_err(|e| e.to_string()),
        None => foreign.validate(slice.key, &slice.bytes),
    };
    if let Err(why) = &checked {
        log::warn!("registry: import of {} refused: {why}", slice.key.label());
    }
    checked.is_ok()
}

fn write_slice_bytes(slices: &dyn SliceStore, slice: &StagedSlice) -> Result<(), StoreError> {
    let mut session = slices.begin_write(slice.key)?;
    if let Err(e) = session.append(&slice.bytes) {
        session.abort();
        return Err(e);
    }
    session.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bank_family_is_refused_whole_when_one_of_its_banks_does_not_decode() {
        let store = RegistryStore::with_adapter_count(1);
        let empty_bank = super::super::persistence_slices::encode_persistence_blob(
            &super::super::persistence_slices::PersistenceEnvelope::new(
                super::super::persistence_slices::INPUT_DEVICES_SLICE_VERSION,
                super::super::input_devices::PersistableInputDevicesSlice::default(),
            ),
        )
        .expect("encode bank");
        let staged = vec![
            StagedSlice { key: SliceKey::InputDevices { bank: 0 }, bytes: empty_bank.clone() },
            StagedSlice { key: SliceKey::InputDevices { bank: 1 }, bytes: b"torn".to_vec() },
            StagedSlice { key: SliceKey::InputDevices { bank: 2 }, bytes: empty_bank },
        ];
        let (kept, refused) = store.keep_decodable(staged, &RegistryOwnedOnly);
        assert!(kept.is_empty(), "a bank is a position in one list; half a list is no list");
        assert_eq!(refused, 3);
    }

    #[test]
    fn a_slice_written_but_not_reloaded_is_held_from_the_flush_until_a_read() {
        let slices = dali2rust_bsp::slice_store_files::InMemorySliceStore::new();
        let store = RegistryStore::with_adapter_count(1);
        let imported = [StagedSlice { key: SliceKey::Policies, bytes: b"imported".to_vec() }];
        store.write_staged(&slices, &imported).expect("write");
        store.withhold_until_read(&[SliceKey::Policies], 1);
        assert_eq!(store.persist_counters.unread_slices.load(Ordering::Relaxed), 1);

        store.dirty.mark_policies_dirty();
        store.flush_dirty_slices(&slices);
        assert_eq!(slices.load(SliceKey::Policies).expect("slice"), b"imported");
    }

    #[test]
    fn the_three_per_unit_slices_are_never_transferred() {
        let keys = transferable_slices(1);
        assert!(!keys.contains(&SliceKey::DaliSettings));
        assert!(!keys.contains(&SliceKey::RedundancySettings));
        assert!(!keys.contains(&SliceKey::Adapters));
        assert_eq!(keys.first(), Some(&SliceKey::ControllerSettings));
        assert!(keys.contains(&SliceKey::HomeAssistantSettings));
    }


    #[test]
    fn physical_devices_come_before_the_bindings_that_depend_on_them() {
        let keys = transferable_slices(1);
        let last_bank = keys
            .iter()
            .rposition(|k| matches!(k, SliceKey::PhysicalDeviceBank { .. }))
            .expect("physical device banks");
        let pd = keys
            .iter()
            .position(|k| matches!(k, SliceKey::PhysicalDevices { .. }))
            .expect("physical devices");
        let vl = keys
            .iter()
            .position(|k| matches!(k, SliceKey::VirtualLamps { .. }))
            .expect("virtual lamps");
        assert!(
            last_bank < pd,
            "the banks arrive before the whole-adapter slot, so its reload finds them loaded"
        );
        assert!(pd < vl, "physical devices must hydrate first");
    }

    #[test]
    fn a_name_round_trips_to_its_key() {
        for key in transferable_slices(2) {
            assert_eq!(slice_key_from_name(&key.label(), 2), Some(key));
        }
    }

    #[test]
    fn a_name_outside_the_transferable_set_resolves_to_nothing() {
        assert_eq!(slice_key_from_name("dali_settings", 1), None);
        assert_eq!(slice_key_from_name("nonsense", 1), None);
    }

    #[test]
    fn every_adapter_gets_its_own_scene_block() {
        assert_eq!(
            transferable_slices(2).len(),
            transferable_slices(1).len() + 35
        );
    }
}
