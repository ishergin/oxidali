use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_api::bus_codec::SOURCE_ID_UNSPECIFIED;
use dali2rust_api::confirmation_bridge::{spawn_confirmation_bridge, PendingConfirmationSlots};
use dali2rust_api::http::dispatcher::CorrelationIdAllocator;
use dali2rust_api::http::handler::ApiHandler;
use dali2rust_api::http::handlers::physical_devices::{
    AdapterDiscoveryRunsHandler, PhysicalDeviceAttributeReadsHandler, PhysicalDeviceGetHandler,
    PhysicalDevicePatchHandler, PhysicalDeviceTargetStateHandler,
    PhysicalDeviceWriteAttributesHandler,
};
use dali2rust_api::http::handlers::virtual_lamps::{
    VirtualLampBindingDeleteHandler, VirtualLampBindingPutHandler, VirtualLampPatchHandler,
    VirtualLampTargetStateHandler,
};
use dali2rust_api::http::adapter_state::AdapterSettingsApplyWatch;
use dali2rust_api::http::physical_device_state::{
    CapabilityFlagsDto, MemoryBankSummaryDto, PhysicalDeviceCoreDto, PhysicalDeviceHttpState,
    PhysicalDevicePatchWatch, PhysicalDeviceSummaryDto,
    PhysicalDeviceStateDto,
};
use dali2rust_api::http::types::HttpResponse;
use dali2rust_domain::registry::{AttributeSectionKind, AttributeSectionView};
use dali2rust_api::http::virtual_lamp_state::{
    VirtualLampBindingApplyWatch, VirtualLampBindingDto, VirtualLampDto, VirtualLampHttpState,
    VirtualLampPatchWatch,
};
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::bus::build_confirmation_envelope;
use dali2rust_contracts::msg::{BusCommandPayload, CommandEnvelope, DeliveryStatus};
use serde_json::Value;

struct IncrementingApplyWatch(AtomicU32);

impl PhysicalDevicePatchWatch for IncrementingApplyWatch {
    fn physical_override_applied_load(&self) -> u32 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }
}

impl VirtualLampPatchWatch for IncrementingApplyWatch {
    fn virtual_lamp_metadata_applied_load(&self) -> u32 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }
}

impl VirtualLampBindingApplyWatch for IncrementingApplyWatch {
    fn virtual_lamp_binding_applied_load(&self) -> u32 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }
}

impl AdapterSettingsApplyWatch for IncrementingApplyWatch {
    fn adapter_settings_applied_load(&self) -> u32 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }
}

fn immediate_apply_watch() -> Arc<IncrementingApplyWatch> {
    Arc::new(IncrementingApplyWatch(AtomicU32::new(0)))
}

