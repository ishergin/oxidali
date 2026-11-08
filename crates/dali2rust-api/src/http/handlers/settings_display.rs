use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{DisplayPage, DisplaySettingsUpdateCommand};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::DisplaySettingsView;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::display_settings_state::{
    DisplaySettingsApplyWatch, DisplaySettingsHttpState, ROTATION_UPRIGHT_DEG,
    ROTATION_UPSIDE_DOWN_DEG,
};
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

use super::common::{
    check_body_keys, json_err, json_stream_dto, parse_bool_field, parse_json_body,
    publish_and_await_apply,
};
use crate::http::handlers::common::require_get;

type JsonObject = serde_json::Map<String, serde_json::Value>;

const ALLOWED_KEYS: &[&str] = &[
    "enabled",
    "i2c_address",
    "rotation_deg",
    "screensaver_timeout_s",
    "pages",
];

declare_handler_shell! {
    DisplaySettingsGetHandler {
        state: Arc<dyn DisplaySettingsHttpState>,
    }
}

impl ApiHandler for DisplaySettingsGetHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if let Err(error) = require_get(method) {
            return error;
        }
        json_stream_dto(self.state.display_settings_dto())
    }
}

declare_handler_shell!(DisplaySettingsPatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn DisplaySettingsHttpState>,
    apply_watch: Arc<dyn DisplaySettingsApplyWatch>,
    bus_id: BusId,
    timeout_ms: u64,
});

impl crate::http::handlers::common::MutatingHandler for DisplaySettingsPatchHandler {
    type Validated = DisplaySettingsUpdateCommand;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        _params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<DisplaySettingsUpdateCommand, HttpResponse> {
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        check_body_keys(obj, ALLOWED_KEYS, &[])?;
        parse_display_settings_patch(obj)
    }

    fn execute(&self, patch: DisplaySettingsUpdateCommand) -> Result<(), HttpResponse> {
        if patch.patch_mask == 0 {
            return Ok(());
        }
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            None,
            patch,
        );
        let watch = Arc::clone(&self.apply_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
            move || watch.display_settings_applied_load(),
        )
    }

    fn respond(&self, (): ()) -> HttpResponse {
        json_stream_dto(self.state.display_settings_dto())
    }
}

fn empty_patch() -> DisplaySettingsUpdateCommand {
    DisplaySettingsUpdateCommand {
        patch_mask: 0,
        enabled: false,
        i2c_address: 0,
        upside_down: false,
        screensaver_timeout_s: 0,
        pages_mask: 0,
    }
}

fn parse_display_settings_patch(
    obj: &JsonObject,
) -> Result<DisplaySettingsUpdateCommand, HttpResponse> {
    let mut patch = empty_patch();
    parse_panel_fields(obj, &mut patch)?;
    parse_content_fields(obj, &mut patch)?;
    Ok(patch)
}

fn parse_panel_fields(
    obj: &JsonObject,
    patch: &mut DisplaySettingsUpdateCommand,
) -> Result<(), HttpResponse> {
    if let Some(enabled) = parse_bool_field(obj, "enabled")? {
        patch.patch_mask |= DisplaySettingsUpdateCommand::PATCH_ENABLED;
        patch.enabled = enabled;
    }
    if let Some(address) = parse_i2c_address(obj)? {
        patch.patch_mask |= DisplaySettingsUpdateCommand::PATCH_I2C_ADDRESS;
        patch.i2c_address = address;
    }
    if let Some(upside_down) = parse_rotation(obj)? {
        patch.patch_mask |= DisplaySettingsUpdateCommand::PATCH_UPSIDE_DOWN;
        patch.upside_down = upside_down;
    }
    Ok(())
}

fn parse_content_fields(
    obj: &JsonObject,
    patch: &mut DisplaySettingsUpdateCommand,
) -> Result<(), HttpResponse> {
    if let Some(timeout_s) = parse_screensaver(obj)? {
        patch.patch_mask |= DisplaySettingsUpdateCommand::PATCH_SCREENSAVER_TIMEOUT_S;
        patch.screensaver_timeout_s = timeout_s;
    }
    if let Some(mask) = parse_pages(obj)? {
        patch.patch_mask |= DisplaySettingsUpdateCommand::PATCH_PAGES_MASK;
        patch.pages_mask = mask;
    }
    Ok(())
}

fn parse_u32(obj: &JsonObject, key: &str) -> Result<Option<u32>, HttpResponse> {
    let Some(v) = obj.get(key) else {
        return Ok(None);
    };
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .map(Some)
        .ok_or_else(|| json_err(422, "invalid_value"))
}

fn parse_i2c_address(obj: &JsonObject) -> Result<Option<u8>, HttpResponse> {
    let Some(n) = parse_u32(obj, "i2c_address")? else {
        return Ok(None);
    };
    u8::try_from(n)
        .ok()
        .filter(|address| DisplaySettingsView::address_is_valid(*address))
        .map(Some)
        .ok_or_else(|| json_err(422, "invalid_value"))
}

fn parse_rotation(obj: &JsonObject) -> Result<Option<bool>, HttpResponse> {
    match parse_u32(obj, "rotation_deg")? {
        None => Ok(None),
        Some(deg) if deg == u32::from(ROTATION_UPRIGHT_DEG) => Ok(Some(false)),
        Some(deg) if deg == u32::from(ROTATION_UPSIDE_DOWN_DEG) => Ok(Some(true)),
        Some(_) => Err(json_err(422, "invalid_value")),
    }
}

fn parse_screensaver(obj: &JsonObject) -> Result<Option<u32>, HttpResponse> {
    match parse_u32(obj, "screensaver_timeout_s")? {
        Some(s) if !DisplaySettingsView::screensaver_is_valid(s) => {
            Err(json_err(422, "invalid_value"))
        }
        other => Ok(other),
    }
}

fn parse_pages(obj: &JsonObject) -> Result<Option<u8>, HttpResponse> {
    let Some(v) = obj.get("pages") else {
        return Ok(None);
    };
    let names = v.as_array().ok_or_else(|| json_err(422, "invalid_value"))?;
    let mut mask: u8 = 0;
    for name in names {
        let name = name.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
        let page = DisplayPage::parse(name).ok_or_else(|| json_err(422, "invalid_enum"))?;
        if mask & page.mask_bit() != 0 {
            return Err(json_err(422, "invalid_value"));
        }
        mask |= page.mask_bit();
    }
    if !DisplaySettingsView::pages_are_valid(mask) {
        return Err(json_err(422, "invalid_value"));
    }
    Ok(Some(mask))
}
