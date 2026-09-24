use dali2rust_domain::registry::RedundancySettingsView;
use serde::Serialize;

use crate::http::settings_surface::declare_settings_http_surface;

#[derive(Clone, Debug, Serialize)]
pub struct RedundancySettingsDto {
    pub enabled: bool,
    pub role: &'static str,
    pub probe_interval_ms: u32,
    pub takeover_after_missed: u8,
    pub boot_listen_ms: u32,
    pub peer_device_short_address: Option<u8>,
    pub peer_url: String,
}

pub const ROLE_PRIMARY: &str = "primary";
pub const ROLE_STANDBY: &str = "standby";

declare_settings_http_surface! {
    dto: RedundancySettingsDto,
    read_port: RedundancySettingsReadPort,
    view_fn: redundancy_settings_view,
    view_to_dto: redundancy_settings_view_to_dto,
    watch_port: RedundancySettingsApplyWatchPort,
    watch_load: redundancy_settings_applied_load,
    http_state: RedundancySettingsHttpState,
    dto_fn: redundancy_settings_dto,
    apply_watch: RedundancySettingsApplyWatch,
    read_bridge: RedundancySettingsHttpStateBridge,
    watch_bridge: RedundancySettingsApplyWatchBridge,
}

pub fn redundancy_settings_view_to_dto(view: RedundancySettingsView) -> RedundancySettingsDto {
    RedundancySettingsDto {
        enabled: view.enabled,
        role: if view.standby_role {
            ROLE_STANDBY
        } else {
            ROLE_PRIMARY
        },
        probe_interval_ms: view.probe_interval_ms,
        takeover_after_missed: view.takeover_after_missed,
        boot_listen_ms: view.boot_listen_ms,
        peer_device_short_address: view.peer_device_short_address,
        peer_url: view.peer_url,
    }
}
