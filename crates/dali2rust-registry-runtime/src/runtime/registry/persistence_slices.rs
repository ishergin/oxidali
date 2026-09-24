use dali2rust_domain::registry::{MemoryBankSummaryView, PhysicalDeviceAttributesView};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub const ADAPTERS_SLICE_VERSION: u32 = 9;
pub const GROUPS_SLICE_VERSION: u32 = 10;
pub const VIRTUAL_LAMPS_SLICE_VERSION: u32 = 9;
pub const PHYSICAL_DEVICES_SLICE_VERSION: u32 = 16;
pub const SCENES_SLICE_VERSION: u32 = 10;
pub const HCL_SCHEDULES_SLICE_VERSION: u32 = 9;
pub const POLLER_SETTINGS_SLICE_VERSION: u32 = 11;
pub const INPUT_DEVICES_SLICE_VERSION: u32 = 1;
pub const DALI_SETTINGS_SLICE_VERSION: u32 = 11;
pub const REDUNDANCY_SETTINGS_SLICE_VERSION: u32 = 2;
pub const POLICIES_SLICE_VERSION: u32 = 1;
pub const HOME_ASSISTANT_SETTINGS_SLICE_VERSION: u32 = 2;







#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistenceEnvelope<T> {
    pub version: u32,
    pub data: T,
}

impl<T> PersistenceEnvelope<T> {
    pub fn new(version: u32, data: T) -> Self {
        Self { version, data }
    }
}

pub fn encode_persistence_blob<T: Serialize + ?Sized>(
    value: &T,
) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(value)
}

pub fn decode_persistence_blob<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}

pub fn decode_versioned_slice<T: DeserializeOwned>(
    bytes: &[u8],
    expected: u32,
) -> Result<T, SliceDecodeError> {
    let (version, body) =
        postcard::take_from_bytes::<u32>(bytes).map_err(SliceDecodeError::Malformed)?;
    if version != expected {
        return Err(SliceDecodeError::Version { found: version, expected });
    }
    postcard::from_bytes(body).map_err(SliceDecodeError::Malformed)
}

#[derive(Debug)]
pub enum SliceDecodeError {
    Version { found: u32, expected: u32 },
    Malformed(postcard::Error),
}

