use std::collections::HashMap;

use super::store::RegistryStore;

fn keys_named<V>(rows: &HashMap<(u8, u8), V>, named: impl Fn(&V) -> bool) -> Vec<(u8, u8)> {
    let mut keys: Vec<(u8, u8)> = rows
        .iter()
        .filter(|(_, row)| named(row))
        .map(|(key, _)| *key)
        .collect();
    keys.sort_unstable();
    keys
}

impl RegistryStore {
    pub fn virtual_lamps_named(&self, name: &str) -> Vec<(u8, u8)> {
        keys_named(&self.read_inner().lamps, |rec| rec.name.as_str() == name)
    }

    pub fn groups_named(&self, name: &str) -> Vec<(u8, u8)> {
        keys_named(&self.read_inner().groups, |rec| rec.name.as_str() == name)
    }

    pub fn physical_devices_named(&self, name: &str) -> Vec<(u8, u8)> {
        keys_named(&self.read_inner().physical_devices, |rec| rec.name.as_str() == name)
    }

    pub fn input_devices_named(&self, name: &str) -> Vec<(u8, u8)> {
        keys_named(&self.read_inner().input_devices, |rec| rec.name.as_deref() == Some(name))
    }

    pub fn scenes_named(&self, name: &str) -> Vec<(u8, u8)> {
        keys_named(&self.read_inner().scenes, |rec| rec.name.as_str() == name)
    }

    pub fn adapter_id_exists(&self, adapter_id: u8) -> bool {
        usize::from(adapter_id) < self.read_inner().adapters.len()
    }
}
