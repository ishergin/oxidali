use std::collections::HashMap;
use dali2rust_contracts::msg::{VirtualLampConfigUpdateCommand};
use std::io::Write;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::LightSetpoint;

use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handlers::resource_surface::{
    declare_handler_shell, declare_metadata_patch_handler, declare_read_handler,
};
use crate::http::types::HttpResponse;
use crate::http::virtual_lamp_state::{
    VirtualLampBindingApplyWatch, VirtualLampHttpState, VirtualLampPatchWatch,
};

use super::common::{
    json_err, json_stream_dto, parse_adapter_id,
    parse_resource_id_param, parse_typed_body, vl_cap_supports_color_mode, publish_and_await_apply,
    write_json_array_items,
};
use crate::http::target_state_request::TargetStateBody;

const MAX_LAMP_ID: u16 = 63;

fn parse_lamp_id(params: &HashMap<String, String>) -> Result<u8, HttpResponse> {
    parse_resource_id_param(params, "lamp_id", MAX_LAMP_ID)
}

fn parse_adapter_and_lamp(
    adapter_count: u8,
    params: &HashMap<String, String>,
) -> Result<(u8, u8), HttpResponse> {
    let adapter_id = parse_adapter_id(adapter_count, params)?;
    let lamp_id = parse_lamp_id(params)?;
    Ok((adapter_id, lamp_id))
}

declare_read_handler! {
    VirtualLampsListHandler,
    state: VirtualLampHttpState,
    respond: |state: &Arc<dyn VirtualLampHttpState>, aid| {
        let state = Arc::clone(state);
        HttpResponse::json_stream(
            200,
            Box::new(move |w: &mut dyn Write| {
                write!(w, "{{\"adapter_id\":{},\"virtual_lamps\":[", aid)?;
                let ids = state.list_virtual_lamp_ids(aid);
                write_json_array_items(
                    w,
                    ids.into_iter().map(|lamp_id| state.virtual_lamp_dto(aid, lamp_id)),
                )?;
                w.write_all(b"]}")?;
                Ok(())
            }),
        )
    },
}

declare_read_handler! {
    VirtualLampGetHandler,
    state: VirtualLampHttpState,
    id: parse_lamp_id,
    respond: |state: &Arc<dyn VirtualLampHttpState>, aid, lid| {
        json_stream_dto(state.virtual_lamp_dto(aid, lid))
    },
}

declare_metadata_patch_handler! {
    VirtualLampPatchHandler,
    state: VirtualLampHttpState,
    watch: VirtualLampPatchWatch,
    watch_load: virtual_lamp_metadata_applied_load,
    data: VlPatchData,
    id: parse_lamp_id,
    parse: parse_vl_patch_data,
    command: VirtualLampConfigUpdateCommand { virtual_lamp_id, ha_entity_enabled },
    origin: None,
    echo: |state: &Arc<dyn VirtualLampHttpState>, aid, lid| {
        json_stream_dto(state.virtual_lamp_dto(aid, lid))
    },
}

declare_handler_shell! {
    VirtualLampBindingPutHandler {
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn VirtualLampHttpState>,
        binding_watch: Arc<dyn VirtualLampBindingApplyWatch>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl crate::http::handlers::common::MutatingHandler for VirtualLampBindingPutHandler {
    type Validated = (u8, u8, u8);
    type Executed = (u8, u8, u8);

    fn expected_method(&self) -> &'static str {
        "PUT"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(u8, u8, u8), HttpResponse> {
        let (aid, lid) = parse_adapter_and_lamp(self.state.adapter_count(), params)?;
        let v: serde_json::Value =
            serde_json::from_slice(body).map_err(|_| json_err(400, "invalid_json"))?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;

        let Some(sa_val) = obj.get("physical_short_address").filter(|_| obj.len() == 1) else {
            return Err(json_err(400, "unknown_field"));
        };
        let sa_u = sa_val
            .as_u64()
            .ok_or_else(|| json_err(422, "invalid_value"))?;

        if sa_u > 63 {
            return Err(json_err(422, "invalid_value"));
        }
        let short = sa_u as u8;

        if self.state.physical_short_on_other_adapter(aid, short) {
            return Err(json_err(409, "conflict"));
        }

        Ok((aid, lid, short))
    }

    fn execute(&self, args: (u8, u8, u8)) -> Result<(u8, u8, u8), HttpResponse> {
        let (aid, lid, short) = args;
        let prior = self.state.virtual_lamp_dto(aid, lid);
        let correlation_id = self.correlation.next_id();

        let cmd = if prior.binding.is_some() {
            dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampRebindCommand { adapter_id: aid, virtual_lamp_id: lid, physical_short_address: short })
        } else {
            dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: aid, virtual_lamp_id: lid, physical_short_address: short })
        };

        let frame = BusFrame::command(cmd);

        let watch = Arc::clone(&self.binding_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            frame,
            move || watch.virtual_lamp_binding_applied_load(),
        )?;
        Ok((aid, lid, short))
    }

    fn respond(&self, args: (u8, u8, u8)) -> HttpResponse {
        let (aid, lid, short) = args;
        let dto = self.state.virtual_lamp_dto(aid, lid);
        if dto.binding.as_ref().map(|b| b.physical_short_address) != Some(short) {
            return json_err(422, "invalid_value");
        }
        json_stream_dto(dto)
    }
}

