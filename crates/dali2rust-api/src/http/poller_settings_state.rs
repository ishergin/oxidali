use dali2rust_contracts::msg::DaliAttributeGroup;
use dali2rust_domain::registry::PollerSettingsView;
use serde::Serialize;

use crate::http::settings_surface::declare_settings_http_surface;

#[derive(Clone, Debug, Serialize)]
pub struct PollerSettingsDto {
    pub enabled: bool,
    pub interval_ms: u32,
    pub attribute_groups_default: Vec<&'static str>,
    pub include_dt8_color: bool,
    pub include_energy: bool,
    pub include_diagnostics: bool,
    pub skip_unbound_virtual_lamps: bool,
}

declare_settings_http_surface! {
    dto: PollerSettingsDto,
    read_port: PollerSettingsReadPort,
    view_fn: poller_settings_view,
    view_to_dto: poller_settings_view_to_dto,
    watch_port: PollerSettingsApplyWatchPort,
    watch_load: poller_settings_applied_load,
    http_state: PollerSettingsHttpState,
    dto_fn: poller_settings_dto,
    apply_watch: PollerSettingsApplyWatch,
    read_bridge: PollerSettingsHttpStateBridge,
    watch_bridge: PollerSettingsApplyWatchBridge,
}

pub(crate) fn poller_settings_view_to_dto(view: PollerSettingsView) -> PollerSettingsDto {
    PollerSettingsDto {
        enabled: view.enabled,
        interval_ms: view.interval_ms,
        attribute_groups_default: DaliAttributeGroup::ALL
            .into_iter()
            .filter(|g| view.attribute_groups_mask & g.mask_bit() != 0)
            .map(DaliAttributeGroup::as_str)
            .collect(),
        include_dt8_color: view.include_dt8_color,
        include_energy: view.include_energy,
        include_diagnostics: view.include_diagnostics,
        skip_unbound_virtual_lamps: view.skip_unbound_virtual_lamps,
    }
}
