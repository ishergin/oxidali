use crate::compiler::NameResolver;
use crate::refs::{DeviceRef, GroupRef, InputDeviceRef, LampRef, SceneRef};

fn fnv1a(name: &str) -> u32 {
    const FNV_OFFSET: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;
    let mut hash = FNV_OFFSET;
    for byte in name.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[derive(Debug, Clone)]
pub struct StubResolver {
    primary: u8,
    adapters: Vec<u8>,
    permissive: bool,
    lamps: Vec<(String, LampRef)>,
    groups: Vec<(String, GroupRef)>,
    devices: Vec<(String, DeviceRef)>,
    input_devices: Vec<(String, InputDeviceRef)>,
    scenes: Vec<(String, SceneRef)>,
}

impl StubResolver {
    fn new(permissive: bool) -> StubResolver {
        StubResolver {
            primary: 0,
            adapters: vec![0],
            permissive,
            lamps: Vec::new(),
            groups: Vec::new(),
            devices: Vec::new(),
            input_devices: Vec::new(),
            scenes: Vec::new(),
        }
    }

    pub fn permissive() -> StubResolver {
        StubResolver::new(true)
    }

    pub fn strict() -> StubResolver {
        StubResolver::new(false)
    }

    pub fn with_adapter(mut self, adapter_id: u8) -> Self {
        if !self.adapters.contains(&adapter_id) {
            self.adapters.push(adapter_id);
        }
        self
    }

    pub fn with_lamp(mut self, name: &str, lamp: LampRef) -> Self {
        self.lamps.push((name.into(), lamp));
        self
    }

    pub fn with_group(mut self, name: &str, group: GroupRef) -> Self {
        self.groups.push((name.into(), group));
        self
    }

    pub fn with_device(mut self, name: &str, device: DeviceRef) -> Self {
        self.devices.push((name.into(), device));
        self
    }

    pub fn with_input_device(mut self, name: &str, device: InputDeviceRef) -> Self {
        self.input_devices.push((name.into(), device));
        self
    }

    pub fn with_scene(self, name: &str, scene: u8) -> Self {
        let adapter_id = self.primary;
        self.with_scene_on(name, SceneRef { adapter_id, id: scene })
    }

    pub fn with_scene_on(mut self, name: &str, scene: SceneRef) -> Self {
        self.scenes.push((name.into(), scene));
        self
    }

    fn lookup<T: Copy>(&self, entries: &[(String, T)], name: &str, invent: impl FnOnce() -> T) -> Vec<T> {
        let found: Vec<T> = entries.iter().filter(|(n, _)| n == name).map(|(_, v)| *v).collect();
        if found.is_empty() && self.permissive {
            return vec![invent()];
        }
        found
    }
}

impl NameResolver for StubResolver {
    fn primary_adapter(&self) -> u8 {
        self.primary
    }

    fn adapter_exists(&self, adapter_id: u8) -> bool {
        self.adapters.contains(&adapter_id)
    }

    fn resolve_lamp(&self, name: &str) -> Vec<LampRef> {
        self.lookup(&self.lamps, name, || LampRef {
            adapter_id: self.primary,
            id: (fnv1a(name) & 0x03FF) as u16,
        })
    }

    fn resolve_group(&self, name: &str) -> Vec<GroupRef> {
        self.lookup(&self.groups, name, || GroupRef {
            adapter_id: self.primary,
            id: (fnv1a(name) & 0x00FF) as u16,
        })
    }

    fn resolve_device(&self, name: &str) -> Vec<DeviceRef> {
        self.lookup(&self.devices, name, || DeviceRef {
            adapter_id: self.primary,
            short_address: (fnv1a(name) & 0x3F) as u8,
        })
    }

    fn resolve_input_device(&self, name: &str) -> Vec<InputDeviceRef> {
        self.lookup(&self.input_devices, name, || InputDeviceRef {
            adapter_id: self.primary,
            device_short_address: (fnv1a(name) & 0x3F) as u8,
        })
    }

    fn resolve_scene(&self, name: &str) -> Vec<SceneRef> {
        self.lookup(&self.scenes, name, || SceneRef {
            adapter_id: self.primary,
            id: (fnv1a(name) & 0x0F) as u8,
        })
    }
}
