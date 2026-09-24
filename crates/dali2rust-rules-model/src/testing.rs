use crate::compiler::NameResolver;
use crate::refs::{DeviceRef, GroupRef, InputDeviceRef, LampRef};

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
    scenes: Vec<(String, u8)>,
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

    pub fn with_scene(mut self, name: &str, scene: u8) -> Self {
        self.scenes.push((name.into(), scene));
        self
    }

    fn lookup<T: Copy>(entries: &[(String, T)], name: &str) -> Option<T> {
        entries.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }
}

impl NameResolver for StubResolver {
    fn primary_adapter(&self) -> u8 {
        self.primary
    }

    fn adapter_exists(&self, adapter_id: u8) -> bool {
        self.adapters.contains(&adapter_id)
    }

    fn resolve_lamp(&self, name: &str) -> Option<LampRef> {
        Self::lookup(&self.lamps, name).or_else(|| {
            self.permissive.then(|| LampRef {
                adapter_id: self.primary,
                id: (fnv1a(name) & 0x03FF) as u16,
            })
        })
    }

    fn resolve_group(&self, name: &str) -> Option<GroupRef> {
        Self::lookup(&self.groups, name).or_else(|| {
            self.permissive.then(|| GroupRef {
                adapter_id: self.primary,
                id: (fnv1a(name) & 0x00FF) as u16,
            })
        })
    }

    fn resolve_device(&self, name: &str) -> Option<DeviceRef> {
        Self::lookup(&self.devices, name).or_else(|| {
            self.permissive.then(|| DeviceRef {
                adapter_id: self.primary,
                short_address: (fnv1a(name) & 0x3F) as u8,
            })
        })
    }

    fn resolve_input_device(&self, name: &str) -> Option<InputDeviceRef> {
        Self::lookup(&self.input_devices, name).or_else(|| {
            self.permissive.then(|| InputDeviceRef {
                adapter_id: self.primary,
                device_short_address: (fnv1a(name) & 0x3F) as u8,
            })
        })
    }

    fn resolve_scene(&self, name: &str) -> Option<u8> {
        Self::lookup(&self.scenes, name)
            .or_else(|| self.permissive.then(|| (fnv1a(name) & 0x0F) as u8))
    }
}