impl core::fmt::Display for SliceDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Version { found, expected } => {
                write!(f, "unsupported persistence version: {found} (this build writes {expected})")
            }
            Self::Malformed(e) => write!(f, "malformed persistence blob: {e}"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableAdapterSlice {
    pub adapters: Vec<PersistableAdapterRow>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableAdapterRow {
    pub adapter_id: u8,
    pub name: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableHclSchedulesSlice {
    pub schedules: Vec<PersistableHclSchedule>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableHclSchedule {
    pub schedule_id: String,
    pub enabled: bool,
    pub algorithm: u8,
    pub active_days_mask: u8,
    pub latitude_microdeg: Option<i32>,
    pub longitude_microdeg: Option<i32>,
    pub targets: Vec<PersistableHclTarget>,
    pub points: Vec<PersistableHclPoint>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct PersistableHclTarget {
    pub adapter_id: u8,
    pub scope: u8,
    pub group_mask: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct PersistableHclPoint {
    pub time_ref: u8,
    pub offset_minutes: i16,
    pub level_mode: u8,
    pub level: Option<u8>,
    pub color_temperature_kelvin: Option<u16>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct PersistablePollerSettingsSlice {
    pub enabled: bool,
    pub interval_ms: u32,
    pub attribute_groups_mask: u8,
    pub include_dt8_color: bool,
    pub skip_unbound_virtual_lamps: bool,
    pub include_energy: bool,
    pub include_diagnostics: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct PersistableDaliSettingsSlice {
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub application_active: bool,
    pub device_short_address: Option<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableRedundancySettingsSlice {
    pub enabled: bool,
    pub standby_role: bool,
    pub probe_interval_ms: u32,
    pub takeover_after_missed: u8,
    pub boot_listen_ms: u32,
    pub peer_device_short_address: Option<u8>,
    pub peer_url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct PersistablePoliciesSlice {
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub apply_on_discovery: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableHomeAssistantSettingsSlice {
    pub enabled: bool,
    pub broker_host: String,
    pub broker_port: u16,
    pub broker_username: String,
    pub broker_password: String,
    pub discovery_prefix: String,
    pub state_topic_prefix: String,
    pub controller_id: String,
    pub publish_qos: u8,
    pub retain_state: bool,
    pub retain_discovery: bool,
    pub expose_input_devices: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableVirtualLampsSlice {
    pub adapter_id: u8,
    pub lamps: Vec<PersistableVlRecord>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableGroupsSlice {
    pub adapter_id: u8,
    pub groups: Vec<PersistableGroupRecord>,
    pub rows: Vec<PersistableGroupMatrixRow>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableGroupRecord {
    pub group_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableGroupMatrixRow {
    pub virtual_lamp_id: u8,
    pub desired_groups_mask: u16,
    pub desired_seeded: bool,
    pub desired_from_operator: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableSceneSlice {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub name: String,
    pub ha_select_enabled: bool,
    pub rows: Vec<PersistableSceneRow>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableSceneRow {
    pub virtual_lamp_id: u8,
    pub included: bool,
    pub desired_seeded: bool,
    pub power: Option<dali2rust_contracts::msg::PowerState>,
    pub level: Option<u8>,
    pub color: Option<dali2rust_contracts::msg::ColorValue>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistableVlRecord {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
    pub binding_short: Option<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistablePhysicalDevicesSlice {
    pub adapter_id: u8,
    pub devices: Vec<PersistablePhysicalDeviceRecord>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistablePhysicalDeviceRecord {
    pub short_address: u8,
    pub random_address: Option<u32>,
    pub name: String,
    pub notes: Option<String>,
    pub device_type_override: Option<dali2rust_contracts::msg::DeviceType>,
    pub color_mode_override: Option<dali2rust_contracts::msg::ColorMode>,
    pub attributes: PhysicalDeviceAttributesView,
    pub memory_banks: Vec<MemoryBankSummaryView>,
    pub device_type_discovered: dali2rust_contracts::msg::DeviceType,
    pub color_mode_discovered: dali2rust_contracts::msg::ColorMode,
    pub cap_cct: bool,
    pub cap_xy: bool,
    pub cap_rgb: bool,
    pub tc_coolest_mirek: Option<u16>,
    pub tc_warmest_mirek: Option<u16>,
    pub dt8_auto_activation_repair: Option<bool>,
    pub dt8_rgbwaf_control_assert: Option<bool>,
    pub cap_rgbwaf: bool,
    pub supported_device_types: Option<dali2rust_contracts::msg::DeviceTypeSet>,
}

#[cfg(test)]
mod colour_shape_tests {
    use super::{encode_persistence_blob, SCENES_SLICE_VERSION};
    use dali2rust_contracts::msg::{ColorMode, ColorValue};

    #[test]
    fn scene_colour_wire_shape_is_frozen() {
        const FROZEN_V10: &[u8] = &[
            0x05,
            0x8C, 0x15,
            0xE8, 0x07,
            0xD0, 0x0F,
            0x01, 0x02, 0x03,
            0x04, 0x05, 0x06,
        ];
        let sample = ColorValue {
            mode: ColorMode::Rgbwaf,
            color_temperature_kelvin: 2700,
            x: 1000,
            y: 2000,
            r: 1,
            g: 2,
            b: 3,
            w: 4,
            a: 5,
            f: 6,
        };
        assert_eq!(
            encode_persistence_blob(&sample).expect("encode colour"),
            FROZEN_V10,
            "ColorValue changed shape: bump SCENES_SLICE_VERSION (currently {SCENES_SLICE_VERSION}) \
             and re-freeze these bytes"
        );
    }
}
