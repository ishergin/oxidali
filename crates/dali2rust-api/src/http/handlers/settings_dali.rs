use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::DaliSettingsUpdateCommand;
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dali_settings_state::{DaliSettingsApplyWatch, DaliSettingsHttpState};
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

use super::common::{
    json_err, json_stream_dto, parse_bool_field, parse_json_body, publish_and_await_apply,
};
use crate::http::handlers::common::require_get;

const ALLOWED_KEYS: &[&str] = &[
    "dt8_auto_activation_repair",
    "dt8_rgbwaf_control_assert",
    "application_active",
    "device_short_address",
];

declare_handler_shell! {
    DaliSettingsGetHandler {
        state: Arc<dyn DaliSettingsHttpState>,
    }
}

impl ApiHandler for DaliSettingsGetHandler {
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
        json_stream_dto(self.state.dali_settings_dto())
    }
}

declare_handler_shell!(DaliSettingsPatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn DaliSettingsHttpState>,
    apply_watch: Arc<dyn DaliSettingsApplyWatch>,
    bus_id: BusId,
    timeout_ms: u64,
});

#[derive(Default)]
pub struct DaliSettingsPatchData {
    patch_mask: u8,
    dt8_auto_activation_repair: bool,
    dt8_rgbwaf_control_assert: bool,
    application_active: bool,
    device_short_address: u8,
}

impl crate::http::handlers::common::MutatingHandler for DaliSettingsPatchHandler {
    type Validated = DaliSettingsPatchData;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        _params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<DaliSettingsPatchData, HttpResponse> {
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        for key in obj.keys() {
            if !ALLOWED_KEYS.contains(&key.as_str()) {
                return Err(json_err(400, "unknown_field"));
            }
        }
        let mut data = DaliSettingsPatchData::default();
        if let Some(value) = parse_bool_field(obj, "dt8_auto_activation_repair")? {
            data.patch_mask |= DaliSettingsUpdateCommand::PATCH_DT8_AUTO_ACTIVATION_REPAIR;
            data.dt8_auto_activation_repair = value;
        }
        if let Some(value) = parse_bool_field(obj, "dt8_rgbwaf_control_assert")? {
            data.patch_mask |= DaliSettingsUpdateCommand::PATCH_DT8_RGBWAF_CONTROL_ASSERT;
            data.dt8_rgbwaf_control_assert = value;
        }
        if let Some(value) = parse_bool_field(obj, "application_active")? {
            data.patch_mask |= DaliSettingsUpdateCommand::PATCH_APPLICATION_ACTIVE;
            data.application_active = value;
        }
        if let Some(value) = obj.get("device_short_address") {
            data.patch_mask |= DaliSettingsUpdateCommand::PATCH_DEVICE_SHORT_ADDRESS;
            data.device_short_address = match value {
                serde_json::Value::Null => 0xFF,
                other => {
                    let n = other.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
                    if n > 63 {
                        return Err(json_err(422, "invalid_value"));
                    }
                    u8::try_from(n).unwrap_or(0xFF)
                }
            };
        }
        Ok(data)
    }

    fn execute(&self, data: DaliSettingsPatchData) -> Result<(), HttpResponse> {
        if data.patch_mask == 0 {
            return Ok(());
        }
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            None,
            DaliSettingsUpdateCommand {
                patch_mask: data.patch_mask,
                dt8_auto_activation_repair: data.dt8_auto_activation_repair,
                dt8_rgbwaf_control_assert: data.dt8_rgbwaf_control_assert,
                application_active: data.application_active,
                device_short_address: data.device_short_address,
            },
        );
        let watch = Arc::clone(&self.apply_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
            move || watch.dali_settings_applied_load(),
        )
    }

    fn respond(&self, (): ()) -> HttpResponse {
        json_stream_dto(self.state.dali_settings_dto())
    }
}
