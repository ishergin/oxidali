use crate::http::handlers::common::get_adapter_id;
use crate::http::handlers::resource_surface::declare_handler_shell;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    ColorMode, DaliAttributeGroup, DeviceType, DiscoveryMode, LightSetpoint,
    MemoryBankReadPreset, OperationType,
};
use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use dali2rust_platform::clock::UnixTimeMs;

use crate::http::physical_device_state::{
    write_attribute_section, PhysicalDeviceHttpState, PhysicalDevicePatchWatch,
};
use crate::http::types::HttpResponse;
use dali2rust_domain::registry::AttributeSectionKind;

use super::common::{
    accepted_operation_response, json_err, json_err_with_message, json_stream_dto, parse_adapter_id,
    parse_color_mode, parse_device_type, parse_json_body, parse_physical_short, parse_typed_body,
    pd_cap_supports_color_mode, wait_apply_counter, write_json_array_items,
    APPLY_WATCH_BUDGET_MS,
};
use crate::http::target_state_request::TargetStateBody;
use super::operation_dispatch::{
    publish_begin_then_semantic_command_pair,
};

declare_handler_shell! {
    PhysicalDevicesListHandler {
        state: Arc<dyn PhysicalDeviceHttpState>,
        wall: Arc<dyn UnixTimeMs>,
    }
}

impl ApiHandler for PhysicalDevicesListHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let aid = match get_adapter_id(method, self.state.adapter_count(), params) {
            Ok(id) => id,
            Err(error) => return error,
        };
        let aid_u8 = aid;
        let state = Arc::clone(&self.state);
        let now_ms = self.wall.unix_millis();
        HttpResponse::json_stream(
            200,
            Box::new(move |w: &mut dyn Write| {
                write!(w, "{{\"adapter_id\":{},\"now_ms\":{},\"physical_devices\":[", aid_u8, now_ms)?;
                let shorts = state.list_physical_device_short_addresses(aid_u8);
                write_json_array_items(
                    w,
                    shorts
                        .into_iter()
                        .filter_map(|sa| state.physical_device_summary_dto(aid_u8, sa)),
                )?;
                w.write_all(b"]}")?;
                Ok(())
            }),
        )
    }
}

declare_handler_shell! {
    PhysicalDeviceGetHandler {
        state: Arc<dyn PhysicalDeviceHttpState>,
        wall: Arc<dyn UnixTimeMs>,
    }
}

impl ApiHandler for PhysicalDeviceGetHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let adapter_id = match get_adapter_id(method, self.state.adapter_count(), params) {
            Ok(id) => id,
            Err(error) => return error,
        };
        let short = match parse_physical_short(params) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let Some(mut dto) = self.state.physical_device_core_dto(adapter_id, short) else {
            return json_err(404, "not_found");
        };
        dto.now_ms = self.wall.unix_millis();
        json_stream_dto(dto)
    }
}

declare_handler_shell!(PhysicalDevicePatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn PhysicalDeviceHttpState>,
    apply_watch: Arc<dyn PhysicalDevicePatchWatch>,
    wall: Arc<dyn UnixTimeMs>,
    bus_id: BusId,
    timeout_ms: u64,
});

pub struct PdPatchData {
    patch_mask: u8,
    name: String,
    notes: Option<String>,
    clear_dt: bool,
    dt_override: DeviceType,
    clear_cm: bool,
    cm_override: ColorMode,
    dt8_auto_activation_repair: bool,
    dt8_rgbwaf_control_assert: bool,
}

declare_handler_shell! {
    PhysicalDeviceDeleteHandler {
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn PhysicalDeviceHttpState>,
        apply_watch: Arc<dyn PhysicalDevicePatchWatch>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl crate::http::handlers::common::MutatingHandler for PhysicalDeviceDeleteHandler {
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
        let aid = parse_adapter_id(self.state.adapter_count(), params)?;
        let short = parse_physical_short(params)?;
        Ok((aid, short))
    }

    fn execute(&self, args: (u8, u8)) -> Result<(), HttpResponse> {
        let (aid, short) = args;
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::PhysicalDeviceDeleteCommand {
                adapter_id: aid,
                short_address: short,
            },
        );
        let watch = Arc::clone(&self.apply_watch);
        crate::http::handlers::common::publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
            move || watch.physical_override_applied_load(),
        )?;
        Ok(())
    }

    fn respond(&self, _args: ()) -> HttpResponse {
        HttpResponse::no_content()
    }
}

