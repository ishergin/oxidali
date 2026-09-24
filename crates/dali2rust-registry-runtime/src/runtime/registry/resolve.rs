use super::store::RegistryStore;

impl RegistryStore {
    pub fn virtual_lamp_by_name(&self, name: &str) -> Option<(u8, u8)> {
        let g = self.read_inner();
        g.lamps
            .iter()
            .find(|(_, rec)| rec.name.as_str() == name)
            .map(|((adapter, id), _)| (*adapter, *id))
    }

    pub fn group_by_name(&self, name: &str) -> Option<(u8, u8)> {
        let g = self.read_inner();
        g.groups
            .iter()
            .find(|(_, rec)| rec.name.as_str() == name)
            .map(|((adapter, id), _)| (*adapter, *id))
    }

    pub fn physical_device_by_name(&self, name: &str) -> Option<(u8, u8)> {
        let g = self.read_inner();
        g.physical_devices
            .iter()
            .find(|(_, rec)| rec.name.as_str() == name)
            .map(|((adapter, short), _)| (*adapter, *short))
    }

    pub fn input_device_by_name(&self, name: &str) -> Option<(u8, u8)> {
        let g = self.read_inner();
        g.input_devices
            .iter()
            .find(|(_, rec)| rec.name.as_deref() == Some(name))
            .map(|((adapter, short), _)| (*adapter, *short))
    }

    pub fn scene_by_name(&self, name: &str) -> Option<u8> {
        let g = self.read_inner();
        g.scenes
            .iter()
            .find(|(_, rec)| rec.name.as_str() == name)
            .map(|((_, scene_id), _)| *scene_id)
    }

    pub fn adapter_id_exists(&self, adapter_id: u8) -> bool {
        usize::from(adapter_id) < self.read_inner().adapters.len()
    }
}
