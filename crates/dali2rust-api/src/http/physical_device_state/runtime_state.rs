use dali2rust_domain::registry::{
    CapabilityFlagsView, FailureStatusView, PhysicalDeviceStateView, RuntimeErrorView,
    StatusFlagsView,
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct StatusFlagsDto {
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

#[derive(Clone, Debug, Serialize)]
pub struct FailureStatusDto {
    pub raw: u8,
    pub lamp_failure: bool,
    pub gear_failure: bool,
    pub communication_failure: bool,
    pub source: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct RuntimeErrorDto {
    pub code: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct PhysicalDeviceStateDto {
    pub power: String,
    pub level: Option<u8>,
    pub color_mode: String,
    pub color_temperature_kelvin: Option<u16>,
    pub xy: Option<XyDto>,
    pub rgb: Option<RgbDto>,
    pub waf: Option<WafDto>,
    pub status: Option<StatusFlagsDto>,
    pub failure_status: Option<FailureStatusDto>,
    pub value_source: Option<String>,
    pub last_seen_ms: Option<u64>,
    pub last_dapc_source: Option<String>,
    pub error: Option<RuntimeErrorDto>,
}

pub const XY_FULL_SCALE: f64 = 65_535.0;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct XyDto {
    pub x: f64,
    pub y: f64,
}

impl XyDto {
    pub fn from_wire(x: u16, y: u16) -> Self {
        Self {
            x: f64::from(x) / XY_FULL_SCALE,
            y: f64::from(y) / XY_FULL_SCALE,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RgbDto {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct WafDto {
    pub w: u8,
    pub a: u8,
    pub f: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct CapabilityFlagsDto {
    pub brightness: bool,
    pub cct: bool,
    pub xy: bool,
    pub rgb: bool,
    pub rgbwaf: bool,
    pub scenes: bool,
    pub groups: bool,
}

impl From<&CapabilityFlagsDto> for dali2rust_domain::registry::CapabilityFlagsView {
    fn from(dto: &CapabilityFlagsDto) -> Self {
        Self {
            brightness: dto.brightness,
            cct: dto.cct,
            xy: dto.xy,
            rgb: dto.rgb,
            rgbwaf: dto.rgbwaf,
            scenes: dto.scenes,
            groups: dto.groups,
        }
    }
}

fn status_view_to_dto(status: Option<StatusFlagsView>) -> Option<StatusFlagsDto> {
    status.map(|sf| StatusFlagsDto {
        raw: sf.raw,
        lamp_failure: sf.lamp_failure,
        gear_failure: sf.gear_failure,
        lamp_on: sf.lamp_on,
        limit_error: sf.limit_error,
        fade_running: sf.fade_running,
        reset_state: sf.reset_state,
        missing_short_address: sf.missing_short_address,
        power_cycle_seen: sf.power_cycle_seen,
    })
}

fn failure_view_to_dto(status: Option<FailureStatusView>) -> Option<FailureStatusDto> {
    status.map(|fs| FailureStatusDto {
        raw: fs.raw,
        lamp_failure: fs.lamp_failure,
        gear_failure: fs.gear_failure,
        communication_failure: fs.communication_failure,
        source: fs.source.rest_name().to_string(),
    })
}

pub(super) fn runtime_error_dto(error: Option<RuntimeErrorView>) -> Option<RuntimeErrorDto> {
    let RuntimeErrorView { code } = error?;
    Some(RuntimeErrorDto {
        code: code.rest_name(),
    })
}

pub(crate) fn state_view_to_dto(state: PhysicalDeviceStateView) -> PhysicalDeviceStateDto {
    PhysicalDeviceStateDto {
        power: state.power,
        level: state.level,
        color_mode: state.color_mode,
        color_temperature_kelvin: state.color_temperature_kelvin,
        xy: state.xy.map(|(x, y)| XyDto { x, y }),
        rgb: state.rgb.map(|(r, g, b)| RgbDto { r, g, b }),
        waf: state.waf.map(|(w, a, f)| WafDto { w, a, f }),
        status: status_view_to_dto(state.status),
        failure_status: failure_view_to_dto(state.failure_status),
        value_source: state.value_source.map(|src| src.rest_name().to_string()),
        last_seen_ms: state.last_seen_ms,
        last_dapc_source: state.last_dapc_source.rest_name().map(str::to_string),
        error: runtime_error_dto(state.error),
    }
}

pub(crate) fn capabilities_view_to_dto(view: &CapabilityFlagsView) -> CapabilityFlagsDto {
    CapabilityFlagsDto {
        brightness: view.brightness,
        cct: view.cct,
        xy: view.xy,
        rgb: view.rgb,
        rgbwaf: view.rgbwaf,
        scenes: view.scenes,
        groups: view.groups,
    }
}
