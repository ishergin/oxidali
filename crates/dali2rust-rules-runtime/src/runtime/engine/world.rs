use dali2rust_rules_model::LightTarget;

pub type HclTargetKey = LightTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallTime {
    pub minutes_of_day: u16,
    pub weekday: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SunTimes {
    pub sunrise_min: u16,
    pub sunset_min: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampState {
    pub adapter_id: u8,
    pub id: u16,
    pub is_on: bool,
    pub level: u8,
    pub cct_kelvin: Option<u16>,
    pub last_level: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupState {
    pub adapter_id: u8,
    pub id: u16,
    pub any_on: bool,
    pub all_off: bool,
    pub member_count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceState {
    pub adapter_id: u8,
    pub short_address: u8,
    pub online: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputState {
    pub adapter_id: u8,
    pub short_address: u8,
    pub instance_number: u8,
    pub occupied: Option<bool>,
    pub light: Option<u16>,
    pub position: Option<u16>,
    pub last_event_age_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HclTargetState {
    pub target: HclTargetKey,
    pub enabled: bool,
    pub overridden: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WorldSnapshot {
    pub now_ms: u64,
    pub wall: Option<WallTime>,
    pub sun: Option<SunTimes>,
    pub controller_active: bool,
    pub lamps: Vec<LampState>,
    pub groups: Vec<GroupState>,
    pub devices: Vec<DeviceState>,
    pub inputs: Vec<InputState>,
    pub hcl: Vec<HclTargetState>,
}

impl WorldSnapshot {
    #[must_use]
    pub fn empty(now_ms: u64) -> WorldSnapshot {
        WorldSnapshot {
            now_ms,
            controller_active: true,
            ..WorldSnapshot::default()
        }
    }

    pub(crate) fn lamp(&self, adapter_id: u8, id: u16) -> Option<&LampState> {
        self.lamps
            .iter()
            .find(|l| l.adapter_id == adapter_id && l.id == id)
    }

    pub(crate) fn group(&self, adapter_id: u8, id: u16) -> Option<&GroupState> {
        self.groups
            .iter()
            .find(|g| g.adapter_id == adapter_id && g.id == id)
    }

    pub(crate) fn device(&self, adapter_id: u8, short_address: u8) -> Option<&DeviceState> {
        self.devices
            .iter()
            .find(|d| d.adapter_id == adapter_id && d.short_address == short_address)
    }

    pub(crate) fn input(&self, adapter_id: u8, sa: u8, inst: u8) -> Option<&InputState> {
        self.inputs.iter().find(|i| {
            i.adapter_id == adapter_id && i.short_address == sa && i.instance_number == inst
        })
    }

    pub(crate) fn hcl_target(&self, target: &HclTargetKey) -> Option<&HclTargetState> {
        self.hcl.iter().find(|h| h.target == *target)
    }

    pub(crate) fn any_lamp_on(&self, adapter_id: u8) -> bool {
        self.lamps.iter().any(|l| l.adapter_id == adapter_id && l.is_on)
    }
}
