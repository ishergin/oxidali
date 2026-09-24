use dali2rust_domain::registry::PoliciesView;
use serde::Serialize;

use crate::http::settings_surface::declare_settings_http_surface;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PoliciesDto {
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub apply_on_discovery: bool,
    pub manages_anything: bool,
}

declare_settings_http_surface! {
    dto: PoliciesDto,
    read_port: PoliciesReadPort,
    view_fn: policies_view,
    view_to_dto: policies_view_to_dto,
    watch_port: PoliciesApplyWatchPort,
    watch_load: policies_applied_load,
    http_state: PoliciesHttpState,
    dto_fn: policies_dto,
    apply_watch: PoliciesApplyWatch,
    read_bridge: PoliciesHttpStateBridge,
    watch_bridge: PoliciesApplyWatchBridge,
}

pub fn policies_view_to_dto(view: PoliciesView) -> PoliciesDto {
    PoliciesDto {
        system_failure_level: view.system_failure_level,
        power_on_level: view.power_on_level,
        apply_on_discovery: view.apply_on_discovery,
        manages_anything: view.manages_anything(),
    }
}