fn params(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn response_json(resp: HttpResponse) -> Value {
    serde_json::from_slice(&resp.into_body_bytes()).expect("json response")
}

fn error_code(resp: HttpResponse) -> String {
    response_json(resp)
        .get("error")
        .and_then(|v| v.as_str())
        .expect("error code")
        .to_string()
}

fn base_state() -> PhysicalDeviceStateDto {
    PhysicalDeviceStateDto {
        power: "unknown".to_string(),
        level: None,
        color_mode: "unknown".to_string(),
        color_temperature_kelvin: None,
        xy: None,
        rgb: None,
        waf: None,
        status: None,
        failure_status: None,
        value_source: None,
        last_seen_ms: None,
        last_dapc_source: None,
        error: None,
    }
}

fn base_caps(cct: bool, xy: bool, rgb: bool) -> CapabilityFlagsDto {
    CapabilityFlagsDto {
        brightness: true,
        cct,
        xy,
        rgb,
        rgbwaf: false,
        scenes: false,
        groups: false,
    }
}

fn sample_pd(short_address: u8, cct: bool, xy: bool, rgb: bool) -> PhysicalDeviceCoreDto {
    PhysicalDeviceCoreDto {
        now_ms: 0,
        adapter_id: 0,
        short_address,
        random_address: None,
        name: format!("pd-{short_address}"),
        notes: None,
        device_type_discovered: "dt6_led",
        device_type_override: None,
        device_type_effective: "dt6_led",
        device_type_source: "discovered",
        supported_device_types: None,
        extended_versions: Vec::new(),
        color_mode_discovered: "brightness",
        color_mode_override: None,
        color_mode_effective: "brightness",
        color_mode_source: "discovered",
        dt8_auto_activation_repair: true,
        dt8_rgbwaf_control_assert: true,
        state: base_state(),
        capabilities: base_caps(cct, xy, rgb),
        color_temperature_range: None,
    }
}

fn sample_vl(lamp_id: u8, binding_short: Option<u8>, cct: bool, xy: bool, rgb: bool) -> VirtualLampDto {
    VirtualLampDto {
        adapter_id: 0,
        virtual_lamp_id: lamp_id,
        name: format!("vl-{lamp_id}"),
        device_type_effective: "dt6_led".to_string(),
        device_type_source: "discovered".to_string(),
        color_mode_effective: "brightness".to_string(),
        color_mode_source: "discovered".to_string(),
        binding: binding_short.map(|physical_short_address| VirtualLampBindingDto {
            physical_short_address,
        }),
        ha_entity_enabled: true,
        state: base_state(),
        capabilities: base_caps(cct, xy, rgb),
        color_temperature_range: None,
    }
}

struct TestStateInner {
    adapter_count: u8,
    pds: HashMap<(u8, u8), PhysicalDeviceCoreDto>,
    vls: HashMap<(u8, u8), VirtualLampDto>,
    physical_short_other_adapter: bool,
}

struct TestState {
    inner: Mutex<TestStateInner>,
}

impl TestState {
    fn new() -> Self {
        Self {
            inner: Mutex::new(TestStateInner {
                adapter_count: 1,
                pds: HashMap::new(),
                vls: HashMap::new(),
                physical_short_other_adapter: false,
            }),
        }
    }

    fn with_pd(self, dto: PhysicalDeviceCoreDto) -> Self {
        self.inner
            .lock()
            .expect("state lock")
            .pds
            .insert((dto.adapter_id, dto.short_address), dto);
        self
    }

    fn with_vl(self, dto: VirtualLampDto) -> Self {
        self.inner
            .lock()
            .expect("state lock")
            .vls
            .insert((dto.adapter_id, dto.virtual_lamp_id), dto);
        self
    }

    fn set_conflict_short_on_other_adapter(&self, value: bool) {
        self.inner
            .lock()
            .expect("state lock")
            .physical_short_other_adapter = value;
    }

    fn set_virtual_lamp_binding(&self, adapter_id: u8, lamp_id: u8, binding_short: Option<u8>) {
        let mut guard = self.inner.lock().expect("state lock");
        if let Some(dto) = guard.vls.get_mut(&(adapter_id, lamp_id)) {
            dto.binding = binding_short.map(|physical_short_address| VirtualLampBindingDto {
                physical_short_address,
            });
        }
    }
}

impl PhysicalDeviceHttpState for TestState {
    fn adapter_count(&self) -> u8 {
        self.inner.lock().expect("state lock").adapter_count
    }

    fn physical_device_summary_dto(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceSummaryDto> {
        let core = self.physical_device_core_dto(adapter_id, short_address)?;
        Some(PhysicalDeviceSummaryDto {
            short_address: core.short_address,
            random_address: core.random_address,
            name: core.name,
            device_type_effective: core.device_type_effective,
            color_mode_effective: core.color_mode_effective,
            capabilities: core.capabilities,
            state: core.state,
            groups_membership: None,
            gtin: None,
            identification_number: None,
        })
    }

    fn physical_device_core_dto(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceCoreDto> {
        self.inner
            .lock()
            .expect("state lock")
            .pds
            .get(&(adapter_id, short_address))
            .cloned()
    }

    fn physical_device_attribute_section(
        &self,
        adapter_id: u8,
        short_address: u8,
        kind: AttributeSectionKind,
    ) -> Option<AttributeSectionView> {
        self.physical_device_exists(adapter_id, short_address)
            .then(|| match kind {
                AttributeSectionKind::Groups => {
                    AttributeSectionView::Groups(Box::default())
                }
                _ => AttributeSectionView::Common102(Box::default()),
            })
    }

    fn physical_device_memory_banks(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<Vec<MemoryBankSummaryDto>> {
        self.physical_device_exists(adapter_id, short_address)
            .then(Vec::new)
    }

    fn physical_device_capabilities(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<CapabilityFlagsDto> {
        self.physical_device_core_dto(adapter_id, short_address)
            .map(|dto| dto.capabilities)
    }

    fn physical_device_supported_types(&self, adapter_id: u8, short_address: u8) -> Option<Vec<u8>> {
        self.physical_device_core_dto(adapter_id, short_address)
            .and_then(|dto| dto.supported_device_types)
    }

    fn physical_device_exists(&self, adapter_id: u8, short_address: u8) -> bool {
        self.inner
            .lock()
            .expect("state lock")
            .pds
            .contains_key(&(adapter_id, short_address))
    }

    fn list_physical_device_short_addresses(&self, adapter_id: u8) -> Vec<u8> {
        let mut shorts: Vec<u8> = self
            .inner
            .lock()
            .expect("state lock")
            .pds
            .values()
            .filter(|dto| dto.adapter_id == adapter_id)
            .map(|dto| dto.short_address)
            .collect();
        shorts.sort_unstable();
        shorts
    }
}

impl VirtualLampHttpState for TestState {
    fn virtual_lamp_dto(&self, adapter_id: u8, lamp_id: u8) -> VirtualLampDto {
        self.inner
            .lock()
            .expect("state lock")
            .vls
            .get(&(adapter_id, lamp_id))
            .cloned()
            .unwrap_or_else(|| sample_vl(lamp_id, None, false, false, false))
    }

    fn list_virtual_lamp_ids(&self, adapter_id: u8) -> Vec<u8> {
        self.inner
            .lock()
            .expect("state lock")
            .vls
            .keys()
            .filter(|(aid, _)| *aid == adapter_id)
            .map(|(_, lamp_id)| *lamp_id)
            .collect()
    }

    fn physical_short_on_other_adapter(&self, _adapter_id: u8, _short_address: u8) -> bool {
        self.inner
            .lock()
            .expect("state lock")
            .physical_short_other_adapter
    }
}

struct BusHarness {
    publisher: dali2rust_bus::BusPublisher,
    cmd_rx: Arc<Mutex<Receiver<BusFrame>>>,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    confirmation_timeout_ms: u64,
    _bridge: std::thread::JoinHandle<()>,
    _host: BusHost,
}

impl BusHarness {
    fn new() -> Self {
        let config = BusConfig::default();
        let (host, publisher, (cmd_rx, conf_rx)) = BusHost::spawn(
            config,
            |reg| {
                (
                    reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                    reg.subscribe_confirmations(32),
                )
            },
        );
        let slots = Arc::new(PendingConfirmationSlots::new());
        let bridge = spawn_confirmation_bridge(conf_rx, Arc::clone(&slots));
        Self {
            publisher,
            cmd_rx: Arc::new(Mutex::new(cmd_rx)),
            slots,
            correlation: Arc::new(CorrelationIdAllocator::new()),
            confirmation_timeout_ms: config.confirmation_timeout_ms,
            _bridge: bridge,
            _host: host,
        }
    }

    fn confirmation_deadline(&self) -> Duration {
        Duration::from_millis(self.confirmation_timeout_ms)
    }

    fn next_command(&self) -> CommandEnvelope {
        let frame = self
            .cmd_rx
            .lock()
            .expect("cmd lock")
            .recv_timeout(self.confirmation_deadline())
            .expect("command frame");
        let BusFrame::Command(command) = frame else {
            panic!("expected command frame");
        };
        command.as_ref().clone()
    }

    fn ack_next_command(&self) -> std::thread::JoinHandle<CommandEnvelope> {
        let rx = Arc::clone(&self.cmd_rx);
        let publisher = self.publisher.clone();
        let deadline = self.confirmation_deadline();
        std::thread::spawn(move || {
            let frame = rx
                .lock()
                .expect("cmd lock")
                .recv_timeout(deadline)
                .expect("command frame");
            let BusFrame::Command(command) = frame else {
                panic!("expected command frame");
            };
            let command = command.as_ref().clone();
            assert_eq!(
                publisher.try_publish(
                    BusChannel::Confirmations,
                    BusFrame::confirmation(build_confirmation_envelope(
                        command.meta.correlation_id,
                        DeliveryStatus::Ok,
                        0,
                        SOURCE_ID_UNSPECIFIED,
                    )),
                ),
                PublishResult::Queued
            );
            command
        })
    }
}

#[test]
fn physical_device_patch_validation_errors() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_pd(sample_pd(1, false, false, false)));
    let handler = PhysicalDevicePatchHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        state,
        immediate_apply_watch(),
        Arc::new(FixedWall(123_456)),
        BusId::default(),
        harness.confirmation_timeout_ms,
    );
    let request_params = params(&[("adapter_id", "0"), ("short", "1")]);

    assert_eq!(
        error_code(handler.handle_request("PATCH", "", br#"{"boom":1}"#, &request_params)),
        "unknown_field"
    );
    assert_eq!(
        error_code(handler.handle_request("PATCH", "", br#"{"state":{}}"#, &request_params)),
        "unsupported_field"
    );
    let fade_resp = response_json(handler.handle_request(
        "PATCH",
        "",
        br#"{"fade_time_ms":100}"#,
        &request_params,
    ));
    assert_eq!(fade_resp["error"], "unsupported_field");
    assert_eq!(fade_resp["message"], "use POST .../write-attributes");
    assert_eq!(
        error_code(
            handler.handle_request("PATCH", "", br#"{"device_type_override":"bad"}"#, &request_params)
        ),
        "invalid_enum"
    );
    assert_eq!(
        error_code(handler.handle_request("PATCH", "", br#"{"name":"x""#, &request_params)),
        "invalid_json"
    );
    for body in [&br#"{"name":123}"#[..], &br#"{"name":{}}"#[..]] {
        assert_eq!(
            error_code(handler.handle_request("PATCH", "", body, &request_params)),
            "invalid_value",
            "a name that is neither a string nor null is a client error"
        );
    }
    assert_ne!(
        handler
            .handle_request("PATCH", "", br#"{"name":null}"#, &request_params)
            .status,
        422,
        "null is RFC 7396 remove, not a type error"
    );
    let over_cap = format!(r#"{{"name":"{}"}}"#, "Я".repeat(33));
    let resp = response_json(handler.handle_request(
        "PATCH",
        "",
        over_cap.as_bytes(),
        &request_params,
    ));
    assert_eq!(resp["error"], "invalid_value");
    let message = resp["message"].as_str().expect("message");
    assert!(
        message.contains("66 bytes") && message.contains("64 bytes"),
        "the 422 must name the actual size and the limit: {message}"
    );
    assert!(
        message.contains("UTF-8"),
        "and say the limit is in bytes, not characters: {message}"
    );
}

#[test]
fn physical_device_patch_accepts_a_full_width_cyrillic_name() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_pd(sample_pd(1, false, false, false)));
    let handler = PhysicalDevicePatchHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        state,
        immediate_apply_watch(),
        Arc::new(FixedWall(123_456)),
        BusId::default(),
        harness.confirmation_timeout_ms,
    );
    let request_params = params(&[("adapter_id", "0"), ("short", "1")]);
    let name = "Я".repeat(32);
    assert_eq!(name.len(), 64, "32 Cyrillic characters are exactly 64 bytes");
    let body = format!(r#"{{"name":"{name}"}}"#);

    let resp = handler.handle_request("PATCH", "", body.as_bytes(), &request_params);
    assert_ne!(resp.status, 422, "a 64-byte name is within the cap");

    let BusCommandPayload::PhysicalDeviceOverrideCommand(published) =
        harness.next_command().payload
    else {
        panic!("expected PhysicalDeviceOverrideCommand");
    };
    assert_eq!(
        published.name.as_str(),
        name,
        "the whole name must reach the wire, not a truncated prefix"
    );
}

#[test]
fn physical_device_patch_publishes_the_override_then_the_notes() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_pd(sample_pd(1, false, false, false)));
    let handler = PhysicalDevicePatchHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        state,
        immediate_apply_watch(),
        Arc::new(FixedWall(123_456)),
        BusId::default(),
        harness.confirmation_timeout_ms,
    );
    let request_params = params(&[("adapter_id", "0"), ("short", "1")]);

    let _ = handler.handle_request(
        "PATCH",
        "",
        br#"{"name":"Desk","notes":"bench"}"#,
        &request_params,
    );

    let BusCommandPayload::PhysicalDeviceOverrideCommand(first) = harness.next_command().payload
    else {
        panic!("the override command must go first");
    };
    assert_eq!(first.name.as_str(), "Desk");
    let BusCommandPayload::PhysicalDeviceNotesUpdateCommand(second) =
        harness.next_command().payload
    else {
        panic!("the notes command must follow");
    };
    assert_eq!(second.notes.as_str(), "bench");
    assert_eq!(second.short_address, 1);
}

#[test]
fn physical_device_write_attributes_validation_and_happy_path() {
    let harness = BusHarness::new();
    let handler = PhysicalDeviceWriteAttributesHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.correlation),
        BusId::default(),
        1,
    );
    let request_params = params(&[("adapter_id", "0"), ("short", "1")]);

    assert_eq!(
        error_code(handler.handle_request("POST", "", br#"{"name":"nope"}"#, &request_params)),
        "unsupported_field"
    );
    assert_eq!(
        error_code(handler.handle_request("POST", "", br#"{"fade_rate":16}"#, &request_params)),
        "invalid_value"
    );
    assert_eq!(
        error_code(handler.handle_request("POST", "", br#"{"power_on_level":255}"#, &request_params)),
        "invalid_value"
    );
    assert_eq!(
        error_code(
            handler.handle_request("POST", "", br#"{"system_failure_level":255}"#, &request_params)
        ),
        "invalid_value"
    );
    assert_eq!(
        error_code(
            handler.handle_request("POST", "", br#"{"extended_fade_time_ms":70000}"#, &request_params)
        ),
        "invalid_value"
    );

    let rx = Arc::clone(&harness.cmd_rx);
    let publisher = harness.publisher.clone();
    let deadline = harness.confirmation_deadline();
    let ack = std::thread::spawn(move || {
        let begin = rx
            .lock()
            .expect("cmd lock")
            .recv_timeout(deadline)
            .expect("begin command");
        assert!(matches!(begin, BusFrame::Command(_)));
        let frame = rx
            .lock()
            .expect("cmd lock")
            .recv_timeout(deadline)
            .expect("semantic command");
        let BusFrame::Command(command) = frame else {
            panic!("expected command frame");
        };
        let command = command.as_ref().clone();
        assert_eq!(
            publisher.try_publish(
                BusChannel::Confirmations,
                BusFrame::confirmation(build_confirmation_envelope(
                    command.meta.correlation_id,
                    DeliveryStatus::Ok,
                    0,
                    SOURCE_ID_UNSPECIFIED,
                )),
            ),
            PublishResult::Queued
        );
        command
    });
    let resp = response_json(handler.handle_request("POST", "", br#"{"fade_rate":7}"#, &request_params));
    assert_eq!(resp["type"], "attribute_write");
    assert!(resp["operation_id"].as_str().unwrap_or_default().starts_with("pd-wattr-0-1-"));
    let semantic = ack.join().expect("write attrs ack");
    let BusCommandPayload::DaliWriteAttributesCommand(body) = semantic.payload else {
        panic!("expected DaliWriteAttributesCommand");
    };
    assert_eq!(body.short_address, 1);
    assert_eq!(body.fade_rate, Some(7));
}

#[test]
fn physical_device_target_state_validation_and_happy_path() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_pd(sample_pd(2, false, false, false)));
    let handler = PhysicalDeviceTargetStateHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        state,
        Arc::new(FixedWall(123_456)),
        BusId::default(),
        harness.confirmation_timeout_ms,
    );

    assert_eq!(
        error_code(handler.handle_request("PUT", "", br#"{}"#, &params(&[("adapter_id", "0")]))),
        "missing_short_address"
    );
    assert_eq!(
        error_code(
            handler.handle_request("PUT", "", br#"{}"#, &params(&[("adapter_id", "0"), ("short", "99")]))
        ),
        "invalid_value"
    );
    assert_eq!(
        error_code(
            handler.handle_request("PUT", "", br#"{}"#, &params(&[("adapter_id", "0"), ("short", "9")]))
        ),
        "not_found"
    );

    let request_params = params(&[("adapter_id", "0"), ("short", "2")]);
    assert_eq!(
        error_code(handler.handle_request("PUT", "", br#"{"boom":1}"#, &request_params)),
        "unknown_field"
    );
    assert_eq!(
        error_code(handler.handle_request("PUT", "", br#"{"status":{"raw":0}}"#, &request_params)),
        "unsupported_field"
    );
    assert_eq!(
        error_code(handler.handle_request("PUT", "", br#"{"rgb":{"r":1,"g":2,"b":3}}"#, &request_params)),
        "unsupported_capability"
    );

    let ack = harness.ack_next_command();
    let resp = handler.handle_request("PUT", "", br#"{"power":"on","level":80}"#, &request_params);
    assert_eq!(resp.status, 200);
    let command = ack.join().expect("ack thread");
    let BusCommandPayload::DaliSetTargetStateCommand(body) = command.payload else {
        panic!("expected DaliSetTargetStateCommand");
    };
    assert_eq!(body.short_address, 2);
    assert_eq!(body.setpoint.level, 80);
}

#[test]
fn adapter_discovery_runs_contract() {
    let harness = BusHarness::new();
    let handler = AdapterDiscoveryRunsHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.correlation),
        BusId::default(),
        1,
    );
    let request_params = params(&[("adapter_id", "0")]);

    let resp = response_json(handler.handle_request(
        "POST",
        "",
        br#"{"mode":"scan_known_short_addresses"}"#,
        &request_params,
    ));
    assert_eq!(resp["type"], "discovery");
    assert!(resp["operation_id"].as_str().unwrap_or_default().starts_with("pd-disc-0-"));
    assert!(matches!(
        harness.next_command().payload,
        BusCommandPayload::OperationBeginCommand(_)
    ));
    assert!(matches!(
        harness.next_command().payload,
        BusCommandPayload::DaliDiscoverDevicesCommand(_)
    ));

    assert_eq!(
        error_code(handler.handle_request("POST", "", br#"{"mode":"nope"}"#, &request_params)),
        "invalid_enum"
    );
    assert_eq!(
        error_code(handler.handle_request("POST", "", br#"{"mode":"x""#, &request_params)),
        "invalid_json"
    );
}

#[test]
fn physical_device_attribute_reads_contract() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_pd(sample_pd(3, false, false, false)));
    let handler = PhysicalDeviceAttributeReadsHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.correlation),
        BusId::default(),
        state,
    );

    assert_eq!(
        error_code(
            handler.handle_request("POST", "", br#"{"attribute_groups":["common_102"]}"#, &params(&[("adapter_id", "0"), ("short", "9")]))
        ),
        "not_found"
    );
    let request_params = params(&[("adapter_id", "0"), ("short", "3")]);
    assert_eq!(
        error_code(handler.handle_request("POST", "", br#"{"attribute_groups":[]}"#, &request_params)),
        "invalid_value"
    );
    assert_eq!(
        error_code(handler.handle_request("POST", "", br#"{"attribute_groups":["bad"]}"#, &request_params)),
        "invalid_enum"
    );
    let resp = response_json(handler.handle_request(
        "POST",
        "",
        br#"{"attribute_groups":["common_102","groups"]}"#,
        &request_params,
    ));
    assert_eq!(resp["type"], "attribute_read");
    assert!(resp["operation_id"].as_str().unwrap_or_default().starts_with("pd-attr-0-3-"));
    assert!(matches!(
        harness.next_command().payload,
        BusCommandPayload::OperationBeginCommand(_)
    ));
    let semantic = harness.next_command();
    let BusCommandPayload::DaliReadAttributesCommand(body) = semantic.payload else {
        panic!("expected DaliReadAttributesCommand");
    };
    assert_eq!(body.short_address, 3);
    assert_eq!(body.attribute_groups_mask & (1 << 1), 1 << 1);
    assert_eq!(body.attribute_groups_mask & (1 << 4), 1 << 4);
    assert_eq!(
        body.memory_banks,
        dali2rust_contracts::msg::MemoryBankReadPreset::None
    );
}

#[test]
fn physical_device_attribute_reads_memory_bank_preset_contract() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_pd(sample_pd(4, false, false, false)));
    let handler = PhysicalDeviceAttributeReadsHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.correlation),
        BusId::default(),
        state,
    );
    let request_params = params(&[("adapter_id", "0"), ("short", "4")]);
    let resp = response_json(handler.handle_request(
        "POST",
        "",
        br#"{"attribute_groups":["common_102"],"memory_banks":"identity"}"#,
        &request_params,
    ));
    assert_eq!(resp["type"], "attribute_read");
    assert!(matches!(
        harness.next_command().payload,
        BusCommandPayload::OperationBeginCommand(_)
    ));
    let semantic = harness.next_command();
    let BusCommandPayload::DaliReadAttributesCommand(body) = semantic.payload else {
        panic!("expected DaliReadAttributesCommand");
    };
    assert_eq!(
        body.memory_banks,
        dali2rust_contracts::msg::MemoryBankReadPreset::Identity
    );
}

#[test]
fn virtual_lamp_patch_validation_errors() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_vl(sample_vl(1, None, false, false, false)));
    let handler = VirtualLampPatchHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        state,
        immediate_apply_watch(),
        BusId::default(),
        harness.confirmation_timeout_ms,
    );
    let request_params = params(&[("adapter_id", "0"), ("lamp_id", "1")]);

    assert_eq!(
        error_code(handler.handle_request("PATCH", "", br#"{"boom":1}"#, &request_params)),
        "unknown_field"
    );
    assert_eq!(
        error_code(handler.handle_request("PATCH", "", br#"{"binding":{}}"#, &request_params)),
        "unsupported_field"
    );
    assert_eq!(
        error_code(handler.handle_request("PATCH", "", br#"{"name":""}"#, &request_params)),
        "invalid_value"
    );
    assert_eq!(
        error_code(
            handler.handle_request(
                "PATCH",
                "",
                format!(r#"{{"name":"{}"}}"#, "x".repeat(65)).as_bytes(),
                &request_params,
            )
        ),
        "invalid_value"
    );
    assert_eq!(
        error_code(
            handler.handle_request("PATCH", "", br#"{"declared_type":"dt8_color"}"#, &request_params)
        ),
        "unknown_field"
    );
    assert_eq!(
        error_code(
            handler.handle_request(
                "PATCH",
                "",
                br#"{"declared_color_mode":"cct"}"#,
                &request_params,
            )
        ),
        "unknown_field"
    );
    assert_eq!(
        error_code(handler.handle_request("PATCH", "", br#"{}"#, &request_params)),
        "invalid_json"
    );
}

#[test]
fn virtual_lamp_binding_put_validates_and_selects_bind_vs_rebind() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_vl(sample_vl(2, None, false, false, false)));
    let state_trait: Arc<dyn VirtualLampHttpState> = state.clone();
    let handler = VirtualLampBindingPutHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        state_trait,
        immediate_apply_watch(),
        BusId::default(),
        harness.confirmation_timeout_ms,
    );
    let request_params = params(&[("adapter_id", "0"), ("lamp_id", "2")]);

    assert_eq!(
        error_code(handler.handle_request("PUT", "", br#"{"short":1}"#, &request_params)),
        "unknown_field"
    );
    assert_eq!(
        error_code(
            handler.handle_request(
                "PUT",
                "",
                br#"{"physical_short_address":"x"}"#,
                &request_params,
            )
        ),
        "invalid_value"
    );
    assert_eq!(
        error_code(
            handler.handle_request(
                "PUT",
                "",
                br#"{"physical_short_address":64}"#,
                &request_params,
            )
        ),
        "invalid_value"
    );
    state.set_conflict_short_on_other_adapter(true);
    assert_eq!(
        error_code(
            handler.handle_request(
                "PUT",
                "",
                br#"{"physical_short_address":7}"#,
                &request_params,
            )
        ),
        "conflict"
    );
    state.set_conflict_short_on_other_adapter(false);

    let rx = Arc::clone(&harness.cmd_rx);
    let publisher = harness.publisher.clone();
    let deadline = harness.confirmation_deadline();
    let state_for_bind = Arc::clone(&state);
    let bind_ack = std::thread::spawn(move || {
        let frame = rx
            .lock()
            .expect("cmd lock")
            .recv_timeout(deadline)
            .expect("command frame");
        let BusFrame::Command(command) = frame else {
            panic!("expected command frame");
        };
        let command = command.as_ref().clone();
        state_for_bind.set_virtual_lamp_binding(0, 2, Some(7));
        assert_eq!(
            publisher.try_publish(
                BusChannel::Confirmations,
                BusFrame::confirmation(build_confirmation_envelope(
                    command.meta.correlation_id,
                    DeliveryStatus::Ok,
                    0,
                    SOURCE_ID_UNSPECIFIED,
                )),
            ),
            PublishResult::Queued
        );
        command
    });
    let bind_resp = handler.handle_request(
        "PUT",
        "",
        br#"{"physical_short_address":7}"#,
        &request_params,
    );
    assert_eq!(bind_resp.status, 200);
    let bind_command = bind_ack.join().expect("bind ack");
    assert!(matches!(
        bind_command.payload,
        BusCommandPayload::VirtualLampBindCommand(_)
    ));

    state.set_virtual_lamp_binding(0, 2, Some(3));
    let rx = Arc::clone(&harness.cmd_rx);
    let publisher = harness.publisher.clone();
    let deadline = harness.confirmation_deadline();
    let state_for_rebind = Arc::clone(&state);
    let rebind_ack = std::thread::spawn(move || {
        let frame = rx
            .lock()
            .expect("cmd lock")
            .recv_timeout(deadline)
            .expect("command frame");
        let BusFrame::Command(command) = frame else {
            panic!("expected command frame");
        };
        let command = command.as_ref().clone();
        state_for_rebind.set_virtual_lamp_binding(0, 2, Some(9));
        assert_eq!(
            publisher.try_publish(
                BusChannel::Confirmations,
                BusFrame::confirmation(build_confirmation_envelope(
                    command.meta.correlation_id,
                    DeliveryStatus::Ok,
                    0,
                    SOURCE_ID_UNSPECIFIED,
                )),
            ),
            PublishResult::Queued
        );
        command
    });
    let rebind_resp = handler.handle_request(
        "PUT",
        "",
        br#"{"physical_short_address":9}"#,
        &request_params,
    );
    assert_eq!(rebind_resp.status, 200);
    let rebind_command = rebind_ack.join().expect("rebind ack");
    assert!(matches!(
        rebind_command.payload,
        BusCommandPayload::VirtualLampRebindCommand(_)
    ));
}

#[test]
fn virtual_lamp_binding_delete_returns_updated_dto() {
    let harness = BusHarness::new();
    let state = Arc::new(TestState::new().with_vl(sample_vl(3, Some(11), false, false, false)));
    let state_trait: Arc<dyn VirtualLampHttpState> = state.clone();
    let handler = VirtualLampBindingDeleteHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        state_trait,
        immediate_apply_watch(),
        BusId::default(),
        harness.confirmation_timeout_ms,
    );
    let request_params = params(&[("adapter_id", "0"), ("lamp_id", "3")]);

    let rx = Arc::clone(&harness.cmd_rx);
    let publisher = harness.publisher.clone();
    let deadline = harness.confirmation_deadline();
    let state_for_delete = Arc::clone(&state);
    let ack = std::thread::spawn(move || {
        let frame = rx
            .lock()
            .expect("cmd lock")
            .recv_timeout(deadline)
            .expect("command frame");
        let BusFrame::Command(command) = frame else {
            panic!("expected command frame");
        };
        let command = command.as_ref().clone();
        state_for_delete.set_virtual_lamp_binding(0, 3, None);
        assert_eq!(
            publisher.try_publish(
                BusChannel::Confirmations,
                BusFrame::confirmation(build_confirmation_envelope(
                    command.meta.correlation_id,
                    DeliveryStatus::Ok,
                    0,
                    SOURCE_ID_UNSPECIFIED,
                )),
            ),
            PublishResult::Queued
        );
        command
    });

    let resp = response_json(handler.handle_request("DELETE", "", &[], &request_params));
    assert!(resp.get("binding").is_none());
    let command = ack.join().expect("delete ack");
    assert!(matches!(
        command.payload,
        BusCommandPayload::VirtualLampUnbindCommand(_)
    ));
}

#[test]
fn virtual_lamp_target_state_validation_and_execute() {
    let harness = BusHarness::new();
    let unbound_state = Arc::new(TestState::new().with_vl(sample_vl(4, None, false, false, false)));
    let unbound_trait: Arc<dyn VirtualLampHttpState> = unbound_state.clone();
    let unbound = VirtualLampTargetStateHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        unbound_trait,
        BusId::default(),
        harness.confirmation_timeout_ms,
    );
    let request_params = params(&[("adapter_id", "0"), ("lamp_id", "4")]);

    assert_eq!(
        error_code(unbound.handle_request("PUT", "", br#"{"boom":1}"#, &request_params)),
        "unknown_field"
    );
    assert_eq!(
        error_code(
            unbound.handle_request("PUT", "", br#"{"status":{"raw":0}}"#, &request_params)
        ),
        "unsupported_field"
    );
    assert_eq!(
        error_code(
            unbound.handle_request("PUT", "", br#"{"rgb":{"r":1,"g":2,"b":3}}"#, &request_params)
        ),
        "unsupported_capability"
    );
    let unbound_resp =
        response_json(unbound.handle_request("PUT", "", br#"{"power":"on","level":50}"#, &request_params));
    assert!(unbound_resp.get("binding").is_none());

    let bound_state = Arc::new(TestState::new().with_vl(sample_vl(5, Some(12), true, true, true)));
    let bound_trait: Arc<dyn VirtualLampHttpState> = bound_state.clone();
    let bound = VirtualLampTargetStateHandler::new(
        harness.publisher.clone(),
        Arc::clone(&harness.slots),
        Arc::clone(&harness.correlation),
        bound_trait,
        BusId::default(),
        harness.confirmation_timeout_ms,
    );
    let ack = harness.ack_next_command();
    let bound_resp = bound.handle_request(
        "PUT",
        "",
        br#"{"power":"on","level":33}"#,
        &params(&[("adapter_id", "0"), ("lamp_id", "5")]),
    );
    assert_eq!(bound_resp.status, 200);
    let command = ack.join().expect("target state ack");
    let BusCommandPayload::DaliSetTargetStateCommand(body) = command.payload else {
        panic!("expected DaliSetTargetStateCommand");
    };
    assert_eq!(body.virtual_lamp_id, 5);
    assert_eq!(body.setpoint.level, 33);
}

struct FixedWall(u64);
impl dali2rust_platform::clock::UnixTimeMs for FixedWall {
    fn unix_millis(&self) -> u64 {
        self.0
    }
}

#[test]
fn physical_device_get_stamps_device_relative_now_ms() {
    let state = Arc::new(TestState::new().with_pd(sample_pd(7, false, false, false)));
    let handler = PhysicalDeviceGetHandler::new(state, Arc::new(FixedWall(123_456)));
    let mut params = HashMap::new();
    params.insert("adapter_id".to_string(), "0".to_string());
    params.insert("short".to_string(), "7".to_string());
    let resp = handler.handle_request("GET", "/api/v1/adapters/0/physical-devices/7", &[], &params);
    assert_eq!(resp.status, 200);
    let json: serde_json::Value =
        serde_json::from_slice(&resp.into_body_bytes()).expect("device JSON");
    assert_eq!(json["now_ms"].as_u64(), Some(123_456));
}
