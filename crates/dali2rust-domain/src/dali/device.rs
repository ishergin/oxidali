pub const SCENE_NOT_SET: u8 = 0xFF;
const SHORT_ADDRESS_NONE: u8 = 0xFF;

pub const MAX_ARC_POWER_LEVEL: u8 = 254;

// IEC 62386-102 §11.5.20
pub const ACTUAL_LEVEL_MASK: u8 = 0xFF;

pub const ARC_POWER_LEVEL_MASK: u8 = 0xFF;

const DEFAULT_TOP_LEVEL: u8 = MAX_ARC_POWER_LEVEL;
const DALI_SCENE_COUNT_DEV: usize = 16;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Device {
    pub id: i64,
    pub short_address: u8,
    pub level: u8,
    pub status: u8,
    pub min_level: u8,
    pub max_level: u8,
    pub failure_status: u8,
    pub groups: u16,
    #[serde(default = "default_scenes")]
    pub scenes: [u8; DALI_SCENE_COUNT_DEV],
    pub power_on_level: u8,
    pub system_failure_level: u8,
    pub fade_time: u8,
    pub fade_rate: u8,
    pub device_type: u8,
    pub version_number: u8,
    pub physical_minimum: u8,
    #[serde(default)]
    pub color_type: u8,
    #[serde(default)]
    pub color_temperature: u16,
    #[serde(default)]
    pub color_value_x: u16,
    #[serde(default)]
    pub color_value_y: u16,
}

fn default_scenes() -> [u8; DALI_SCENE_COUNT_DEV] {
    [SCENE_NOT_SET; DALI_SCENE_COUNT_DEV]
}

impl Default for Device {
    fn default() -> Self {
        Self {
            id: 0,
            short_address: SHORT_ADDRESS_NONE,
            level: 0,
            status: 0,
            min_level: 0,
            max_level: DEFAULT_TOP_LEVEL,
            failure_status: 0,
            groups: 0,
            scenes: default_scenes(),
            power_on_level: 0,
            system_failure_level: DEFAULT_TOP_LEVEL,
            fade_time: 0,
            fade_rate: 0,
            device_type: 0,
            version_number: 0,
            physical_minimum: 0,
            color_type: 0,
            color_temperature: 0,
            color_value_x: 0,
            color_value_y: 0,
        }
    }
}