impl crate::http::handlers::common::MutatingHandler for PhysicalDevicePatchHandler {
    type Validated = (u8, u8, PdPatchData);
    type Executed = (u8, u8);

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(u8, u8, PdPatchData), HttpResponse> {
        let aid = parse_adapter_id(self.state.adapter_count(), params)?;
        let short = parse_physical_short(params)?;
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        validate_pd_patch_keys(obj)?;
        let data = parse_pd_patch_fields(obj)?;
        validate_pd_patch_caps(&data)?;
        validate_pd_override_within_declared_types(
            self.state.physical_device_supported_types(aid, short),
            &data,
        )?;
        Ok((aid, short, data))
    }

    fn execute(&self, args: (u8, u8, PdPatchData)) -> Result<(u8, u8), HttpResponse> {
        let (aid, short, data) = args;
        let batch = self.build_pd_patch_batch(aid, short, &data);
        if batch.is_empty() {
            return Ok((aid, short));
        }
        let published = batch.len() as u32;
        let before = self.apply_watch.physical_override_applied_load();
        crate::http::dispatcher::publish_batch_and_wait_for_success(
            &self.publisher,
            &self.slots,
            self.timeout_ms,
            batch,
        )?;
        let watch = Arc::clone(&self.apply_watch);
        wait_apply_counter(
            move || watch.physical_override_applied_load(),
            before.saturating_add(published.saturating_sub(1)),
            APPLY_WATCH_BUDGET_MS.min(self.timeout_ms),
        )?;
        Ok((aid, short))
    }

    fn respond(&self, args: (u8, u8)) -> HttpResponse {
        let (aid, short) = args;
        let Some(mut dto) = self.state.physical_device_core_dto(aid, short) else {
            return json_err(404, "not_found");
        };
        dto.now_ms = self.wall.unix_millis();
        json_stream_dto(dto)
    }
}

impl PhysicalDevicePatchHandler {
    fn build_pd_patch_batch(
        &self,
        aid: u8,
        short: u8,
        data: &PdPatchData,
    ) -> Vec<(u64, BusFrame)> {
        let mut batch = Vec::with_capacity(2);
        if data.patch_mask != 0 {
            let correlation_id = self.correlation.next_id();
            let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceOverrideCommand { adapter_id: aid, short_address: short, patch_mask: data.patch_mask, name: dali2rust_contracts::msg::fixed_text_64(&data.name), clear_device_type_override: data.clear_dt, device_type_override: data.dt_override, clear_color_mode_override: data.clear_cm, color_mode_override: data.cm_override, dt8_auto_activation_repair: data.dt8_auto_activation_repair, dt8_rgbwaf_control_assert: data.dt8_rgbwaf_control_assert });
            batch.push((correlation_id, BusFrame::command(cmd)));
        }
        if let Some(notes) = &data.notes {
            let correlation_id = self.correlation.next_id();
            let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceNotesUpdateCommand { adapter_id: aid, short_address: short, notes: dali2rust_contracts::msg::fixed_text_48(notes) });
            batch.push((correlation_id, BusFrame::command(cmd)));
        }
        batch
    }
}

declare_handler_shell!(
    PhysicalDeviceWriteAttributesHandler {
        publisher: BusPublisher,
        correlation: Arc<CorrelationIdAllocator>,
        bus_id: BusId,
        adapter_count: u8,
    }
);

pub struct PdWriteAttrData {
    fade_time_ms: Option<u32>,
    fade_rate: Option<u8>,
    power_on_level: Option<u8>,
    system_failure_level: Option<u8>,
    extended_fade_time_ms: Option<u16>,
    tc_coolest_mirek: Option<u16>,
    tc_warmest_mirek: Option<u16>,
    min_level: Option<u8>,
    max_level: Option<u8>,
    dimming_curve: Option<u8>,
}

