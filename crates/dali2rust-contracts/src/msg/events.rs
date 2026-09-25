use serde::{Deserialize, Serialize};

use super::bounded::{FixedBytes24, FixedItems, FixedText32, FixedText64};
use super::commands::{
    DaliProgramTarget, DaliSceneTargetState, GroupMembershipAction, SceneProgramAction,
};
use super::errors::CompactErrorPayload;
use super::kinds::{
    AttributeGroupReadOutcome, ColorMode, DaliTargetScope, DecodeStatus, DeviceType, DeviceTypeSet,
    IdentifyMechanism, InputDeviceLifecycleKind, InputEventKind, ObservedFrameWidth, ObservedKind,
    OperationStatus, OperationType, OperationWorkerSignal, RuntimeSource,
};
use super::payload_macros::declare_bus_payloads;
use super::state::{Dt6ReadSnapshot, LightSetpoint, RuntimeObservation, SetpointDimensions};
use super::wire::DaliEventPayload;

pub const MAX_PERSISTENCE_SLICES: usize = 151;
pub const MAX_PERSISTENCE_ERRORS: usize = 149;

pub type PersistenceSliceList = FixedItems<PersistenceSliceKind, MAX_PERSISTENCE_SLICES>;
pub type PersistenceSliceErrorList = FixedItems<PersistenceSliceError, MAX_PERSISTENCE_ERRORS>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistenceSliceKind {
    Adapters,
    Groups { adapter_id: u8 },
    VirtualLamps { adapter_id: u8 },
    PhysicalDevices { adapter_id: u8 },
    Scenes { adapter_id: u8, scene_id: u8 },
    HclSchedules,
    PollerSettings,
    HomeAssistantSettings,
    DaliSettings,
    InputDevices { bank: u8 },
    RedundancySettings,
    Policies,
    PhysicalDeviceBank { adapter_id: u8, bank: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistenceSliceError {
    pub kind: PersistenceSliceKind,
    pub error: FixedText64,
}

declare_bus_payloads! {
    union BusEventPayload;
    names EVENT_VARIANT_NAMES;
    tests event_payload_tests;
    probe crate::msg::payload_test_samples::event_probe;
    extern {
        DaliEventPayload {
            budget = DaliEventPayload {
                wire_address: u8::MAX,
                command: u8::MAX,
                repeat_count: u8::MAX,
            };
        }
    }

    pub struct IpAddressAssignedEvent {
        pub ip_v4: [u8; 4],
    }
    budget = IpAddressAssignedEvent {
        ip_v4: [u8::MAX; 4],
    };

    pub struct RuntimeStateChangedEvent {
        pub adapter_id: u8,
        pub virtual_lamp_id: Option<u8>,
        pub short_address: Option<u8>,
        pub state_setpoint: LightSetpoint,
        pub state_observation: RuntimeObservation,
        pub commit_source: RuntimeSource,
        pub commit_dimensions: SetpointDimensions,
    }
    budget = RuntimeStateChangedEvent {
        adapter_id: u8::MAX,
        virtual_lamp_id: Some(63),
        short_address: Some(63),
        state_setpoint: crate::msg::payload_test_samples::worst_setpoint(),
        state_observation: crate::msg::payload_test_samples::worst_observation(),
        commit_source: RuntimeSource::AdapterProxy,
        commit_dimensions: crate::msg::SetpointDimensions { level: true, color: true },
    };

    pub struct AdapterSettingsChangedEvent {
        pub adapter_id: u8,
        pub name: FixedText64,
        pub enabled: bool,
    }
    budget = AdapterSettingsChangedEvent {
        adapter_id: u8::MAX,
        name: crate::msg::payload_test_samples::worst_text64(),
        enabled: true,
    };

    pub struct StatsReportedEvent {
        pub persistence_unavailable_total_delta: u64,
        pub persistence_no_space_total_delta: u64,
    }
    budget = StatsReportedEvent {
        persistence_unavailable_total_delta: u64::MAX,
        persistence_no_space_total_delta: u64::MAX,
    };

    pub struct OperationStatusChangedEvent {
        pub operation_type: OperationType,
        pub status: OperationStatus,
        pub error: Option<CompactErrorPayload>,
        pub started_at_ms: u64,
        pub finished_at_ms: Option<u64>,
        pub ttl_remaining_ms: u32,
        pub operation_key: FixedText32,
    }
    budget = OperationStatusChangedEvent {
        operation_type: OperationType::CommissioningReplaceDevice,
        status: OperationStatus::Cancelled,
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
        started_at_ms: u64::MAX,
        finished_at_ms: Some(u64::MAX),
        ttl_remaining_ms: u32::MAX,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct OperationWorkerSignalEvent {
        pub workflow_correlation_id: u64,
        pub signal: OperationWorkerSignal,
        pub error: Option<CompactErrorPayload>,
    }
    budget = OperationWorkerSignalEvent {
        workflow_correlation_id: u64::MAX,
        signal: OperationWorkerSignal::WorkerFailed,
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
    };

    pub struct PhysicalDeviceChangedEvent {
        pub adapter_id: u8,
        pub short_address: u8,
    }
    budget = PhysicalDeviceChangedEvent {
        adapter_id: u8::MAX,
        short_address: 63,
    };

    pub struct VirtualLampChangedEvent {
        pub adapter_id: u8,
        pub virtual_lamp_id: u8,
    }
    budget = VirtualLampChangedEvent {
        adapter_id: u8::MAX,
        virtual_lamp_id: 63,
    };

    pub struct GroupChangedEvent {
        pub adapter_id: u8,
        pub group_id: u8,
    }
    budget = GroupChangedEvent {
        adapter_id: u8::MAX,
        group_id: 15,
    };

    pub struct GroupMatrixChangedEvent {
        pub adapter_id: u8,
    }
    budget = GroupMatrixChangedEvent {
        adapter_id: u8::MAX,
    };

    pub struct DaliAttributesWrittenEvent {
        pub short_address: u8,
        pub fade_time_ms: Option<u32>,
        pub fade_rate: Option<u8>,
        pub power_on_level: Option<u8>,
        pub system_failure_level: Option<u8>,
        pub extended_fade_time_ms: Option<u16>,
        pub registry_adapter_id: u8,
        pub tc_coolest_mirek: Option<u16>,
        pub tc_warmest_mirek: Option<u16>,
        pub min_level: Option<u8>,
        pub max_level: Option<u8>,
        pub dimming_curve: Option<u8>,
    }
    budget = DaliAttributesWrittenEvent {
        short_address: 63,
        fade_time_ms: Some(u32::MAX),
        fade_rate: Some(u8::MAX),
        power_on_level: Some(u8::MAX),
        system_failure_level: Some(u8::MAX),
        extended_fade_time_ms: Some(u16::MAX),
        registry_adapter_id: u8::MAX,
        tc_coolest_mirek: Some(u16::MAX),
        tc_warmest_mirek: Some(u16::MAX),
        min_level: Some(u8::MAX),
        max_level: Some(u8::MAX),
        dimming_curve: Some(u8::MAX),
    };

    pub struct DaliTargetStateAppliedEvent {
        pub registry_adapter_id: u8,
        pub scope: DaliTargetScope,
        pub virtual_lamp_id: Option<u8>,
        pub short_address: Option<u8>,
        pub group_id: Option<u8>,
        pub setpoint: LightSetpoint,
        pub dapc_applied: bool,
        pub source: RuntimeSource,
        pub applied_at_mono_ms: u32,
    }
    budget = DaliTargetStateAppliedEvent {
        registry_adapter_id: u8::MAX,
        scope: DaliTargetScope::AddressRange,
        virtual_lamp_id: Some(63),
        short_address: Some(63),
        group_id: Some(15),
        setpoint: crate::msg::payload_test_samples::worst_setpoint(),
        dapc_applied: true,
        source: RuntimeSource::Sniffer,
        applied_at_mono_ms: u32::MAX,
    };

    pub struct DaliTargetStateFailedEvent {
        pub adapter_id: u8,
        pub short_address: u8,
        pub error: CompactErrorPayload,
        pub scope: DaliTargetScope,
        pub group_id: Option<u8>,
        pub virtual_lamp_id: Option<u8>,
    }
    budget = DaliTargetStateFailedEvent {
        adapter_id: u8::MAX,
        short_address: 63,
        error: crate::msg::payload_test_samples::worst_compact_error_payload(),
        scope: DaliTargetScope::AddressRange,
        group_id: Some(15),
        virtual_lamp_id: Some(63),
    };

    pub struct DaliGroupMembershipProgrammedEvent {
        pub registry_adapter_id: u8,
        pub target: DaliProgramTarget,
        pub group_id: u8,
        pub action: GroupMembershipAction,
        pub physical_short_address: Option<u8>,
        pub membership: Option<u16>,
        pub error: Option<CompactErrorPayload>,
    }
    budget = DaliGroupMembershipProgrammedEvent {
        registry_adapter_id: u8::MAX,
        target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 63 },
        group_id: 15,
        action: GroupMembershipAction::Add,
        physical_short_address: Some(63),
        membership: Some(u16::MAX),
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
    };

    pub struct DaliAttributesReadEvent {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub last_chunk: bool,
        pub chunk: DaliAttributeReadChunk,
    }
    budget = crate::msg::payload_test_samples::worst_attributes_read_event();

    pub struct DaliMemoryBankReadEvent {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub bank: u8,
        pub start_offset: u16,
        pub chunk_index: u16,
        pub last_chunk: bool,
        pub data: FixedBytes24,
    }
    budget = DaliMemoryBankReadEvent {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        bank: u8::MAX,
        start_offset: u16::MAX,
        chunk_index: u16::MAX,
        last_chunk: true,
        data: FixedBytes24 {
            len: 24,
            data: [u8::MAX; 24],
        },
    };

    #[derive(Default)]
    pub struct DaliMemoryBankReadAbortedEvent {}
    budget = DaliMemoryBankReadAbortedEvent {};

    pub struct DaliDiscoveryProgressEvent {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub random_address: Option<u32>,
        pub device_type: DeviceType,
        pub color_mode: ColorMode,
        pub dt8_xy_capable: bool,
        pub dt8_tc_capable: bool,
        pub dt8_rgb_capable: bool,
        pub dt8_rgbwaf_capable: bool,
        pub supported_device_types: Option<DeviceTypeSet>,
    }
    budget = DaliDiscoveryProgressEvent {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        random_address: Some(u32::MAX),
        device_type: DeviceType::Other,
        color_mode: ColorMode::Unknown,
        dt8_xy_capable: true,
        dt8_tc_capable: true,
        dt8_rgb_capable: true,
        dt8_rgbwaf_capable: true,
        supported_device_types: Some(DeviceTypeSet::from_bits(u64::MAX)),
    };

    pub struct DaliDiscoveryCompletedEvent {
        pub registry_adapter_id: u8,
    }
    budget = DaliDiscoveryCompletedEvent {
        registry_adapter_id: u8::MAX,
    };

    #[derive(Default)]
    pub struct DaliDiscoveryFailedEvent {}
    budget = DaliDiscoveryFailedEvent {};

    pub struct PersistenceLoadResultEvent {
        pub slice: PersistenceSliceKind,
        pub outcome: PersistenceSliceOutcome,
        pub error: Option<CompactErrorPayload>,
    }
    budget = PersistenceLoadResultEvent {
        slice: PersistenceSliceKind::Scenes {
            adapter_id: u8::MAX,
            scene_id: 15,
        },
        outcome: PersistenceSliceOutcome::Failed,
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
    };

    pub struct DaliDiscoveryScanReconciledEvent {
        pub registry_adapter_id: u8,
        pub confirmed_mask: u64,
    }
    budget = DaliDiscoveryScanReconciledEvent {
        registry_adapter_id: u8::MAX,
        confirmed_mask: u64::MAX,
    };

    pub struct DaliAttributeReadOutcomesEvent {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub identity: AttributeGroupReadOutcome,
        pub runtime_status: AttributeGroupReadOutcome,
        pub common_102: AttributeGroupReadOutcome,
        pub dt8_color: AttributeGroupReadOutcome,
        pub dt6_led: AttributeGroupReadOutcome,
        pub groups: AttributeGroupReadOutcome,
        pub scenes: AttributeGroupReadOutcome,
        pub extended: AttributeGroupReadOutcome,
        pub memory_banks: AttributeGroupReadOutcome,
        pub scene_colours: AttributeGroupReadOutcome,
    }
    budget = DaliAttributeReadOutcomesEvent {
        registry_adapter_id: u8::MAX,
        short_address: u8::MAX,
        identity: AttributeGroupReadOutcome::ContendedAbort,
        runtime_status: AttributeGroupReadOutcome::ContendedAbort,
        common_102: AttributeGroupReadOutcome::ContendedAbort,
        dt8_color: AttributeGroupReadOutcome::ContendedAbort,
        dt6_led: AttributeGroupReadOutcome::ContendedAbort,
        groups: AttributeGroupReadOutcome::ContendedAbort,
        scenes: AttributeGroupReadOutcome::ContendedAbort,
        extended: AttributeGroupReadOutcome::ContendedAbort,
        memory_banks: AttributeGroupReadOutcome::ContendedAbort,
        scene_colours: AttributeGroupReadOutcome::ContendedAbort,
    };

    pub struct SceneChangedEvent {
        pub adapter_id: u8,
        pub scene_id: u8,
    }
    budget = SceneChangedEvent {
        adapter_id: u8::MAX,
        scene_id: 15,
    };

    pub struct SceneMatrixChangedEvent {
        pub adapter_id: u8,
        pub scene_id: u8,
    }
    budget = SceneMatrixChangedEvent {
        adapter_id: u8::MAX,
        scene_id: 15,
    };

    pub struct DaliSceneProgrammedEvent {
        pub registry_adapter_id: u8,
        pub target: DaliProgramTarget,
        pub scene_id: u8,
        pub action: SceneProgramAction,
        pub physical_short_address: Option<u8>,
        pub target_state: Option<DaliSceneTargetState>,
        pub scene_level: Option<u8>,
        pub error: Option<CompactErrorPayload>,
    }
    budget = DaliSceneProgrammedEvent {
        registry_adapter_id: u8::MAX,
        target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 63 },
        scene_id: 15,
        action: SceneProgramAction::Update,
        physical_short_address: Some(63),
        target_state: Some(crate::msg::payload_test_samples::worst_scene_target_state()),
        scene_level: Some(u8::MAX),
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
    };

    pub struct DaliSceneRecalledEvent {
        pub registry_adapter_id: u8,
        pub scope: DaliTargetScope,
        pub short_address: u8,
        pub group_id: u8,
        pub scene_id: u8,
        pub error: Option<CompactErrorPayload>,
        pub recalled_at_mono_ms: u32,
    }
    budget = DaliSceneRecalledEvent {
        registry_adapter_id: u8::MAX,
        scope: DaliTargetScope::AddressRange,
        short_address: 63,
        group_id: 15,
        scene_id: 15,
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
        recalled_at_mono_ms: u32::MAX,
    };

    pub struct DaliObservedFrameEvent {
        pub registry_adapter_id: u8,
        pub observed_kind: ObservedKind,
        pub scope: DaliTargetScope,
        pub short_address: Option<u8>,
        pub group_id: Option<u8>,
        pub scene_id: Option<u8>,
        pub setpoint: Option<LightSetpoint>,
        pub dapc_observed: bool,
        pub level_transition: Option<super::kinds::LevelTransition>,
        pub raw_frame: [u8; 3],
        pub raw_width: ObservedFrameWidth,
        pub decode_status: DecodeStatus,
        pub observed_at_ms: u64,
        pub observed_at_mono_ms: u32,
    }
    budget = DaliObservedFrameEvent {
        registry_adapter_id: u8::MAX,
        observed_kind: ObservedKind::UnknownObserved,
        scope: DaliTargetScope::AddressRange,
        short_address: Some(63),
        group_id: Some(15),
        scene_id: Some(15),
        setpoint: Some(crate::msg::payload_test_samples::worst_setpoint()),
        dapc_observed: true,
        level_transition: Some(crate::msg::kinds::LevelTransition::GoToLastActiveLevel),
        raw_frame: [u8::MAX; 3],
        raw_width: ObservedFrameWidth::Forward24,
        decode_status: DecodeStatus::Ambiguous,
        observed_at_ms: u64::MAX,
        observed_at_mono_ms: u32::MAX,
    };

    pub struct DaliDeviceIdentifiedEvent {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub mechanism: IdentifyMechanism,
        pub operation_key: FixedText32,
        pub error: Option<CompactErrorPayload>,
    }
    budget = DaliDeviceIdentifiedEvent {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        mechanism: IdentifyMechanism::BlinkRecallMaxMin,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
    };

    pub struct DaliAddressingCompletedEvent {
        pub registry_adapter_id: u8,
        pub old_short_address: u8,
        pub new_short_address: u8,
        pub operation_key: FixedText32,
        pub error: Option<CompactErrorPayload>,
    }
    budget = DaliAddressingCompletedEvent {
        registry_adapter_id: u8::MAX,
        old_short_address: 63,
        new_short_address: 63,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
    };

    pub struct DaliDeviceReplacedEvent {
        pub registry_adapter_id: u8,
        pub failed_short_address: u8,
        pub replacement_short_address: u8,
        pub restored_metadata_and_overrides: bool,
        pub restored_attributes: bool,
        pub restored_groups: bool,
        pub restored_scenes: bool,
        pub operation_key: FixedText32,
        pub error: Option<CompactErrorPayload>,
    }
    budget = DaliDeviceReplacedEvent {
        registry_adapter_id: u8::MAX,
        failed_short_address: 63,
        replacement_short_address: 63,
        restored_metadata_and_overrides: true,
        restored_attributes: true,
        restored_groups: true,
        restored_scenes: true,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
        error: Some(crate::msg::payload_test_samples::worst_compact_error_payload()),
    };

    pub struct HclScheduleChangedEvent {
        pub schedule_id: FixedText32,
        pub removed: bool,
        pub enabled: bool,
    }
    budget = HclScheduleChangedEvent {
        schedule_id: crate::msg::payload_test_samples::worst_text32(),
        removed: true,
        enabled: true,
    };

    pub struct PollerSettingsChangedEvent {
        pub enabled: bool,
        pub interval_ms: u32,
        pub attribute_groups_mask: u8,
        pub include_dt8_color: bool,
        pub include_energy: bool,
        pub include_diagnostics: bool,
        pub skip_unbound_virtual_lamps: bool,
    }
    budget = PollerSettingsChangedEvent {
        enabled: true,
        interval_ms: u32::MAX,
        attribute_groups_mask: u8::MAX,
        include_dt8_color: true,
        include_energy: true,
        include_diagnostics: true,
        skip_unbound_virtual_lamps: true,
    };

    pub struct HomeAssistantSettingsChangedEvent {
        pub enabled: bool,
    }
    budget = HomeAssistantSettingsChangedEvent { enabled: true };

    pub struct HomeAssistantDiscoveryPublishedEvent {
        pub entities_published: u16,
        pub entities_failed: u16,
    }
    budget = HomeAssistantDiscoveryPublishedEvent {
        entities_published: u16::MAX,
        entities_failed: u16::MAX,
    };

    pub struct DaliSettingsChangedEvent {
        pub dt8_auto_activation_repair: bool,
        pub dt8_rgbwaf_control_assert: bool,
        pub application_active: bool,
        pub application_active_moved_by: u8,
    }
    budget = DaliSettingsChangedEvent {
        dt8_auto_activation_repair: true,
        dt8_rgbwaf_control_assert: true,
        application_active: true,
        application_active_moved_by: u8::MAX,
    };

    pub struct DaliBusHealthProbedEvent {
        pub registry_adapter_id: u8,
        pub control_answered: bool,
        pub lamp_failure: BusHealthVerdict,
    }
    budget = DaliBusHealthProbedEvent {
        registry_adapter_id: u8::MAX,
        control_answered: true,
        lamp_failure: BusHealthVerdict::Several,
    };

    pub struct DaliInputEventObservedEvent {
        pub registry_adapter_id: u8,
        pub scheme: u8,
        pub short_address: Option<u8>,
        pub device_group: Option<u8>,
        pub instance_group: Option<u8>,
        pub instance_number: Option<u8>,
        pub instance_type: Option<u8>,
        pub event_info: u16,
        pub typed: InputEventKind,
        pub typed_value: u16,
        pub observed_at_ms: u64,
        pub observed_at_mono_ms: u32,
    }
    budget = DaliInputEventObservedEvent {
        registry_adapter_id: u8::MAX,
        scheme: 4,
        short_address: Some(63),
        device_group: Some(31),
        instance_group: Some(31),
        instance_number: Some(31),
        instance_type: Some(31),
        event_info: u16::MAX,
        typed: InputEventKind::Occupancy,
        typed_value: u16::MAX,
        observed_at_ms: u64::MAX,
        observed_at_mono_ms: u32::MAX,
    };

    pub struct DaliInputDeviceLifecycleEvent {
        pub registry_adapter_id: u8,
        pub kind: InputDeviceLifecycleKind,
        pub short_address: Option<u8>,
        pub device_group: Option<u8>,
        pub observed_at_ms: u64,
        pub observed_at_mono_ms: u32,
    }
    budget = DaliInputDeviceLifecycleEvent {
        registry_adapter_id: u8::MAX,
        kind: InputDeviceLifecycleKind::PowerCycle,
        short_address: Some(63),
        device_group: Some(31),
        observed_at_ms: u64::MAX,
        observed_at_mono_ms: u32::MAX,
    };

    pub struct Dali103ScanProgressEvent {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub presence_unproven: bool,
        pub instance_count: u8,
        pub device_capabilities: Option<u8>,
        pub device_status: Option<u8>,
        pub version_number: Option<u8>,
    }
    budget = Dali103ScanProgressEvent {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        presence_unproven: true,
        instance_count: 32,
        device_capabilities: Some(u8::MAX),
        device_status: Some(u8::MAX),
        version_number: Some(u8::MAX),
    };

    pub struct Dali103InstanceConfiguredEvent {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub instance_number: u8,
        pub event_scheme: Option<u8>,
        pub event_filter: Option<[u8; 3]>,
        pub event_priority: Option<u8>,
        pub instance_groups: [Option<Option<u8>>; 3],
        pub timers: [Option<u8>; 4],
        pub manual_config_active: Option<bool>,
        pub feedback_opcode_map: Option<u8>,
        pub feedback_capability: Option<u8>,
        pub feedback_colour_capability: Option<u8>,
        pub feedback_timing: Option<u8>,
        pub feedback_active_brightness: Option<u8>,
        pub feedback_active_colour: Option<u8>,
        pub feedback_inactive_brightness: Option<u8>,
        pub feedback_inactive_colour: Option<u8>,
        pub instance_status: Option<u8>,
        pub resolution: Option<u8>,
        pub instance_status_written: bool,
        pub instance_type: Option<u8>,
    }
    budget = Dali103InstanceConfiguredEvent {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        instance_number: 31,
        event_scheme: Some(4),
        event_filter: Some([u8::MAX; 3]),
        event_priority: Some(5),
        instance_groups: [Some(Some(31)); 3],
        timers: [Some(u8::MAX); 4],
        manual_config_active: Some(true),
        feedback_opcode_map: Some(2),
        feedback_capability: Some(u8::MAX),
        feedback_colour_capability: Some(63),
        feedback_timing: Some(u8::MAX),
        feedback_active_brightness: Some(u8::MAX),
        feedback_active_colour: Some(63),
        feedback_inactive_brightness: Some(u8::MAX),
        feedback_inactive_colour: Some(63),
        instance_status: Some(u8::MAX),
        resolution: Some(u8::MAX),
        instance_status_written: true,
        instance_type: Some(u8::MAX),
    };

    pub struct InputDeviceChangedEvent {
        pub adapter_id: u8,
        pub short_address: u8,
    }
    budget = InputDeviceChangedEvent {
        adapter_id: u8::MAX,
        short_address: 63,
    };

    pub struct Dali103ApplicationControlObservedEvent {
        pub registry_adapter_id: u8,
        pub scope_broadcast: bool,
        pub short_address: u8,
        pub enable: bool,
        pub observed_at_ms: u64,
    }
    budget = Dali103ApplicationControlObservedEvent {
        registry_adapter_id: u8::MAX,
        scope_broadcast: true,
        short_address: 63,
        enable: true,
        observed_at_ms: u64::MAX,
    };

    pub struct RulesChangedEvent {
        pub revision: u32,
        pub rule_count: u8,
        pub lang_id: u8,
    }
    budget = RulesChangedEvent {
        revision: u32::MAX,
        rule_count: u8::MAX,
        lang_id: u8::MAX,
    };

    pub struct RulesActivationEvent {
        pub rule_name: FixedText64,
        pub dry: bool,
        pub effects: u8,
        pub partial: u8,
        pub trigger_to_publish_ms: u16,
    }
    budget = RulesActivationEvent {
        rule_name: crate::msg::payload_test_samples::worst_text64(),
        dry: true,
        effects: u8::MAX,
        partial: 3,
        trigger_to_publish_ms: u16::MAX,
    };

    pub struct Dali103ScanStartedEvent {
        pub registry_adapter_id: u8,
    }
    budget = Dali103ScanStartedEvent {
        registry_adapter_id: u8::MAX,
    };

    pub struct RedundancySettingsChangedEvent {
        pub enabled: bool,
        pub standby_role: bool,
        pub probe_interval_ms: u32,
        pub takeover_after_missed: u8,
        pub boot_listen_ms: u32,
        pub peer_device_short_address: u8,
        pub peer_url: FixedText64,
    }
    budget = RedundancySettingsChangedEvent {
        enabled: true,
        standby_role: true,
        probe_interval_ms: u32::MAX,
        takeover_after_missed: u8::MAX,
        boot_listen_ms: u32::MAX,
        peer_device_short_address: u8::MAX,
        peer_url: crate::msg::payload_test_samples::worst_text64(),
    };

    pub struct RedundancyTransitionEvent {
        pub now_active: bool,
        pub reason: u8,
        pub detected_at_ms: u32,
        pub completed_at_ms: u32,
        pub last_peer_answer_ms: u32,
        pub missed_probes: u8,
    }
    budget = RedundancyTransitionEvent {
        now_active: true,
        reason: u8::MAX,
        detected_at_ms: u32::MAX,
        completed_at_ms: u32::MAX,
        last_peer_answer_ms: u32::MAX,
        missed_probes: u8::MAX,
    };


    pub struct Dali103ArbitrationProbedEvent {
        pub registry_adapter_id: u8,
        pub owned: bool,
    }
    budget = Dali103ArbitrationProbedEvent {
        registry_adapter_id: u8::MAX,
        owned: true,
    };


    pub struct PoliciesChangedEvent {
        pub system_failure_level: u8,
        pub power_on_level: u8,
        pub apply_on_discovery: bool,
    }
    budget = PoliciesChangedEvent {
        system_failure_level: u8::MAX,
        power_on_level: u8::MAX,
        apply_on_discovery: true,
    };

    pub struct Dali103HandoverSentEvent {
        pub registry_adapter_id: u8,
        pub peer_short_address: u8,
    }
    budget = Dali103HandoverSentEvent {
        registry_adapter_id: u8::MAX,
        peer_short_address: u8::MAX,
    };

    pub struct RegistrySliceReloadedEvent {
        pub slice_name: FixedText32,
    }
    budget = RegistrySliceReloadedEvent {
        slice_name: crate::msg::payload_test_samples::worst_text32(),
    };

}

pub const APPLICATION_ACTIVE_UNMOVED: u8 = 0;
pub const APPLICATION_ACTIVE_MOVED_BY_COMMAND: u8 = 1;
pub const APPLICATION_ACTIVE_MOVED_BY_WIRE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusHealthVerdict {
    Clear,
    One,
    Several,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistenceSliceOutcome {
    Loaded,
    Defaults,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaliAttributeReadChunk {
    Identity {
        random_address: u32,
    },
    Common102 {
        version: Option<u8>,
        device_type: Option<u8>,
        physical_minimum: Option<u8>,
        min_level: Option<u8>,
        max_level: Option<u8>,
        power_on_level: Option<u8>,
        system_failure_level: Option<u8>,
        fade_time_ms: Option<u32>,
        fade_rate: Option<u8>,
        supported_device_types: Option<DeviceTypeSet>,
        light_source_type: Option<u8>,
        light_source_types: Option<u32>,
    },
    Dt8Color {
        color_mode: ColorMode,
        xy_capable: bool,
        tc_capable: bool,
        rgb_capable: bool,
        color_type: Option<u8>,
        color_value_0: Option<u16>,
        color_value_1: Option<u16>,
        color_value_2: Option<u16>,
        tc_coolest_mirek: Option<u16>,
        tc_warmest_mirek: Option<u16>,
        gear_features: Option<u8>,
        rgbwaf_capable: bool,
        rgbwaf_control: Option<u8>,
    },
    Groups {
        membership: Option<u16>,
    },
    Scenes {
        levels: Option<[u8; 16]>,
    },
    Dt6Led {
        snapshot: Dt6ReadSnapshot,
    },
    Extended {
        fade_time_ms: Option<u16>,
        version_number: Option<u8>,
    },
    RuntimeStatus {
        setpoint: Option<LightSetpoint>,
        observation: RuntimeObservation,
        read_started_mono_ms: u32,
    },
    SceneColour {
        scene: u8,
        level: Option<u8>,
        colour_type: Option<u8>,
        values: [Option<u16>; 6],
    },
}

impl DaliAttributeReadOutcomesEvent {
    #[must_use]
    pub fn section_outcomes(&self) -> [AttributeGroupReadOutcome; 10] {
        let Self {
            registry_adapter_id: _,
            short_address: _,
            identity,
            runtime_status,
            common_102,
            dt8_color,
            dt6_led,
            groups,
            scenes,
            extended,
            memory_banks,
            scene_colours,
        } = *self;
        [
            identity,
            runtime_status,
            common_102,
            dt8_color,
            dt6_led,
            groups,
            scenes,
            extended,
            memory_banks,
            scene_colours,
        ]
    }

    #[must_use]
    pub fn any_section_absent(&self) -> bool {
        self.section_outcomes()
            .contains(&AttributeGroupReadOutcome::DeviceAbsent)
    }
}

impl OperationWorkerSignalEvent {
    pub fn started(workflow_correlation_id: u64) -> Self {
        Self {
            workflow_correlation_id,
            signal: OperationWorkerSignal::WorkerStarted,
            error: None,
        }
    }

    pub fn succeeded(workflow_correlation_id: u64) -> Self {
        Self {
            workflow_correlation_id,
            signal: OperationWorkerSignal::WorkerSucceeded,
            error: None,
        }
    }

    pub fn failed(
        workflow_correlation_id: u64,
        code: super::errors::ErrorCode,
        message: &str,
    ) -> Self {
        Self {
            workflow_correlation_id,
            signal: OperationWorkerSignal::WorkerFailed,
            error: Some(CompactErrorPayload::new(code, message)),
        }
    }
}

impl IpAddressAssignedEvent {
    pub fn from_ip_text(ip: &str) -> Self {
        const OCTETS: usize = 4;
        let mut out = [0u8; OCTETS];
        let mut idx = 0usize;
        let mut ok = true;
        for part in ip.split('.') {
            if idx >= OCTETS {
                ok = false;
                break;
            }
            match part.parse::<u8>() {
                Ok(v) => out[idx] = v,
                Err(_) => {
                    ok = false;
                    break;
                }
            }
            idx += 1;
        }
        Self {
            ip_v4: if ok && idx == OCTETS { out } else { [0u8; OCTETS] },
        }
    }
}
