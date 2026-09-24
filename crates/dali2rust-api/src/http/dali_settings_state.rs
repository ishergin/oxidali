use dali2rust_domain::registry::DaliSettingsView;
use serde::Serialize;

use crate::http::settings_surface::declare_settings_http_surface;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct DaliSettingsDto {
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub application_active: bool,
    pub device_short_address: Option<u8>,
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
    }
}