impl crate::http::handlers::common::MutatingHandler for PhysicalDeviceWriteAttributesHandler {
    type Validated = (u8, u8, PdWriteAttrData);
    type Executed = String;

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(u8, u8, PdWriteAttrData), HttpResponse> {
        let aid = parse_adapter_id(self.adapter_count, params)?;
        let short = parse_physical_short(params)?;
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        validate_pd_write_attr_keys(obj)?;
        let data = parse_pd_write_attr_fields(obj)?;
        Ok((aid, short, data))
    }

    fn execute(&self, args: (u8, u8, PdWriteAttrData)) -> Result<String, HttpResponse> {
        let (aid, short, data) = args;
        let op_serial = self.correlation.next_id();
        let op_key = format!("pd-wattr-{aid}-{short}-{op_serial}");
        let workflow = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, workflow, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliWriteAttributesCommand { short_address: short, fade_time_ms: data.fade_time_ms, fade_rate: data.fade_rate, power_on_level: data.power_on_level, system_failure_level: data.system_failure_level, extended_fade_time_ms: data.extended_fade_time_ms, registry_adapter_id: aid, tc_coolest_mirek: data.tc_coolest_mirek, tc_warmest_mirek: data.tc_warmest_mirek, min_level: data.min_level, max_level: data.max_level, dimming_curve: data.dimming_curve, signals_operation: true });
        publish_begin_then_semantic_command_pair(
            &self.publisher,
            self.bus_id,
            workflow,
            &op_key,
            OperationType::AttributeWrite,
            cmd,
        )?;
        Ok(op_key)
    }

    fn respond(&self, op_key: String) -> HttpResponse {
        accepted_operation_response(op_key, OperationType::AttributeWrite)
    }
}

declare_handler_shell! {
    PhysicalDeviceTargetStateHandler {
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn PhysicalDeviceHttpState>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl crate::http::handlers::common::MutatingHandler for PhysicalDeviceTargetStateHandler {
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
        let aid = parse_adapter_id(self.state.adapter_count(), params)?;
        let ss = params
            .get("short")
            .ok_or_else(|| json_err(400, "missing_short_address"))?;
        let sa = ss
            .parse::<u16>()
            .map_err(|_| json_err(400, "invalid_resource_id"))?;
        if sa > 63 {
            return Err(json_err(422, "invalid_value"));
        }
        let short = sa as u8;

        let caps = self
            .state
            .physical_device_capabilities(aid, short)
            .ok_or_else(|| json_err(404, "not_found"))?;

        let request = parse_typed_body::<TargetStateBody>(body)?;
        let sp = crate::http::target_state_request::parse_light_setpoint(&request, |cm| {
            pd_cap_supports_color_mode(&caps, cm)
        })?;

        Ok((aid, short, sp))
    }

    fn execute(&self, args: (u8, u8, LightSetpoint)) -> Result<(u8, u8), HttpResponse> {
        let (aid, short, sp) = args;
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliSetTargetStateCommand::for_short(aid, short, &sp));
        let frame = BusFrame::command(cmd);

        crate::http::dispatcher::dispatch_and_wait_for_success(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            frame,
        )?;
        Ok((aid, short))
    }

    fn respond(&self, args: (u8, u8)) -> HttpResponse {
        let (aid, short) = args;
        let Some(dto) = self.state.physical_device_core_dto(aid, short) else {
            return json_err(404, "not_found");
        };
        json_stream_dto(dto)
    }
}

const SECTIONS_PARAM: &str = "sections";

fn parse_sections(params: &HashMap<String, String>) -> Result<Vec<AttributeSectionKind>, HttpResponse> {
    let Some(raw) = params.get(SECTIONS_PARAM) else {
        return Ok(AttributeSectionKind::ALL.to_vec());
    };
    let mut wanted: u16 = 0;
    for name in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let kind = AttributeSectionKind::from_wire_name(name)
            .ok_or_else(|| json_err_with_message(400, "invalid_value", &format!("unknown section '{name}'")))?;
        let bit = AttributeSectionKind::ALL
            .iter()
            .position(|k| *k == kind)
            .expect("every kind is in ALL");
        wanted |= 1 << bit;
    }
    Ok(AttributeSectionKind::ALL
        .iter()
        .copied()
        .enumerate()
        .filter(|(bit, _)| wanted & (1 << bit) != 0)
        .map(|(_, kind)| kind)
        .collect())
}

