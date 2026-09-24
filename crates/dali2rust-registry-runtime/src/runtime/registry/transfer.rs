use dali2rust_platform::slice_store::{SliceKey, SliceStore, StoreError};

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
        keys.push(SliceKey::PhysicalDevices { adapter_id });
        for bank in 0..SliceKey::PHYSICAL_DEVICE_BANKS {
            keys.push(SliceKey::PhysicalDeviceBank { adapter_id, bank });
        }
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

    pub fn import_slice(
        &self,
        slices: &dyn SliceStore,
        key: SliceKey,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        if dali2rust_platform::flash_gate::firmware_write_open() {
            return Err(StoreError::Backend("firmware update in progress".to_string()));
        }
        let _serialised = self.flush_buf.lock().map_err(|_| {
            StoreError::Backend("flush buffer poisoned".to_string())
        })?;
        let mut session = slices.begin_write(key)?;
        if let Err(e) = session.append(bytes) {
            session.abort();
            return Err(e);
        }
        session.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let pd = keys
            .iter()
            .position(|k| matches!(k, SliceKey::PhysicalDevices { .. }))
            .expect("physical devices");
        let vl = keys
            .iter()
            .position(|k| matches!(k, SliceKey::VirtualLamps { .. }))
            .expect("virtual lamps");
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
