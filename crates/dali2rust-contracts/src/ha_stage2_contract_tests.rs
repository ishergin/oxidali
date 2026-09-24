use crate::msg::CommandEnvelope;
use crate::msg::{
    BusEnvelope, ChannelKind, ColorMode, ColorValue, ErrorCode, ErrorPayload, LightSetpoint,
    MessageKind, Origin, PowerState, RegistryRuntimeUpdateCommand, RuntimeObservation,
    RuntimeRegistryUpdateEntry, RuntimeSource, SceneRow, StatusFlags,
};

const FROZEN_COMMAND_ORDER: [&str; 68] = [
    "DaliCommandPayload",
    "AdapterSettingsUpdateCommand",
    "GroupMetadataUpdateCommand",
    "GroupMatrixDesiredPatchCommand",
    "GroupMatrixDesiredReplaceCommand",
    "RegistryRuntimeUpdateCommand",
    "VirtualLampConfigUpdateCommand",
    "OperationBeginCommand",
    "OperationRegistryResetCommand",
    "PhysicalDeviceOverrideCommand",
    "DaliSetTargetStateCommand",
    "DaliWriteAttributesCommand",
    "DaliDiscoverDevicesCommand",
    "DaliReadAttributesCommand",
    "DaliReadMemoryBankCommand",
    "DaliProgramGroupMembershipCommand",
    "VirtualLampBindCommand",
    "VirtualLampRebindCommand",
    "VirtualLampUnbindCommand",
    "GroupApplyExecuteCommand",
    "SceneMetadataUpdateCommand",
    "SceneMatrixDesiredPatchCommand",
    "SceneMatrixDesiredReplaceCommand",
    "DaliProgramSceneCommand",
    "DaliRecallSceneCommand",
    "SceneApplyExecuteCommand",
    "DaliIdentifyDeviceCommand",
    "DaliAddressingCommand",
    "DaliCommissioningStepCommand",
    "DaliReplaceDeviceCommand",
    "HclScheduleUpsertCommand",
    "HclScheduleDeleteCommand",
    "DaliRecallLastActiveLevelCommand",
    "PollerSettingsUpdateCommand",
    "HclOverrideClearCommand",
    "PhysicalDeviceNotesUpdateCommand",
    "ConfigWriteCommitCommand",
    "HomeAssistantSettingsUpdateCommand",
    "HomeAssistantCredentialsUpdateCommand",
    "HomeAssistantTopicsUpdateCommand",
    "HomeAssistantControllerIdUpdateCommand",
    "HomeAssistantDiscoveryPublishCommand",
    "DaliSettingsUpdateCommand",
    "DaliBusHealthProbeCommand",
    "Dali103ScanCommand",
    "Dali103CommissionCommand",
    "Dali103InstanceConfigureCommand",
    "Dali103IdentifyCommand",
    "InputDeviceMetadataUpdateCommand",
    "InputDeviceNotesUpdateCommand",
    "Dali103FeedbackConfigureCommand",
    "Dali103FeedbackDriveCommand",
    "RuleStageCommand",
    "RuleCommitCommand",
    "RuleEnableCommand",
    "RuleRunCommand",
    "MqttPublishCommand",
    "VirtualLampDeleteCommand",
    "PhysicalDeviceDeleteCommand",
    "FirmwareUpdateBeginCommand",
    "RedundancySettingsUpdateCommand",
    "Dali103ArbitrationProbeCommand",
    "Dali103HandoverCommand",
    "PoliciesUpdateCommand",
    "PolicyApplyExecuteCommand",
    "RegistrySliceReloadCommand",
    "DaliStopFadeCommand",
    "RegistryLevelTransitionCommand",
];

#[test]
fn bus_command_payload_variant_order_is_frozen() {
    assert_eq!(
        crate::msg::COMMAND_VARIANT_NAMES,
        &FROZEN_COMMAND_ORDER,
        "BusCommandPayload wire order changed: appending is allowed (extend the snapshot), \
         inserting or reordering is a silent wire break"
    );
}