declare_handler_shell! {
    VirtualLampBindingDeleteHandler {
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn VirtualLampHttpState>,
        binding_watch: Arc<dyn VirtualLampBindingApplyWatch>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl crate::http::handlers::common::MutatingHandler for VirtualLampBindingDeleteHandler {
    type Validated = (u8, u8);
    type Executed = (u8, u8);

    fn expected_method(&self) -> &'static str {
        "DELETE"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<(u8, u8), HttpResponse> {
        parse_adapter_and_lamp(self.state.adapter_count(), params)
    }

    fn execute(&self, args: (u8, u8)) -> Result<(u8, u8), HttpResponse> {
        let (aid, lid) = args;
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampUnbindCommand { adapter_id: aid, virtual_lamp_id: lid });
        let frame = BusFrame::command(cmd);

        let watch = Arc::clone(&self.binding_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            frame,
            move || watch.virtual_lamp_binding_applied_load(),
        )?;
        Ok((aid, lid))
    }

    fn respond(&self, args: (u8, u8)) -> HttpResponse {
        let (aid, lid) = args;
        let dto = self.state.virtual_lamp_dto(aid, lid);
        json_stream_dto(dto)
    }
}

declare_handler_shell! {
    VirtualLampDeleteHandler {
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn VirtualLampHttpState>,
        patch_watch: Arc<dyn VirtualLampPatchWatch>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl crate::http::handlers::common::MutatingHandler for VirtualLampDeleteHandler {
    type Validated = (u8, u8);
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "DELETE"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<(u8, u8), HttpResponse> {
        parse_adapter_and_lamp(self.state.adapter_count(), params)
    }

    fn execute(&self, args: (u8, u8)) -> Result<(), HttpResponse> {
        let (aid, lid) = args;
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::VirtualLampDeleteCommand {
                adapter_id: aid,
                virtual_lamp_id: lid,
            },
        );
        let watch = Arc::clone(&self.patch_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
            move || watch.virtual_lamp_metadata_applied_load(),
        )?;
        Ok(())
    }

    fn respond(&self, _args: ()) -> HttpResponse {
        HttpResponse::no_content()
    }
}

declare_handler_shell! {
    VirtualLampTargetStateHandler {
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn VirtualLampHttpState>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl crate::http::handlers::common::MutatingHandler for VirtualLampTargetStateHandler {
    type Validated = (u8, u8, LightSetpoint);
    type Executed = (u8, u8);

    fn expected_method(&self) -> &'static str {
        "PUT"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(u8, u8, LightSetpoint), HttpResponse> {
        let (aid, lid) = parse_adapter_and_lamp(self.state.adapter_count(), params)?;
        let vl = self.state.virtual_lamp_dto(aid, lid);

        let request = parse_typed_body::<TargetStateBody>(body)?;
        let sp = crate::http::target_state_request::parse_light_setpoint(&request, |cm| {
            vl_cap_supports_color_mode(&vl, cm)
        })?;

        if vl.binding.is_none() {
            return Err(HttpResponse::json(
                200,
                serde_json::to_vec(&vl).unwrap_or_else(|_| b"{}".to_vec()),
            ));
        }
        Ok((aid, lid, sp))
    }

    fn execute(&self, args: (u8, u8, LightSetpoint)) -> Result<(u8, u8), HttpResponse> {
        let (aid, lid, sp) = args;
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliSetTargetStateCommand::for_virtual_lamp(aid, lid, &sp));
        let frame = BusFrame::command(cmd);

        crate::http::dispatcher::dispatch_and_wait_for_success(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            frame,
        )?;
        Ok((aid, lid))
    }

    fn respond(&self, args: (u8, u8)) -> HttpResponse {
        let (aid, lid) = args;
        let dto = self.state.virtual_lamp_dto(aid, lid);
        json_stream_dto(dto)
    }
}

fn validate_vl_patch_keys(obj: &serde_json::Map<String, serde_json::Value>) -> Result<(), HttpResponse> {
    const RO: &[&str] = &[
        "adapter_id",
        "virtual_lamp_id",
        "state",
        "capabilities",
        "device_type_effective",
        "device_type_source",
        "color_mode_effective",
        "color_mode_source",
        "binding",
    ];
    for k in obj.keys() {
        if matches!(
            k.as_str(),
            "name" | "ha_entity_enabled"
        ) {
            continue;
        }
        if RO.contains(&k.as_str()) {
            return Err(json_err(422, "unsupported_field"));
        }
        return Err(json_err(400, "unknown_field"));
    }
    Ok(())
}

fn parse_vl_patch_data(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<VlPatchData, HttpResponse> {
    validate_vl_patch_keys(obj)?;
    let mut data = VlPatchData {
        patch_mask: 0,
        name: None,
        ha_flag: None,
    };

    if let Some(val) = obj.get("name") {
        data.patch_mask |= VirtualLampConfigUpdateCommand::PATCH_NAME;
        let s = val.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
        if s.is_empty() || s.as_bytes().len() > 64 {
            return Err(json_err(422, "invalid_value"));
        }
        data.name = Some(s.to_string());
    }
    if let Some(val) = obj.get("ha_entity_enabled") {
        data.patch_mask |= VirtualLampConfigUpdateCommand::PATCH_HA_ENTITY_ENABLED;
        data.ha_flag = Some(
            val.as_bool()
                .ok_or_else(|| json_err(422, "invalid_value"))?,
        );
    }
    Ok(data)
}