declare_handler_shell! {
    PhysicalDeviceAttributesHandler {
        state: Arc<dyn PhysicalDeviceHttpState>,
        wall: Arc<dyn UnixTimeMs>,
    }
}

impl ApiHandler for PhysicalDeviceAttributesHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let aid = match get_adapter_id(method, self.state.adapter_count(), params) {
            Ok(id) => id,
            Err(error) => return error,
        };
        let short = match parse_physical_short(params) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let sections = match parse_sections(params) {
            Ok(s) => s,
            Err(e) => return e,
        };
        if !self.state.physical_device_exists(aid, short) {
            return json_err(404, "not_found");
        }
        let state = Arc::clone(&self.state);
        let now_ms = self.wall.unix_millis();
        HttpResponse::json_stream(
            200,
            Box::new(move |w: &mut dyn Write| {
                write!(w, "{{\"short_address\":{short},\"now_ms\":{now_ms},\"attributes\":{{")?;
                write_attribute_sections(w, state.as_ref(), aid, short, &sections)?;
                w.write_all(b"}}")?;
                Ok(())
            }),
        )
    }
}

fn write_attribute_sections(
    w: &mut dyn Write,
    state: &dyn PhysicalDeviceHttpState,
    adapter_id: u8,
    short: u8,
    sections: &[AttributeSectionKind],
) -> std::io::Result<()> {
    let mut first = true;
    for kind in sections {
        if let Some(view) = state.physical_device_attribute_section(adapter_id, short, *kind) {
            write_attribute_section(w, &view, &mut first)?;
        }
    }
    Ok(())
}

declare_handler_shell! {
    PhysicalDeviceMemoryBanksHandler {
        state: Arc<dyn PhysicalDeviceHttpState>,
        wall: Arc<dyn UnixTimeMs>,
    }
}

impl ApiHandler for PhysicalDeviceMemoryBanksHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let aid = match get_adapter_id(method, self.state.adapter_count(), params) {
            Ok(id) => id,
            Err(error) => return error,
        };
        let short = match parse_physical_short(params) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let Some(banks) = self.state.physical_device_memory_banks(aid, short) else {
            return json_err(404, "not_found");
        };
        let now_ms = self.wall.unix_millis();
        HttpResponse::json_stream(
            200,
            Box::new(move |w: &mut dyn Write| {
                write!(w, "{{\"short_address\":{short},\"now_ms\":{now_ms},\"memory_banks\":[")?;
                write_json_array_items(w, banks.into_iter())?;
                w.write_all(b"]}")?;
                Ok(())
            }),
        )
    }
}

declare_handler_shell! {
    AdapterDiscoveryRunsHandler {
        publisher: BusPublisher,
        correlation: Arc<CorrelationIdAllocator>,
        bus_id: BusId,
        adapter_count: u8,
    }
}

impl ApiHandler for AdapterDiscoveryRunsHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if method != "POST" {
            return HttpResponse::method_not_allowed();
        }
        let adapter_id = match parse_adapter_id(self.adapter_count, params) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let v: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return json_err(400, "invalid_json"),
        };
        let mode_s = v.get("mode").and_then(|x| x.as_str()).unwrap_or_default();
        let Some(mode) = parse_discovery_mode(mode_s) else {
            return json_err(422, "invalid_enum");
        };
        let op_serial = self.correlation.next_id();
        let op_key = format!("pd-disc-{adapter_id}-{op_serial}");
        let workflow = self.correlation.next_id();
        let disc = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, workflow, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliDiscoverDevicesCommand { mode, registry_adapter_id: adapter_id });
        if let Err(e) = publish_begin_then_semantic_command_pair(
            &self.publisher,
            self.bus_id,
            workflow,
            &op_key,
            OperationType::Discovery,
            disc,
        ) {
            return e;
        }
        accepted_operation_response(op_key, OperationType::Discovery)
    }
}

declare_handler_shell! {
    PhysicalDeviceAttributeReadsHandler {
        publisher: BusPublisher,
        correlation: Arc<CorrelationIdAllocator>,
        bus_id: BusId,
        state: Arc<dyn PhysicalDeviceHttpState>,
    }
}

