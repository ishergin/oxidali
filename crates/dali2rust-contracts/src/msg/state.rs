use serde::{Deserialize, Serialize};

use super::errors::CompactErrorPayload;
use super::kinds::{ColorMode, LastDapcSource, PowerState, RuntimeSource};

pub const MAX_EXTENDED_VERSIONS: usize = 8;

// IEC 62386-102 §11.6.2
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedVersionEntry {
    pub device_type: u8,
    pub version_number: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusFlags {
    pub raw: u8,
    pub lamp_failure: bool,
    pub gear_failure: bool,
    pub lamp_on: bool,
    pub limit_error: bool,
    pub fade_running: bool,
    pub reset_state: bool,
    pub missing_short_address: bool,
    pub power_cycle_seen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureStatus {
    pub raw: u8,
    pub lamp_failure: bool,
    pub gear_failure: bool,
    pub communication_failure: bool,
    pub source: RuntimeSource,
}

impl Default for FailureStatus {
    fn default() -> Self {
        Self {
            raw: 0,
            lamp_failure: false,
            gear_failure: false,
            communication_failure: false,
            source: RuntimeSource::Poller,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ColorValue {
    pub mode: ColorMode,
    pub color_temperature_kelvin: u16,
    pub x: u16,
    pub y: u16,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub w: u8,
    pub a: u8,
    pub f: u8,
}

impl ColorValue {
    pub fn cct(&self) -> Option<u16> {
        (self.mode == ColorMode::Cct && self.color_temperature_kelvin > 0)
            .then_some(self.color_temperature_kelvin)
    }

    pub fn xy_wire(&self) -> Option<(u16, u16)> {
        (self.mode == ColorMode::Xy && (self.x > 0 || self.y > 0)).then_some((self.x, self.y))
    }

    pub fn rgb_channels(&self) -> Option<(u8, u8, u8)> {
        matches!(self.mode, ColorMode::Rgb | ColorMode::Rgbwaf).then_some((self.r, self.g, self.b))
    }

    pub fn waf_channels(&self) -> Option<(u8, u8, u8)> {
        (self.mode == ColorMode::Rgbwaf).then_some((self.w, self.a, self.f))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level {
    pub raw_level: u8,
    pub source: RuntimeSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightSetpoint {
    pub power: PowerState,
    pub level: u8,
    pub color: Option<ColorValue>,
}

impl Default for LightSetpoint {
    fn default() -> Self {
        Self {
            power: PowerState::Unknown,
            level: 0,
            color: None,
        }
    }
}

impl PowerState {
    pub const fn for_level(level: u8) -> Self {
        if level > 0 {
            Self::On
        } else {
            Self::Off
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetpointDimensions {
    pub level: bool,
    pub color: bool,
}

impl SetpointDimensions {
    pub const fn intersects(self, other: Self) -> bool {
        (self.level && other.level) || (self.color && other.color)
    }
}

impl LightSetpoint {
    pub fn states_a_value(&self) -> bool {
        self.power != PowerState::Unknown || self.level > 0 || self.color.is_some()
    }

    pub fn merge_from(&mut self, next: &Self) {
        if next.power != PowerState::Unknown {
            self.power = next.power;
            self.level = next.level;
        } else if next.level > 0 {
            self.level = next.level;
        }
        if next.states_color() || (next.color.is_some() && !self.states_color()) {
            self.color = next.color;
        }
    }

    pub const fn from_level(level: u8, color: Option<ColorValue>) -> Self {
        Self {
            power: PowerState::for_level(level),
            level,
            color,
        }
    }

    pub const fn commanded_power(&self) -> Option<bool> {
        match self.power {
            PowerState::Off => Some(false),
            PowerState::On => Some(true),
            _ if self.level > 0 => Some(true),
            _ => None,
        }
    }

    pub const fn dimensions(&self) -> SetpointDimensions {
        SetpointDimensions {
            level: self.states_level(),
            color: self.states_color(),
        }
    }

    pub const fn states_level(&self) -> bool {
        self.commanded_power().is_some()
    }

    pub const fn states_color(&self) -> bool {
        match &self.color {
            Some(color) => color.mode.states_a_colour(),
            None => false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeObservation {
    pub status_flags: Option<StatusFlags>,
    pub failure_status: Option<FailureStatus>,
    pub value_source: Option<RuntimeSource>,
    pub last_seen_ms: Option<u64>,
    pub last_dapc_source: LastDapcSource,
    pub error: Option<CompactErrorPayload>,
}

impl RuntimeObservation {
    pub fn timestamped(source: RuntimeSource, last_seen_ms: u64) -> Self {
        Self {
            status_flags: None,
            failure_status: None,
            value_source: Some(source),
            last_seen_ms: Some(last_seen_ms),
            last_dapc_source: LastDapcSource::default(),
            error: None,
        }
    }

    pub fn api_timestamped(last_seen_ms: u64) -> Self {
        Self::timestamped(RuntimeSource::Api, last_seen_ms)
    }

    pub fn sniffer_timestamped(last_seen_ms: u64) -> Self {
        Self::timestamped(RuntimeSource::Sniffer, last_seen_ms)
    }

    #[must_use]
    pub fn device_absent(source: RuntimeSource) -> Self {
        Self {
            status_flags: None,
            failure_status: None,
            value_source: Some(source),
            last_seen_ms: None,
            last_dapc_source: LastDapcSource::default(),
            error: Some(CompactErrorPayload::new(
                crate::msg::ErrorCode::DeviceAbsent,
                "no answer",
            )),
        }
    }

    #[must_use]
    pub fn reports_absence(&self) -> bool {
        self.error.as_ref().is_some_and(CompactErrorPayload::is_device_absent)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityFlags {
    pub brightness: bool,
    pub cct: bool,
    pub xy: bool,
    pub rgb: bool,
    pub rgbwaf: bool,
    pub scenes: bool,
    pub groups: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRow {
    pub included: bool,
    pub setpoint: Option<LightSetpoint>,
    pub capabilities: CapabilityFlags,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dt6ReadSnapshot {
    pub gear_type: Option<u8>,
    pub dimming_curve: Option<u8>,
    pub possible_operating_mode: Option<u8>,
    pub features: Option<u8>,
    pub failure_status: Option<u8>,
    pub short_circuit: Option<u8>,
    pub open_circuit: Option<u8>,
    pub load_decrease: Option<u8>,
    pub load_increase: Option<u8>,
    pub current_protector_active: Option<u8>,
    pub thermal_shutdown: Option<u8>,
    pub thermal_overload: Option<u8>,
    pub reference_running: Option<u8>,
    pub reference_measurement_failed: Option<u8>,
    pub current_protector_enabled: Option<u8>,
    pub operating_mode: Option<u8>,
    pub fast_fade_time: Option<u8>,
    pub min_fast_fade_time: Option<u8>,
    pub extended_version_number: Option<u8>,
}

