use dali2rust_contracts::msg::DisplayPage;
use dali2rust_domain::registry::DisplaySettingsView;
use serde::Serialize;

use crate::http::settings_surface::declare_settings_http_surface;

pub const ROTATION_UPRIGHT_DEG: u16 = 0;
pub const ROTATION_UPSIDE_DOWN_DEG: u16 = 180;

#[derive(Clone, Debug, Serialize)]
pub struct DisplaySettingsDto {
    pub enabled: bool,
    pub i2c_address: u8,
    pub rotation_deg: u16,
    pub screensaver_timeout_s: u32,
    pub pages: Vec<&'static str>,
}

declare_settings_http_surface! {
    dto: DisplaySettingsDto,
    read_port: DisplaySettingsReadPort,
    view_fn: display_settings_view,
    view_to_dto: display_settings_view_to_dto,
    watch_port: DisplaySettingsApplyWatchPort,
    watch_load: display_settings_applied_load,
    http_state: DisplaySettingsHttpState,
    dto_fn: display_settings_dto,
    apply_watch: DisplaySettingsApplyWatch,
    read_bridge: DisplaySettingsHttpStateBridge,
    watch_bridge: DisplaySettingsApplyWatchBridge,
}

pub(crate) fn display_settings_view_to_dto(view: DisplaySettingsView) -> DisplaySettingsDto {
    DisplaySettingsDto {
        enabled: view.enabled,
        i2c_address: view.i2c_address,
        rotation_deg: if view.upside_down { ROTATION_UPSIDE_DOWN_DEG } else { ROTATION_UPRIGHT_DEG },
        screensaver_timeout_s: view.screensaver_timeout_s,
        pages: DisplayPage::in_mask(view.pages_mask).map(DisplayPage::as_str).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_listed_in_their_fixed_order_whatever_the_mask_bit_order() {
        let view = DisplaySettingsView {
            pages_mask: DisplayPage::Activity.mask_bit() | DisplayPage::Summary.mask_bit(),
            ..DisplaySettingsView::default()
        };
        assert_eq!(display_settings_view_to_dto(view).pages, vec!["summary", "activity"]);
    }

    #[test]
    fn an_upside_down_panel_reads_as_half_a_turn() {
        let view = DisplaySettingsView { upside_down: true, ..DisplaySettingsView::default() };
        assert_eq!(display_settings_view_to_dto(view).rotation_deg, ROTATION_UPSIDE_DOWN_DEG);
    }
}