const FROZEN_EVENT_ORDER: [&str; 53] = [
    "DaliEventPayload",
    "IpAddressAssignedEvent",
    "RuntimeStateChangedEvent",
    "AdapterSettingsChangedEvent",
    "StatsReportedEvent",
    "OperationStatusChangedEvent",
    "OperationWorkerSignalEvent",
    "PhysicalDeviceChangedEvent",
    "VirtualLampChangedEvent",
    "GroupChangedEvent",
    "GroupMatrixChangedEvent",
    "DaliAttributesWrittenEvent",
    "DaliTargetStateAppliedEvent",
    "DaliTargetStateFailedEvent",
    "DaliGroupMembershipProgrammedEvent",
    "DaliAttributesReadEvent",
    "DaliMemoryBankReadEvent",
    "DaliMemoryBankReadAbortedEvent",
    "DaliDiscoveryProgressEvent",
    "DaliDiscoveryCompletedEvent",
    "DaliDiscoveryFailedEvent",
    "PersistenceLoadResultEvent",
    "DaliDiscoveryScanReconciledEvent",
    "DaliAttributeReadOutcomesEvent",
    "SceneChangedEvent",
    "SceneMatrixChangedEvent",
    "DaliSceneProgrammedEvent",
    "DaliSceneRecalledEvent",
    "DaliObservedFrameEvent",
    "DaliDeviceIdentifiedEvent",
    "DaliAddressingCompletedEvent",
    "DaliDeviceReplacedEvent",
    "HclScheduleChangedEvent",
    "PollerSettingsChangedEvent",
    "HomeAssistantSettingsChangedEvent",
    "HomeAssistantDiscoveryPublishedEvent",
    "DaliSettingsChangedEvent",
    "DaliBusHealthProbedEvent",
    "DaliInputEventObservedEvent",
    "DaliInputDeviceLifecycleEvent",
    "Dali103ScanProgressEvent",
    "Dali103InstanceConfiguredEvent",
    "InputDeviceChangedEvent",
    "Dali103ApplicationControlObservedEvent",
    "RulesChangedEvent",
    "RulesActivationEvent",
    "Dali103ScanStartedEvent",
    "RedundancySettingsChangedEvent",
    "RedundancyTransitionEvent",
    "Dali103ArbitrationProbedEvent",
    "PoliciesChangedEvent",
    "Dali103HandoverSentEvent",
    "RegistrySliceReloadedEvent",
];

#[test]
fn bus_event_payload_variant_order_is_frozen() {
    assert_eq!(
        crate::msg::EVENT_VARIANT_NAMES,
        &FROZEN_EVENT_ORDER,
        "BusEventPayload wire order changed: appending is allowed (extend the snapshot), \
         inserting or reordering is a silent wire break"
    );
}

#[test]
fn light_setpoint_serde_roundtrip() {
    let sample = LightSetpoint {
        power: PowerState::On,
        level: 180,
        color: Some(ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 3000,
            x: 0,
            y: 0,
            r: 0,
            g: 0,
            b: 0,
            w: 0,
            a: 0,
            f: 0,
        }),
    };
    let json = serde_json::to_string(&sample).expect("serialize");
    let back: LightSetpoint = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.power, PowerState::On);
    assert_eq!(back.level, 180);
    let color = back.color.expect("color");
    assert_eq!(color.mode, ColorMode::Cct);
    assert_eq!(color.color_temperature_kelvin, 3000);
}

#[test]
fn runtime_observation_serde_roundtrip() {
    let obs = RuntimeObservation {
        status_flags: Some(StatusFlags {
            raw: 0,
            lamp_failure: false,
            gear_failure: false,
            lamp_on: false,
            limit_error: false,
            fade_running: false,
            reset_state: false,
            missing_short_address: false,
            power_cycle_seen: false,
        }),
        failure_status: None,
        value_source: Some(RuntimeSource::Poller),
        last_seen_ms: Some(123456),
        last_dapc_source: crate::msg::LastDapcSource::Unknown,
        error: None,
    };
    let json = serde_json::to_string(&obs).expect("serialize");
    let back: RuntimeObservation = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.last_seen_ms, Some(123456));
    assert_eq!(back.value_source, Some(RuntimeSource::Poller));
}

