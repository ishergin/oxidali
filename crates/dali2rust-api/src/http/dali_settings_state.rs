use dali2rust_domain::registry::{BusUnitSetter, DaliSettingsView};
use serde::Serialize;

use crate::http::settings_surface::declare_settings_http_surface;

#[derive(Clone, Debug, Serialize)]
pub struct DaliSettingsDto {
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub application_active: bool,
    pub device_short_address: Option<u8>,
    pub power_cycle_notification: bool,
    pub random_address: u32,
    pub device_groups: Vec<u8>,
    pub operating_mode: u8,
    pub set_by: BusUnitSetByDto,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BusUnitSetByDto {
    pub application_active: Option<&'static str>,
    pub device_short_address: Option<&'static str>,
    pub power_cycle_notification: Option<&'static str>,
    pub random_address: Option<&'static str>,
    pub device_groups: Option<&'static str>,
    pub operating_mode: Option<&'static str>,
}

pub const SET_BY_OPERATOR: &str = "operator";
pub const SET_BY_WIRE: &str = "wire";
const DEVICE_GROUPS: u8 = 32;

const fn setter_name(setter: BusUnitSetter) -> Option<&'static str> {
    match setter {
        BusUnitSetter::Unrecorded => None,
        BusUnitSetter::Operator => Some(SET_BY_OPERATOR),
        BusUnitSetter::Wire => Some(SET_BY_WIRE),
    }
}

fn group_numbers(mask: u32) -> Vec<u8> {
    (0..DEVICE_GROUPS).filter(|group| mask & (1 << group) != 0).collect()
}

declare_settings_http_surface! {
    dto: DaliSettingsDto,
    read_port: DaliSettingsReadPort,
    view_fn: dali_settings_view,
    view_to_dto: dali_settings_view_to_dto,
    watch_port: DaliSettingsApplyWatchPort,
    watch_load: dali_settings_applied_load,
    http_state: DaliSettingsHttpState,
    dto_fn: dali_settings_dto,
    apply_watch: DaliSettingsApplyWatch,
    read_bridge: DaliSettingsHttpStateBridge,
    watch_bridge: DaliSettingsApplyWatchBridge,
}

pub fn dali_settings_view_to_dto(view: DaliSettingsView) -> DaliSettingsDto {
    DaliSettingsDto {
        dt8_auto_activation_repair: view.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: view.dt8_rgbwaf_control_assert,
        application_active: view.application_active,
        device_short_address: view.device_short_address,
        power_cycle_notification: view.power_cycle_notification,
        random_address: view.random_address,
        device_groups: group_numbers(view.device_groups),
        operating_mode: view.operating_mode,
        set_by: BusUnitSetByDto {
            application_active: setter_name(view.set_by.application_active),
            device_short_address: setter_name(view.set_by.short_address),
            power_cycle_notification: setter_name(view.set_by.power_cycle_notification),
            random_address: setter_name(view.set_by.random_address),
            device_groups: setter_name(view.set_by.device_groups),
            operating_mode: setter_name(view.set_by.operating_mode),
        },
    }
}