impl ApiHandler for PhysicalDeviceAttributeReadsHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let (adapter_id, short) =
            match validate_pd_operation_request(self.state.as_ref(), method, params) {
                Ok(v) => v,
                Err(e) => return e,
            };
        let aid = adapter_id;
        let (groups_mask, memory_banks) = match parse_attribute_groups_body(body) {
            Ok(m) => m,
            Err(e) => return e,
        };
        let op_serial = self.correlation.next_id();
        let op_key = format!("pd-attr-{adapter_id}-{short}-{op_serial}");
        let workflow = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, workflow, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliReadAttributesCommand { registry_adapter_id: aid, short_address: short, attribute_groups_mask: groups_mask, memory_banks });
        if let Err(e) = publish_begin_then_semantic_command_pair(
            &self.publisher,
            self.bus_id,
            workflow,
            &op_key,
            OperationType::AttributeRead,
            cmd,
        ) {
            return e;
        }
        accepted_operation_response(op_key, OperationType::AttributeRead)
    }
}

fn validate_pd_operation_request(
    state: &dyn PhysicalDeviceHttpState,
    method: &str,
    params: &HashMap<String, String>,
) -> Result<(u8, u8), HttpResponse> {
    if method != "POST" {
        return Err(HttpResponse::method_not_allowed());
    }
    let adapter_id = parse_adapter_id(state.adapter_count(), params)?;
    let short = parse_physical_short(params)?;
    if !state.physical_device_exists(adapter_id, short) {
        return Err(json_err(404, "not_found"));
    }
    Ok((adapter_id, short))
}

fn validate_pd_write_attr_keys(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), HttpResponse> {
    for k in obj.keys() {
        if !matches!(
            k.as_str(),
            "fade_time_ms" | "fade_rate" | "power_on_level" | "system_failure_level"
                | "extended_fade_time_ms" | "tc_coolest_mirek" | "tc_warmest_mirek"
                | "min_level" | "max_level" | "dimming_curve"
        ) {
            return Err(json_err(422, "unsupported_field"));
        }
    }
    Ok(())
}

fn parse_bounded_attr_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    max: u64,
) -> Result<Option<u64>, HttpResponse> {
    match obj.get(key).and_then(|v| v.as_u64()) {
        Some(x) if x > max => Err(json_err(422, "invalid_value")),
        other => Ok(other),
    }
}