#[test]
fn scene_row_serde_roundtrip() {
    let row = SceneRow {
        included: true,
        setpoint: Some(LightSetpoint {
            power: PowerState::On,
            level: 179,
            color: Some(ColorValue {
                mode: ColorMode::Cct,
                color_temperature_kelvin: 2700,
                x: 0,
                y: 0,
                r: 0,
                g: 0,
                b: 0,
                w: 0,
                a: 0,
                f: 0,
            }),
        }),
        capabilities: crate::msg::CapabilityFlags {
            brightness: true,
            cct: true,
            xy: false,
            rgb: false,
            rgbwaf: false,
            scenes: true,
            groups: true,
        },
        dirty: false,
    };
    let json = serde_json::to_string(&row).expect("serialize");
    let back: SceneRow = serde_json::from_str(&json).expect("deserialize");
    assert!(back.included);
    assert_eq!(back.setpoint.expect("included row has setpoint").level, 179);

    let excluded = SceneRow {
        included: false,
        setpoint: None,
        ..row
    };
    let json = serde_json::to_string(&excluded).expect("serialize");
    assert!(json.contains("\"setpoint\":null"));
    let back: SceneRow = serde_json::from_str(&json).expect("deserialize");
    assert!(!back.included);
    assert_eq!(back.setpoint, None);
}

#[test]
fn error_payload_serde_roundtrip() {
    let err = ErrorPayload::with_details(
        ErrorCode::UnsupportedCapability,
        "Virtual lamp 12 does not support rgb",
        &[1, 2, 3],
    );
    let json = serde_json::to_string(&err).expect("serialize");
    let back: ErrorPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.code, ErrorCode::UnsupportedCapability);
    assert_eq!(back.message, "Virtual lamp 12 does not support rgb");
}

#[test]
fn execution_failed_error_payload_roundtrip() {
    let err = ErrorPayload::new(ErrorCode::ExecutionFailed, "execution_failed");
    let json = serde_json::to_string(&err).expect("serialize");
    let back: ErrorPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.code, ErrorCode::ExecutionFailed);
    assert_eq!(back.message, "execution_failed");
}

#[test]
fn every_attribute_read_chunk_variant_fits_wire_budget() {
    for chunk in crate::msg::payload_test_samples::worst_attribute_read_chunks() {
        let ev = crate::msg::DaliAttributesReadEvent {
            registry_adapter_id: u8::MAX,
            short_address: 63,
            last_chunk: true,
            chunk: chunk.clone(),
        };
        let len = crate::msg::payload_test_samples::event_probe(&ev.into());
        assert!(
            len <= crate::bus::MAX_BUS_WIRE_BYTES,
            "chunk {chunk:?}: envelope {len} B exceeds {}",
            crate::bus::MAX_BUS_WIRE_BYTES
        );
    }
}

#[test]
fn compact_error_payload_roundtrip_and_truncation() {
    let long = "x".repeat(40);
    let compact = crate::msg::CompactErrorPayload::new(ErrorCode::OperationFailed, long.as_str());
    assert_eq!(compact.message.len(), 32, "message truncates at 32 chars");
    let json = serde_json::to_string(&compact).expect("serialize");
    let back: crate::msg::CompactErrorPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, compact);

    let full = ErrorPayload::with_details(ErrorCode::UnsupportedCapability, long, &[1, 2, 3]);
    let converted = crate::msg::CompactErrorPayload::from(&full);
    assert_eq!(converted.code, ErrorCode::UnsupportedCapability);
    assert_eq!(converted.message.as_str(), &full.message.as_str()[..32]);
}

#[test]
fn bus_envelope_extended_fields_serde_roundtrip() {
    let meta = BusEnvelope {
        schema_version: 1,
        channel_kind: ChannelKind::Commands,
        message_kind: MessageKind::Command,
        sender_id: 9,
        correlation_id: 4242,
        sequence_no: 1,
        target_adapter_id: 2,
        origin: Origin::Api,
        timestamp_ms: 88,
        bus_id: 1,
        cluster_origin_id: 0,
        adapter_proxy_origin_id: 0,
        payload_table_id: 42,
        error: None,
    };
    let json = serde_json::to_string(&meta).expect("serialize");
    let parsed: BusEnvelope = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.correlation_id, 4242);
    assert_eq!(parsed.origin, Origin::Api);
    assert_eq!(parsed.bus_id, 1u32);
    assert_eq!(parsed.payload_table_id, 42);
}

#[test]
fn registry_runtime_update_command_serde_roundtrip() {
    let cmd = RegistryRuntimeUpdateCommand {
        adapter_id: 0,
        update: RuntimeRegistryUpdateEntry {
            virtual_lamp_id: Some(12),
            short_address: None,
            setpoint: Some(LightSetpoint {
                power: PowerState::On,
                level: 200,
                color: None,
            }),
            observation: None,
            last_dapc_source: None,
            source: RuntimeSource::Sniffer,
            observed_at_mono_ms: Some(7_654_321),
        },
    };
    let json = serde_json::to_string(&cmd).expect("serialize");
    let parsed: RegistryRuntimeUpdateCommand = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.adapter_id, 0);
    assert_eq!(parsed.update.virtual_lamp_id, Some(12));
    assert_eq!(parsed.update.observed_at_mono_ms, Some(7_654_321));
}

#[test]
fn registry_runtime_update_level_envelope_postcard_reasonable_size() {
    let ce = crate::bus::command_envelope(
        0,
        9212,
        1,
        Some(Origin::Internal),
        RegistryRuntimeUpdateCommand::internal(
            0,
            crate::msg::RuntimeRegistryUpdateEntry::sniffer_level(12, 180, 5000),
        ),
    );
    let v = postcard::to_allocvec(&ce).expect("postcard");
    assert!(
        v.len() <= crate::bus::MAX_BUS_WIRE_BYTES,
        "envelope postcard len {} exceeds budget {}",
        v.len(),
        crate::bus::MAX_BUS_WIRE_BYTES
    );
    assert!(
        crate::bus::enforce_bus_wire_budget(v.len()).is_ok(),
        "budget enforcement rejected {} bytes",
        v.len()
    );
    let back: CommandEnvelope = postcard::from_bytes(&v).expect("roundtrip");
    assert_eq!(back.meta.correlation_id, 9212);
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Eq, Debug)]
struct JsonFlatLightSetpoint {
    power: String,
    level: u8,
    color_mode: String,
    #[serde(default)]
    color_temperature_kelvin: Option<u16>,
    xy: Option<JsonXy>,
    rgb: Option<JsonRgb>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Eq, Debug)]
struct JsonXy {
    x: u16,
    y: u16,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Eq, Debug)]
struct JsonRgb {
    r: u8,
    g: u8,
    b: u8,
}

#[test]
fn json_light_setpoint_fixture_roundtrip() {
    let json = r#"{
        "power": "on",
        "level": 180,
        "color_mode": "cct",
        "color_temperature_kelvin": 3000,
        "xy": null,
        "rgb": null
    }"#;
    let v: JsonFlatLightSetpoint = serde_json::from_str(json).expect("serde parse");
    assert_eq!(v.power, "on");
    assert_eq!(v.level, 180);
    assert_eq!(v.color_mode, "cct");
    assert_eq!(v.color_temperature_kelvin, Some(3000));
    let again = serde_json::to_string(&v).expect("serde serialize");
    let v2: JsonFlatLightSetpoint = serde_json::from_str(&again).expect("round");
    assert_eq!(v, v2);
}