fn parse_pd_write_attr_fields(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<PdWriteAttrData, HttpResponse> {
    const U16_MAX: u64 = u16::MAX as u64;
    // IEC 62386-102 Table 4
    const FADE_TIME_MS_MAX: u64 = 90_500;
    // IEC 62386-209 Table 8
    const TC_LIMIT_MAX: u64 = 65_534;
    let tc_coolest_mirek = parse_bounded_attr_field(obj, "tc_coolest_mirek", TC_LIMIT_MAX)?;
    let tc_warmest_mirek = parse_bounded_attr_field(obj, "tc_warmest_mirek", TC_LIMIT_MAX)?;
    if matches!(tc_coolest_mirek, Some(0)) || matches!(tc_warmest_mirek, Some(0)) {
        return Err(json_err(422, "invalid_value"));
    }
    if let (Some(c), Some(w)) = (tc_coolest_mirek, tc_warmest_mirek) {
        if c > w {
            return Err(json_err(422, "invalid_value"));
        }
    }
    let min_level = parse_bounded_attr_field(obj, "min_level", 254)?;
    let max_level = parse_bounded_attr_field(obj, "max_level", 254)?;
    if let (Some(lo), Some(hi)) = (min_level, max_level) {
        if lo > hi {
            return Err(json_err(422, "invalid_value"));
        }
    }
    let dimming_curve = parse_bounded_attr_field(obj, "dimming_curve", 1)?;
    Ok(PdWriteAttrData {
        fade_time_ms: parse_bounded_attr_field(obj, "fade_time_ms", FADE_TIME_MS_MAX)?
            .map(|x| x as u32),
        fade_rate: parse_bounded_attr_field(obj, "fade_rate", 15)?.map(|x| x as u8),
        power_on_level: parse_bounded_attr_field(obj, "power_on_level", 254)?.map(|x| x as u8),
        system_failure_level: parse_bounded_attr_field(obj, "system_failure_level", 254)?
            .map(|x| x as u8),
        extended_fade_time_ms: parse_bounded_attr_field(obj, "extended_fade_time_ms", U16_MAX)?
            .map(|x| x as u16),
        tc_coolest_mirek: tc_coolest_mirek.map(|x| x as u16),
        tc_warmest_mirek: tc_warmest_mirek.map(|x| x as u16),
        min_level: min_level.map(|x| x as u8),
        max_level: max_level.map(|x| x as u8),
        dimming_curve: dimming_curve.map(|x| x as u8),
    })
}

fn parse_attribute_groups_body(body: &[u8]) -> Result<(u8, MemoryBankReadPreset), HttpResponse> {
    let v = parse_json_body(body)?;
    let Some(arr) = v.get("attribute_groups").and_then(|x| x.as_array()) else {
        return Err(json_err(400, "invalid_json"));
    };
    if arr.is_empty() || arr.len() > 16 {
        return Err(json_err(422, "invalid_value"));
    }
    let mut groups_mask: u8 = 0;
    for el in arr {
        let s = el.as_str().ok_or_else(|| json_err(422, "invalid_enum"))?;
        let g = DaliAttributeGroup::parse(s).ok_or_else(|| json_err(422, "invalid_enum"))?;
        groups_mask |= g.mask_bit();
    }
    let memory_banks = match v.get("memory_banks") {
        None | Some(serde_json::Value::Null) => MemoryBankReadPreset::None,
        Some(value) => {
            let Some(raw) = value.as_str() else {
                return Err(json_err(422, "invalid_enum"));
            };
            parse_memory_banks_preset(raw).ok_or_else(|| json_err(422, "invalid_enum"))?
        }
    };
    Ok((groups_mask, memory_banks))
}

const PD_READ_ONLY_KEYS: &[&str] = &[
    "state",
    "attributes",
    "memory_banks",
    "capabilities",
    "adapter_id",
    "short_address",
    "device_type_discovered",
    "device_type_effective",
    "device_type_source",
    "supported_device_types",
    "color_mode_discovered",
    "color_mode_effective",
    "color_mode_source",
];

fn validate_pd_patch_keys(obj: &serde_json::Map<String, serde_json::Value>) -> Result<(), HttpResponse> {
    for k in obj.keys() {
        if matches!(
            k.as_str(),
            "name"
                | "notes"
                | "device_type_override"
                | "color_mode_override"
                | "dt8_auto_activation_repair"
                | "dt8_rgbwaf_control_assert"
        ) {
            continue;
        }
        if PD_READ_ONLY_KEYS.contains(&k.as_str()) {
            return Err(json_err(422, "unsupported_field"));
        }
        if k.contains("fade_time")
            || k.contains("fade_rate")
            || k.contains("power_on")
            || k.contains("extended_fade")
        {
            return Err(json_err_with_message(
                422,
                "unsupported_field",
                "use POST .../write-attributes",
            ));
        }
        return Err(json_err(400, "unknown_field"));
    }
    Ok(())
}

const MAX_PD_NAME_BYTES: usize = 64;
const MAX_PD_NOTES_BYTES: usize = 48;

fn pd_cap_error(field: &str, actual: usize, cap: usize) -> HttpResponse {
    json_err_with_message(
        422,
        "invalid_value",
        &format!("{field} is {actual} bytes; the limit is {cap} bytes (UTF-8, not characters)"),
    )
}

fn validate_pd_patch_caps(data: &PdPatchData) -> Result<(), HttpResponse> {
    if data.name.len() > MAX_PD_NAME_BYTES {
        return Err(pd_cap_error("name", data.name.len(), MAX_PD_NAME_BYTES));
    }
    match &data.notes {
        Some(notes) if notes.len() > MAX_PD_NOTES_BYTES => Err(pd_cap_error(
            "notes",
            notes.len(),
            MAX_PD_NOTES_BYTES,
        )),
        _ => Ok(()),
    }
}

// IEC 62386-102 §9.18
fn validate_pd_override_within_declared_types(
    declared: Option<Vec<u8>>,
    data: &PdPatchData,
) -> Result<(), HttpResponse> {
    use dali2rust_contracts::msg::PhysicalDeviceOverrideCommand as Patch;
    if data.patch_mask & Patch::PATCH_DEVICE_TYPE_OVERRIDE == 0 || data.clear_dt {
        return Ok(());
    }
    let (Some(declared), Some(wanted)) = (declared, data.dt_override.dali_code()) else {
        return Ok(());
    };
    if declared.contains(&wanted) {
        return Ok(());
    }
    Err(json_err(422, "invalid_value"))
}

fn parse_pd_override_field<T>(
    value: &serde_json::Value,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, HttpResponse> {
    if value.is_null() {
        return Ok(None);
    }
    let s = value.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
    Ok(Some(parse(s).ok_or_else(|| json_err(422, "invalid_enum"))?))
}

fn pd_patch_text(value: &serde_json::Value) -> Result<String, HttpResponse> {
    if value.is_null() {
        return Ok(String::new());
    }
    Ok(value
        .as_str()
        .ok_or_else(|| json_err(422, "invalid_value"))?
        .to_string())
}

fn parse_pd_patch_fields(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<PdPatchData, HttpResponse> {
    use dali2rust_contracts::msg::PhysicalDeviceOverrideCommand as Patch;
    let mut data = PdPatchData {
        patch_mask: 0,
        name: String::new(),
        notes: None,
        clear_dt: false,
        dt_override: DeviceType::Unknown,
        clear_cm: false,
        cm_override: ColorMode::Unknown,
        dt8_auto_activation_repair: true,
        dt8_rgbwaf_control_assert: true,
    };
    if let Some(v) = obj.get("name") {
        data.patch_mask |= Patch::PATCH_NAME;
        data.name = pd_patch_text(v)?;
    }
    if let Some(v) = obj.get("notes") {
        data.notes = Some(pd_patch_text(v)?);
    }
    if let Some(v) = obj.get("device_type_override") {
        data.patch_mask |= Patch::PATCH_DEVICE_TYPE_OVERRIDE;
        match parse_pd_override_field(v, parse_device_type)? {
            None => data.clear_dt = true,
            Some(dt) => data.dt_override = dt,
        }
    }
    if let Some(v) = obj.get("color_mode_override") {
        data.patch_mask |= Patch::PATCH_COLOR_MODE_OVERRIDE;
        match parse_pd_override_field(v, parse_color_mode)? {
            None => data.clear_cm = true,
            Some(cm) => data.cm_override = cm,
        }
    }
    parse_pd_dt8_permissions(obj, &mut data)?;
    Ok(data)
}

fn parse_pd_dt8_permissions(
    obj: &serde_json::Map<String, serde_json::Value>,
    data: &mut PdPatchData,
) -> Result<(), HttpResponse> {
    use dali2rust_contracts::msg::PhysicalDeviceOverrideCommand as Patch;
    if let Some(v) = obj.get("dt8_auto_activation_repair") {
        data.patch_mask |= Patch::PATCH_DT8_AUTO_ACTIVATION_REPAIR;
        data.dt8_auto_activation_repair = v.as_bool().ok_or_else(|| json_err(422, "invalid_value"))?;
    }
    if let Some(v) = obj.get("dt8_rgbwaf_control_assert") {
        data.patch_mask |= Patch::PATCH_DT8_RGBWAF_CONTROL_ASSERT;
        data.dt8_rgbwaf_control_assert = v.as_bool().ok_or_else(|| json_err(422, "invalid_value"))?;
    }
    Ok(())
}

fn parse_discovery_mode(s: &str) -> Option<DiscoveryMode> {
    match s {
        "scan_known_short_addresses" => Some(DiscoveryMode::ScanKnownShortAddresses),
        "commission_unaddressed" => Some(DiscoveryMode::CommissionUnaddressed),
        "refresh_known" => Some(DiscoveryMode::RefreshKnown),
        _ => None,
    }
}

fn parse_memory_banks_preset(s: &str) -> Option<MemoryBankReadPreset> {
    match s {
        "none" => Some(MemoryBankReadPreset::None),
        "identity" => Some(MemoryBankReadPreset::Identity),
        "profile" => Some(MemoryBankReadPreset::Profile),
        "all" => Some(MemoryBankReadPreset::All),
        "power" => Some(MemoryBankReadPreset::Power),
        "energy" => Some(MemoryBankReadPreset::Energy),
        "diagnostics" => Some(MemoryBankReadPreset::Diagnostics),
        "luminaire_data" => Some(MemoryBankReadPreset::LuminaireData),
        _ => None,
    }
}
