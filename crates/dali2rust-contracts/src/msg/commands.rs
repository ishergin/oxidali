use serde::{Deserialize, Serialize};

use super::bounded::{FixedItems, FixedText32, FixedText48, FixedText64, FixedText96};
use super::kinds::{
    CommissioningStep, ConfigWriteResource, DaliTargetScope, DiscoveryMode, HclAlgorithm,
    HclLevelMode,
    HclTargetScope, HclTimeRef, InitialiseScope, MemoryBankReadPreset, OperationType, PowerState,
    RuntimeSource,
};
use super::payload_macros::declare_bus_payloads;
use super::state::{ColorValue, LightSetpoint, RuntimeObservation};
use super::wire::DaliCommandPayload;

pub const MAX_GROUP_MATRIX_ROWS_PER_COMMAND: usize = 16;
pub type GroupMatrixDesiredRowList =
    FixedItems<GroupMatrixDesiredRow, MAX_GROUP_MATRIX_ROWS_PER_COMMAND>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMatrixDesiredRow {
    pub virtual_lamp_id: u8,
    pub desired_groups_mask: u16,
}

pub const MAX_SCENE_MATRIX_ROWS_PER_COMMAND: usize = 4;
pub type SceneMatrixDesiredRowList =
    FixedItems<SceneMatrixDesiredRow, MAX_SCENE_MATRIX_ROWS_PER_COMMAND>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaliSceneTargetState {
    pub power: Option<PowerState>,
    pub level: Option<u8>,
    pub color: Option<ColorValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneMatrixDesiredRow {
    pub virtual_lamp_id: u8,
    pub included: bool,
    pub target: Option<DaliSceneTargetState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneProgramAction {
    Write,
    Update,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaliProgramTarget {
    VirtualLamp { virtual_lamp_id: u8 },
    Short { short_address: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupMembershipAction {
    Add,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRegistryUpdateEntry {
    pub virtual_lamp_id: Option<u8>,
    pub short_address: Option<u8>,
    pub setpoint: Option<LightSetpoint>,
    pub observation: Option<RuntimeObservation>,
    pub last_dapc_source: Option<super::kinds::LastDapcSource>,
    pub source: RuntimeSource,
    pub observed_at_mono_ms: Option<u32>,
}

impl Default for RuntimeRegistryUpdateEntry {
    fn default() -> Self {
        Self {
            virtual_lamp_id: None,
            short_address: None,
            setpoint: None,
            observation: None,
            last_dapc_source: None,
            source: RuntimeSource::Poller,
            observed_at_mono_ms: None,
        }
    }
}

pub const MAX_HCL_TARGETS_PER_COMMAND: usize = 4;
pub type HclTargetList = FixedItems<HclTargetRow, MAX_HCL_TARGETS_PER_COMMAND>;

pub const MAX_HCL_POINTS_PER_COMMAND: usize = 2;
pub type HclPointList = FixedItems<HclSchedulePointRow, MAX_HCL_POINTS_PER_COMMAND>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HclTargetRow {
    pub adapter_id: u8,
    pub scope: HclTargetScope,
    pub group_mask: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HclSchedulePointRow {
    pub time_ref: HclTimeRef,
    pub offset_minutes: i16,
    pub level_mode: HclLevelMode,
    pub level: Option<u8>,
    pub color_temperature_kelvin: Option<u16>,
}

declare_bus_payloads! {
    union BusCommandPayload;
    names COMMAND_VARIANT_NAMES;
    tests command_payload_tests;
    probe crate::msg::payload_test_samples::command_probe;
    extern {
        DaliCommandPayload {
            budget = DaliCommandPayload {
                wire_address: u8::MAX,
                command: u8::MAX,
                repeat_count: u8::MAX,
                raw_mode: true,
                raw_expects_backward: true,
            };
        }
    }

    pub struct AdapterSettingsUpdateCommand {
        pub patch_mask: u8,
        pub name: FixedText64,
        pub enabled: bool,
    }
    budget = AdapterSettingsUpdateCommand {
        patch_mask: u8::MAX,
        name: crate::msg::payload_test_samples::worst_text64(),
        enabled: true,
    };

    pub struct GroupMetadataUpdateCommand {
        pub adapter_id: u8,
        pub group_id: u8,
        pub patch_mask: u8,
        pub name: FixedText64,
        pub ha_entity_enabled: bool,
    }
    budget = GroupMetadataUpdateCommand {
        adapter_id: u8::MAX,
        group_id: 15,
        patch_mask: u8::MAX,
        name: crate::msg::payload_test_samples::worst_text64(),
        ha_entity_enabled: true,
    };

    pub struct GroupMatrixDesiredPatchCommand {
        pub adapter_id: u8,
        pub rows: GroupMatrixDesiredRowList,
    }
    budget = GroupMatrixDesiredPatchCommand {
        adapter_id: u8::MAX,
        rows: crate::msg::payload_test_samples::worst_group_matrix_rows(),
    };

    pub struct GroupMatrixDesiredReplaceCommand {
        pub adapter_id: u8,
        pub rows: GroupMatrixDesiredRowList,
    }
    budget = GroupMatrixDesiredReplaceCommand {
        adapter_id: u8::MAX,
        rows: crate::msg::payload_test_samples::worst_group_matrix_rows(),
    };

    pub struct RegistryRuntimeUpdateCommand {
        pub adapter_id: u8,
        pub update: RuntimeRegistryUpdateEntry,
    }
    budget = RegistryRuntimeUpdateCommand {
        adapter_id: u8::MAX,
        update: crate::msg::payload_test_samples::worst_runtime_update_entry(),
    };

    pub struct VirtualLampConfigUpdateCommand {
        pub adapter_id: u8,
        pub virtual_lamp_id: u8,
        pub patch_mask: u8,
        pub name: FixedText64,
        pub ha_entity_enabled: bool,
    }
    budget = VirtualLampConfigUpdateCommand {
        adapter_id: u8::MAX,
        virtual_lamp_id: 63,
        patch_mask: u8::MAX,
        name: crate::msg::payload_test_samples::worst_text64(),
        ha_entity_enabled: true,
    };

    pub struct OperationBeginCommand {
        pub operation_key: FixedText32,
        pub operation_type: OperationType,
        pub ttl_ms: u32,
        pub finished_retention_ms: u32,
        pub expected_outcomes: u16,
    }
    budget = OperationBeginCommand {
        operation_key: crate::msg::payload_test_samples::worst_text32(),
        operation_type: OperationType::CommissioningReplaceDevice,
        ttl_ms: u32::MAX,
        finished_retention_ms: u32::MAX,
        expected_outcomes: u16::MAX,
    };

    #[derive(Default)]
    pub struct OperationRegistryResetCommand {}
    budget = OperationRegistryResetCommand {};

    pub struct PhysicalDeviceOverrideCommand {
        pub adapter_id: u8,
        pub short_address: u8,
        pub patch_mask: u8,
        pub name: FixedText64,
        pub clear_device_type_override: bool,
        pub device_type_override: super::kinds::DeviceType,
        pub clear_color_mode_override: bool,
        pub color_mode_override: super::kinds::ColorMode,
        pub dt8_auto_activation_repair: bool,
        pub dt8_rgbwaf_control_assert: bool,
    }
    budget = PhysicalDeviceOverrideCommand {
        adapter_id: u8::MAX,
        short_address: 63,
        patch_mask: u8::MAX,
        name: crate::msg::payload_test_samples::worst_text64(),
        clear_device_type_override: true,
        device_type_override: crate::msg::kinds::DeviceType::Other,
        clear_color_mode_override: true,
        color_mode_override: crate::msg::kinds::ColorMode::Unknown,
        dt8_auto_activation_repair: true,
        dt8_rgbwaf_control_assert: true,
    };

    pub struct DaliSetTargetStateCommand {
        pub scope: DaliTargetScope,
        pub virtual_lamp_id: u8,
        pub short_address: u8,
        pub group_id: u8,
        pub setpoint: LightSetpoint,
        pub registry_adapter_id: u8,
    }
    budget = DaliSetTargetStateCommand {
        scope: DaliTargetScope::AddressRange,
        virtual_lamp_id: 63,
        short_address: 63,
        group_id: 15,
        setpoint: crate::msg::payload_test_samples::worst_setpoint(),
        registry_adapter_id: u8::MAX,
    };

    pub struct DaliWriteAttributesCommand {
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
        pub signals_operation: bool,
    }
    budget = DaliWriteAttributesCommand {
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
        signals_operation: true,
    };

    pub struct DaliDiscoverDevicesCommand {
        pub mode: DiscoveryMode,
        pub registry_adapter_id: u8,
    }
    budget = DaliDiscoverDevicesCommand {
        mode: DiscoveryMode::RefreshKnown,
        registry_adapter_id: u8::MAX,
    };

    pub struct DaliReadAttributesCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub attribute_groups_mask: u8,
        pub memory_banks: MemoryBankReadPreset,
    }
    budget = DaliReadAttributesCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        attribute_groups_mask: u8::MAX,
        memory_banks: MemoryBankReadPreset::All,
    };

    pub struct DaliReadMemoryBankCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub bank: u8,
        pub start: u16,
        pub length: u16,
    }
    budget = DaliReadMemoryBankCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        bank: u8::MAX,
        start: u16::MAX,
        length: u16::MAX,
    };

    pub struct DaliProgramGroupMembershipCommand {
        pub registry_adapter_id: u8,
        pub target: DaliProgramTarget,
        pub group_id: u8,
        pub action: GroupMembershipAction,
    }
    budget = DaliProgramGroupMembershipCommand {
        registry_adapter_id: u8::MAX,
        target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 63 },
        group_id: 15,
        action: GroupMembershipAction::Add,
    };

    pub struct VirtualLampBindCommand {
        pub adapter_id: u8,
        pub virtual_lamp_id: u8,
        pub physical_short_address: u8,
    }
    budget = VirtualLampBindCommand {
        adapter_id: u8::MAX,
        virtual_lamp_id: 63,
        physical_short_address: 63,
    };

    pub struct VirtualLampRebindCommand {
        pub adapter_id: u8,
        pub virtual_lamp_id: u8,
        pub physical_short_address: u8,
    }
    budget = VirtualLampRebindCommand {
        adapter_id: u8::MAX,
        virtual_lamp_id: 63,
        physical_short_address: 63,
    };

    pub struct VirtualLampUnbindCommand {
        pub adapter_id: u8,
        pub virtual_lamp_id: u8,
    }
    budget = VirtualLampUnbindCommand {
        adapter_id: u8::MAX,
        virtual_lamp_id: 63,
    };

    pub struct GroupApplyExecuteCommand {
        pub registry_adapter_id: u8,
        pub operation_key: FixedText32,
    }
    budget = GroupApplyExecuteCommand {
        registry_adapter_id: u8::MAX,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct SceneMetadataUpdateCommand {
        pub adapter_id: u8,
        pub scene_id: u8,
        pub patch_mask: u8,
        pub name: FixedText64,
        pub ha_select_enabled: bool,
    }
    budget = SceneMetadataUpdateCommand {
        adapter_id: u8::MAX,
        scene_id: 15,
        patch_mask: u8::MAX,
        name: crate::msg::payload_test_samples::worst_text64(),
        ha_select_enabled: true,
    };

    pub struct SceneMatrixDesiredPatchCommand {
        pub adapter_id: u8,
        pub scene_id: u8,
        pub rows: SceneMatrixDesiredRowList,
    }
    budget = SceneMatrixDesiredPatchCommand {
        adapter_id: u8::MAX,
        scene_id: 15,
        rows: crate::msg::payload_test_samples::worst_scene_matrix_rows(),
    };

    pub struct SceneMatrixDesiredReplaceCommand {
        pub adapter_id: u8,
        pub scene_id: u8,
        pub rows: SceneMatrixDesiredRowList,
    }
    budget = SceneMatrixDesiredReplaceCommand {
        adapter_id: u8::MAX,
        scene_id: 15,
        rows: crate::msg::payload_test_samples::worst_scene_matrix_rows(),
    };

    pub struct DaliProgramSceneCommand {
        pub registry_adapter_id: u8,
        pub target: DaliProgramTarget,
        pub scene_id: u8,
        pub action: SceneProgramAction,
        pub target_state: Option<DaliSceneTargetState>,
    }
    budget = DaliProgramSceneCommand {
        registry_adapter_id: u8::MAX,
        target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 63 },
        scene_id: 15,
        action: SceneProgramAction::Update,
        target_state: Some(crate::msg::payload_test_samples::worst_scene_target_state()),
    };

    pub struct DaliRecallSceneCommand {
        pub registry_adapter_id: u8,
        pub scope: DaliTargetScope,
        pub short_address: u8,
        pub group_id: u8,
        pub scene_id: u8,
    }
    budget = DaliRecallSceneCommand {
        registry_adapter_id: u8::MAX,
        scope: DaliTargetScope::AddressRange,
        short_address: 63,
        group_id: 15,
        scene_id: 15,
    };

    pub struct SceneApplyExecuteCommand {
        pub registry_adapter_id: u8,
        pub scene_id: u8,
        pub operation_key: FixedText32,
    }
    budget = SceneApplyExecuteCommand {
        registry_adapter_id: u8::MAX,
        scene_id: 15,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct DaliIdentifyDeviceCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub operation_key: FixedText32,
    }
    budget = DaliIdentifyDeviceCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct DaliAddressingCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub new_short_address: u8,
        pub verify_after_program: bool,
        pub operation_key: FixedText32,
    }
    budget = DaliAddressingCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        new_short_address: 63,
        verify_after_program: true,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct DaliCommissioningStepCommand {
        pub registry_adapter_id: u8,
        pub step: CommissioningStep,
        pub scope: Option<InitialiseScope>,
        pub short_address: Option<u8>,
        pub search_address: Option<u32>,
    }
    budget = DaliCommissioningStepCommand {
        registry_adapter_id: u8::MAX,
        step: CommissioningStep::Terminate,
        scope: Some(InitialiseScope::Short),
        short_address: Some(63),
        search_address: Some(0x00FF_FFFF),
    };

    pub struct DaliReplaceDeviceCommand {
        pub registry_adapter_id: u8,
        pub failed_short_address: u8,
        pub replacement_short_address: u8,
        pub restore_metadata_and_overrides: bool,
        pub restore_attributes: bool,
        pub restore_groups: bool,
        pub restore_scenes: bool,
        pub operation_key: FixedText32,
    }
    budget = DaliReplaceDeviceCommand {
        registry_adapter_id: u8::MAX,
        failed_short_address: 63,
        replacement_short_address: 63,
        restore_metadata_and_overrides: true,
        restore_attributes: true,
        restore_groups: true,
        restore_scenes: true,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
    };
    pub struct HclScheduleUpsertCommand {
        pub schedule_id: FixedText32,
        pub enabled: bool,
        pub algorithm: HclAlgorithm,
        pub active_days_mask: u8,
        pub latitude_microdeg: Option<i32>,
        pub longitude_microdeg: Option<i32>,
        pub first_target_index: u8,
        pub targets: HclTargetList,
        pub first_point_index: u8,
        pub points: HclPointList,
        pub last_chunk: bool,
    }
    budget = HclScheduleUpsertCommand {
        schedule_id: crate::msg::payload_test_samples::worst_text32(),
        enabled: true,
        algorithm: HclAlgorithm::Interpolated,
        active_days_mask: u8::MAX,
        latitude_microdeg: Some(-90_000_000),
        longitude_microdeg: Some(-180_000_000),
        first_target_index: u8::MAX,
        targets: crate::msg::payload_test_samples::worst_hcl_targets(),
        first_point_index: u8::MAX,
        points: crate::msg::payload_test_samples::worst_hcl_points(),
        last_chunk: true,
    };

    pub struct HclScheduleDeleteCommand {
        pub schedule_id: FixedText32,
    }
    budget = HclScheduleDeleteCommand {
        schedule_id: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct DaliRecallLastActiveLevelCommand {
        pub registry_adapter_id: u8,
        pub scope: HclTargetScope,
        pub group_id: Option<u8>,
    }
    budget = DaliRecallLastActiveLevelCommand {
        registry_adapter_id: u8::MAX,
        scope: HclTargetScope::Group,
        group_id: Some(15),
    };

    pub struct PollerSettingsUpdateCommand {
        pub patch_mask: u8,
        pub enabled: bool,
        pub interval_ms: u32,
        pub attribute_groups_mask: u8,
        pub include_dt8_color: bool,
        pub skip_unbound_virtual_lamps: bool,
        pub include_energy: bool,
        pub include_diagnostics: bool,
    }
    budget = PollerSettingsUpdateCommand {
        patch_mask: u8::MAX,
        enabled: true,
        interval_ms: u32::MAX,
        attribute_groups_mask: u8::MAX,
        include_dt8_color: true,
        skip_unbound_virtual_lamps: true,
        include_energy: true,
        include_diagnostics: true,
    };

    pub struct HclOverrideClearCommand {
        pub schedule_id: FixedText32,
    }
    budget = HclOverrideClearCommand {
        schedule_id: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct PhysicalDeviceNotesUpdateCommand {
        pub adapter_id: u8,
        pub short_address: u8,
        pub notes: FixedText48,
    }
    budget = PhysicalDeviceNotesUpdateCommand {
        adapter_id: u8::MAX,
        short_address: 63,
        notes: crate::msg::payload_test_samples::worst_text48(),
    };

    pub struct ConfigWriteCommitCommand {
        pub resource: ConfigWriteResource,
        pub adapter_id: u8,
        pub scene_id: Option<u8>,
        pub chunks: u8,
    }
    budget = ConfigWriteCommitCommand {
        resource: ConfigWriteResource::SceneMatrix,
        adapter_id: u8::MAX,
        scene_id: Some(15),
        chunks: u8::MAX,
    };

    pub struct HomeAssistantSettingsUpdateCommand {
        pub patch_mask: u8,
        pub enabled: bool,
        pub broker_host: FixedText64,
        pub broker_port: u16,
        pub publish_qos: u8,
        pub retain_state: bool,
        pub retain_discovery: bool,
        pub expose_input_devices: bool,
    }
    budget = HomeAssistantSettingsUpdateCommand {
        patch_mask: u8::MAX,
        enabled: true,
        broker_host: crate::msg::payload_test_samples::worst_text64(),
        broker_port: u16::MAX,
        publish_qos: u8::MAX,
        retain_state: true,
        retain_discovery: true,
        expose_input_devices: true,
    };

    pub struct HomeAssistantCredentialsUpdateCommand {
        pub patch_mask: u8,
        pub broker_username: FixedText32,
        pub broker_password: FixedText48,
    }
    budget = HomeAssistantCredentialsUpdateCommand {
        patch_mask: u8::MAX,
        broker_username: crate::msg::payload_test_samples::worst_text32(),
        broker_password: crate::msg::payload_test_samples::worst_text48(),
    };

    pub struct HomeAssistantTopicsUpdateCommand {
        pub patch_mask: u8,
        pub discovery_prefix: FixedText32,
        pub state_topic_prefix: FixedText32,
    }
    budget = HomeAssistantTopicsUpdateCommand {
        patch_mask: u8::MAX,
        discovery_prefix: crate::msg::payload_test_samples::worst_text32(),
        state_topic_prefix: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct HomeAssistantControllerIdUpdateCommand {
        pub controller_id: FixedText32,
    }
    budget = HomeAssistantControllerIdUpdateCommand {
        controller_id: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct HomeAssistantDiscoveryPublishCommand {}
    budget = HomeAssistantDiscoveryPublishCommand {};

    pub struct DaliSettingsUpdateCommand {
        pub patch_mask: u8,
        pub dt8_auto_activation_repair: bool,
        pub dt8_rgbwaf_control_assert: bool,
        pub application_active: bool,
        pub device_short_address: u8,
    }
    budget = DaliSettingsUpdateCommand {
        patch_mask: u8::MAX,
        dt8_auto_activation_repair: true,
        dt8_rgbwaf_control_assert: true,
        application_active: true,
        device_short_address: u8::MAX,
    };

    pub struct DaliBusHealthProbeCommand {
        pub registry_adapter_id: u8,
    }
    budget = DaliBusHealthProbeCommand {
        registry_adapter_id: u8::MAX,
    };

    pub struct Dali103ScanCommand {
        pub registry_adapter_id: u8,
    }
    budget = Dali103ScanCommand {
        registry_adapter_id: u8::MAX,
    };

    pub struct Dali103CommissionCommand {
        pub registry_adapter_id: u8,
        pub include_addressed: bool,
    }
    budget = Dali103CommissionCommand {
        registry_adapter_id: u8::MAX,
        include_addressed: true,
    };

    pub struct Dali103InstanceConfigureCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub instance_number: u8,
        pub patch_mask: u16,
        pub event_scheme: u8,
        pub event_filter: [u8; 3],
        pub event_priority: u8,
        pub instance_groups: [Option<u8>; 3],
        pub timer_multipliers: [Option<u8>; 4],
        pub instance_enabled: bool,
    }
    budget = Dali103InstanceConfigureCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        instance_number: 31,
        patch_mask: u16::MAX,
        event_scheme: 4,
        event_filter: [u8::MAX; 3],
        event_priority: 5,
        instance_groups: [Some(31); 3],
        timer_multipliers: [Some(u8::MAX); 4],
        instance_enabled: true,
    };

    pub struct Dali103IdentifyCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
    }
    budget = Dali103IdentifyCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
    };

    pub struct InputDeviceMetadataUpdateCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub patch_mask: u8,
        pub name: FixedText64,
        pub ha_expose: bool,
    }
    budget = InputDeviceMetadataUpdateCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        patch_mask: u8::MAX,
        name: crate::msg::payload_test_samples::worst_text64(),
        ha_expose: true,
    };

    pub struct InputDeviceNotesUpdateCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub notes: FixedText48,
    }
    budget = InputDeviceNotesUpdateCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        notes: crate::msg::payload_test_samples::worst_text48(),
    };

    pub struct Dali103FeedbackConfigureCommand {
        pub registry_adapter_id: u8,
        pub short_address: u8,
        pub instance_number: u8,
        pub patch_mask: u8,
        pub timing: u8,
        pub active_brightness: u8,
        pub active_colour: u8,
        pub inactive_brightness: u8,
        pub inactive_colour: u8,
        pub opcode_map: u8,
    }
    budget = Dali103FeedbackConfigureCommand {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        instance_number: 31,
        patch_mask: u8::MAX,
        timing: u8::MAX,
        active_brightness: u8::MAX,
        active_colour: 63,
        inactive_brightness: u8::MAX,
        inactive_colour: 63,
        opcode_map: 2,
    };

    pub struct Dali103FeedbackDriveCommand {
        pub registry_adapter_id: u8,
        pub action: u8,
        pub short_address: Option<u8>,
        pub feature_number: Option<u8>,
        pub feature_group: Option<u8>,
        pub selected_group: u8,
        pub opcode_map: u8,
    }
    budget = Dali103FeedbackDriveCommand {
        registry_adapter_id: u8::MAX,
        action: 2,
        short_address: Some(63),
        feature_number: Some(31),
        feature_group: Some(31),
        selected_group: 31,
        opcode_map: 2,
    };

    pub struct RuleStageCommand {
        pub chunk_index: u8,
        pub chunk_count: u8,
        pub bytes: FixedItems<u8, RULE_SOURCE_CHUNK_BYTES>,
    }
    budget = RuleStageCommand {
        chunk_index: u8::MAX,
        chunk_count: u8::MAX,
        bytes: crate::msg::payload_test_samples::worst_rule_chunk(),
    };

    pub struct RuleCommitCommand {
        pub chunk_count: u8,
        pub total_len: u16,
        pub source_hash: u32,
        pub base_revision: u32,
        pub lang_id: u8,
    }
    budget = RuleCommitCommand {
        chunk_count: u8::MAX,
        total_len: u16::MAX,
        source_hash: u32::MAX,
        base_revision: u32::MAX,
        lang_id: u8::MAX,
    };

    pub struct RuleEnableCommand {
        pub name: FixedText64,
        pub enabled: bool,
    }
    budget = RuleEnableCommand {
        name: crate::msg::payload_test_samples::worst_text64(),
        enabled: true,
    };

    pub struct RuleRunCommand {
        pub name: FixedText64,
        pub dry: bool,
    }
    budget = RuleRunCommand {
        name: crate::msg::payload_test_samples::worst_text64(),
        dry: true,
    };

    pub struct MqttPublishCommand {
        pub topic: FixedText48,
        pub payload: FixedText48,
        pub retain: bool,
    }
    budget = MqttPublishCommand {
        topic: crate::msg::payload_test_samples::worst_text48(),
        payload: crate::msg::payload_test_samples::worst_text48(),
        retain: true,
    };

    pub struct VirtualLampDeleteCommand {
        pub adapter_id: u8,
        pub virtual_lamp_id: u8,
    }
    budget = VirtualLampDeleteCommand {
        adapter_id: u8::MAX,
        virtual_lamp_id: 63,
    };

    pub struct PhysicalDeviceDeleteCommand {
        pub adapter_id: u8,
        pub short_address: u8,
    }
    budget = PhysicalDeviceDeleteCommand {
        adapter_id: u8::MAX,
        short_address: 63,
    };

    pub struct FirmwareUpdateBeginCommand {
        pub url: FixedText96,
    }
    budget = FirmwareUpdateBeginCommand {
        url: crate::msg::payload_test_samples::worst_text96(),
    };

    pub struct RedundancySettingsUpdateCommand {
        pub patch_mask: u8,
        pub enabled: bool,
        pub standby_role: bool,
        pub probe_interval_ms: u32,
        pub takeover_after_missed: u8,
        pub boot_listen_ms: u32,
        pub peer_device_short_address: u8,
        pub peer_url: FixedText64,
    }
    budget = RedundancySettingsUpdateCommand {
        patch_mask: u8::MAX,
        enabled: true,
        standby_role: true,
        probe_interval_ms: u32::MAX,
        takeover_after_missed: u8::MAX,
        boot_listen_ms: u32::MAX,
        peer_device_short_address: u8::MAX,
        peer_url: crate::msg::payload_test_samples::worst_text64(),
    };


    pub struct Dali103ArbitrationProbeCommand {
        pub registry_adapter_id: u8,
    }
    budget = Dali103ArbitrationProbeCommand {
        registry_adapter_id: u8::MAX,
    };


    pub struct Dali103HandoverCommand {
        pub registry_adapter_id: u8,
        pub peer_short_address: u8,
    }
    budget = Dali103HandoverCommand {
        registry_adapter_id: u8::MAX,
        peer_short_address: 63,
    };


    pub struct PoliciesUpdateCommand {
        pub patch_mask: u8,
        pub system_failure_level: u8,
        pub power_on_level: u8,
        pub apply_on_discovery: bool,
    }
    budget = PoliciesUpdateCommand {
        patch_mask: u8::MAX,
        system_failure_level: u8::MAX,
        power_on_level: u8::MAX,
        apply_on_discovery: true,
    };


    pub struct PolicyApplyExecuteCommand {
        pub registry_adapter_id: u8,
        pub operation_key: FixedText32,
    }
    budget = PolicyApplyExecuteCommand {
        registry_adapter_id: u8::MAX,
        operation_key: crate::msg::payload_test_samples::worst_text32(),
    };


    pub struct RegistrySliceReloadCommand {
        pub slice_name: FixedText32,
    }
    budget = RegistrySliceReloadCommand {
        slice_name: crate::msg::payload_test_samples::worst_text32(),
    };

    pub struct DaliStopFadeCommand {
        pub registry_adapter_id: u8,
        pub scope: DaliTargetScope,
        pub virtual_lamp_id: u8,
        pub short_address: u8,
        pub group_id: u8,
    }
    budget = DaliStopFadeCommand {
        registry_adapter_id: u8::MAX,
        scope: DaliTargetScope::AddressRange,
        virtual_lamp_id: 63,
        short_address: 63,
        group_id: 15,
    };

    pub struct RegistryLevelTransitionCommand {
        pub adapter_id: u8,
        pub virtual_lamp_id: Option<u8>,
        pub short_address: Option<u8>,
        pub transition: super::kinds::LevelTransition,
        pub observation: Option<RuntimeObservation>,
        pub source: RuntimeSource,
        pub observed_at_mono_ms: Option<u32>,
    }
    budget = RegistryLevelTransitionCommand {
        adapter_id: u8::MAX,
        virtual_lamp_id: Some(63),
        short_address: Some(63),
        transition: crate::msg::kinds::LevelTransition::GoToLastActiveLevel,
        observation: Some(crate::msg::payload_test_samples::worst_observation()),
        source: RuntimeSource::Sniffer,
        observed_at_mono_ms: Some(u32::MAX),
    };
}

impl FirmwareUpdateBeginCommand {
    pub const URL_MAX_BYTES: usize = 96;
}

pub const RULE_SOURCE_CHUNK_BYTES: usize = 96;

pub const MAX_RULES_SOURCE_BYTES: usize = 12_240;

pub const OPERATION_TTL_MS: u32 = 600_000;

pub const DISCOVERY_SCAN_ADDRESSES: u32 = 64;

pub const DISCOVERY_WORST_FRAMES_PER_ADDRESS: u32 = 1 + 15 + 12 + 30 + 1;

pub const DISCOVERY_FRAME_BUDGET_MS: u32 = 500;

pub const DISCOVERY_TTL_MS: u32 =
    DISCOVERY_SCAN_ADDRESSES * DISCOVERY_WORST_FRAMES_PER_ADDRESS * DISCOVERY_FRAME_BUDGET_MS;

pub const CONFIG_WRITE_TTL_MS: u32 = 10_000;

pub const fn operation_ttl_ms(operation_type: OperationType) -> u32 {
    match operation_type {
        OperationType::Discovery => DISCOVERY_TTL_MS,
        OperationType::ConfigWrite => CONFIG_WRITE_TTL_MS,
        OperationType::FirmwareUpdate => FIRMWARE_UPDATE_TTL_MS,
        _ => OPERATION_TTL_MS,
    }
}

pub const FIRMWARE_UPDATE_TTL_MS: u32 = 300_000;
pub const OPERATION_FINISHED_RETENTION_MS: u32 = 60_000;

impl OperationBeginCommand {
    pub fn with_defaults(
        operation_key: &str,
        operation_type: OperationType,
        expected_outcomes: u16,
    ) -> Self {
        Self {
            operation_key: super::bounded::fixed_text_32(operation_key),
            operation_type,
            ttl_ms: operation_ttl_ms(operation_type),
            finished_retention_ms: OPERATION_FINISHED_RETENTION_MS,
            expected_outcomes,
        }
    }
}

impl PhysicalDeviceOverrideCommand {
    pub const PATCH_NAME: u8 = 1;
    pub const PATCH_DEVICE_TYPE_OVERRIDE: u8 = 4;
    pub const PATCH_COLOR_MODE_OVERRIDE: u8 = 8;
    pub const PATCH_DT8_AUTO_ACTIVATION_REPAIR: u8 = 16;
    pub const PATCH_DT8_RGBWAF_CONTROL_ASSERT: u8 = 32;
}

impl PoliciesUpdateCommand {
    pub const PATCH_SYSTEM_FAILURE_LEVEL: u8 = 1;
    pub const PATCH_POWER_ON_LEVEL: u8 = 2;
    pub const PATCH_APPLY_ON_DISCOVERY: u8 = 4;
    pub const UNMANAGED: u8 = u8::MAX;
}

impl RedundancySettingsUpdateCommand {
    pub const PATCH_ENABLED: u8 = 1;
    pub const PATCH_ROLE: u8 = 2;
    pub const PATCH_PROBE_INTERVAL_MS: u8 = 4;
    pub const PATCH_TAKEOVER_AFTER_MISSED: u8 = 8;
    pub const PATCH_BOOT_LISTEN_MS: u8 = 16;
    pub const PATCH_PEER_SHORT_ADDRESS: u8 = 32;
    pub const PATCH_PEER_URL: u8 = 64;
}

impl DaliSettingsUpdateCommand {
    pub const PATCH_DT8_AUTO_ACTIVATION_REPAIR: u8 = 1;
    pub const PATCH_DT8_RGBWAF_CONTROL_ASSERT: u8 = 2;
    pub const PATCH_APPLICATION_ACTIVE: u8 = 4;
    pub const PATCH_DEVICE_SHORT_ADDRESS: u8 = 8;
}

impl VirtualLampConfigUpdateCommand {
    pub const PATCH_NAME: u8 = 1;
    pub const PATCH_HA_ENTITY_ENABLED: u8 = 2;
}

impl AdapterSettingsUpdateCommand {
    pub const PATCH_NAME: u8 = 1;
    pub const PATCH_ENABLED: u8 = 2;
}

impl PollerSettingsUpdateCommand {
    pub const PATCH_ENABLED: u8 = 1;
    pub const PATCH_INTERVAL_MS: u8 = 2;
    pub const PATCH_ATTRIBUTE_GROUPS_MASK: u8 = 8;
    pub const PATCH_INCLUDE_DT8_COLOR: u8 = 16;
    pub const PATCH_SKIP_UNBOUND_VIRTUAL_LAMPS: u8 = 32;
    pub const PATCH_INCLUDE_ENERGY: u8 = 64;
    pub const PATCH_INCLUDE_DIAGNOSTICS: u8 = 128;
}

impl HomeAssistantSettingsUpdateCommand {
    pub const PATCH_ENABLED: u8 = 1;
    pub const PATCH_BROKER_HOST: u8 = 2;
    pub const PATCH_BROKER_PORT: u8 = 4;
    pub const PATCH_PUBLISH_QOS: u8 = 8;
    pub const PATCH_RETAIN_STATE: u8 = 16;
    pub const PATCH_RETAIN_DISCOVERY: u8 = 32;
    pub const PATCH_EXPOSE_INPUT_DEVICES: u8 = 64;

    pub const BROKER_HOST_MAX_BYTES: usize = 64;
}

impl HomeAssistantCredentialsUpdateCommand {
    pub const PATCH_BROKER_USERNAME: u8 = 1;
    pub const PATCH_BROKER_PASSWORD: u8 = 2;

    pub const BROKER_USERNAME_MAX_BYTES: usize = 32;
    pub const BROKER_PASSWORD_MAX_BYTES: usize = 48;
}

impl HomeAssistantTopicsUpdateCommand {
    pub const PATCH_DISCOVERY_PREFIX: u8 = 1;
    pub const PATCH_STATE_TOPIC_PREFIX: u8 = 2;

    pub const PREFIX_MAX_BYTES: usize = 32;
}

impl HomeAssistantControllerIdUpdateCommand {
    pub const CONTROLLER_ID_MAX_BYTES: usize = 32;
}

impl GroupMetadataUpdateCommand {
    pub const PATCH_NAME: u8 = 1;
    pub const PATCH_HA_ENTITY_ENABLED: u8 = 2;
}

impl SceneMetadataUpdateCommand {
    pub const PATCH_NAME: u8 = 1;
    pub const PATCH_HA_SELECT_ENABLED: u8 = 2;
}

impl DaliRecallSceneCommand {
    pub fn broadcast(registry_adapter_id: u8, scene_id: u8) -> Self {
        Self {
            registry_adapter_id,
            scope: DaliTargetScope::Broadcast,
            short_address: 0,
            group_id: 0,
            scene_id,
        }
    }

    pub fn for_group(registry_adapter_id: u8, group_id: u8, scene_id: u8) -> Self {
        Self {
            registry_adapter_id,
            scope: DaliTargetScope::Group,
            short_address: 0,
            group_id,
            scene_id,
        }
    }
}

impl DaliSetTargetStateCommand {
    pub fn for_short(registry_adapter_id: u8, short_address: u8, setpoint: &LightSetpoint) -> Self {
        Self {
            scope: DaliTargetScope::Short,
            virtual_lamp_id: 0,
            short_address,
            group_id: 0,
            setpoint: setpoint.clone(),
            registry_adapter_id,
        }
    }

    pub fn for_virtual_lamp(
        registry_adapter_id: u8,
        virtual_lamp_id: u8,
        setpoint: &LightSetpoint,
    ) -> Self {
        Self {
            scope: DaliTargetScope::VirtualLamp,
            virtual_lamp_id,
            short_address: 0,
            group_id: 0,
            setpoint: setpoint.clone(),
            registry_adapter_id,
        }
    }

    pub fn for_group(registry_adapter_id: u8, group_id: u8, setpoint: &LightSetpoint) -> Self {
        Self {
            scope: DaliTargetScope::Group,
            virtual_lamp_id: 0,
            short_address: 0,
            group_id,
            setpoint: setpoint.clone(),
            registry_adapter_id,
        }
    }
}

impl RegistryRuntimeUpdateCommand {
    pub fn internal(registry_adapter_id: u8, update: RuntimeRegistryUpdateEntry) -> Self {
        Self {
            adapter_id: registry_adapter_id,
            update,
        }
    }
}

impl RuntimeRegistryUpdateEntry {
    pub fn sniffer_level(virtual_lamp_id: u8, level: u8, last_seen_ms: u64) -> Self {
        Self {
            virtual_lamp_id: Some(virtual_lamp_id),
            short_address: None,
            setpoint: Some(LightSetpoint {
                power: super::kinds::PowerState::On,
                level,
                color: Some(super::state::ColorValue::default()),
            }),
            observation: Some(RuntimeObservation::sniffer_timestamped(last_seen_ms)),
            last_dapc_source: None,
            source: RuntimeSource::Sniffer,
            observed_at_mono_ms: None,
        }
    }

    pub fn api_short_physical(short_address: u8, setpoint: &LightSetpoint, last_seen_ms: u64) -> Self {
        Self {
            virtual_lamp_id: None,
            short_address: Some(short_address),
            setpoint: Some(setpoint.clone()),
            observation: Some(RuntimeObservation::api_timestamped(last_seen_ms)),
            last_dapc_source: None,
            source: RuntimeSource::Api,
            observed_at_mono_ms: None,
        }
    }

    pub fn api_virtual_lamp_and_short(
        virtual_lamp_id: u8,
        short_address: u8,
        setpoint: &LightSetpoint,
        last_seen_ms: u64,
    ) -> Self {
        Self {
            virtual_lamp_id: Some(virtual_lamp_id),
            short_address: Some(short_address),
            setpoint: Some(setpoint.clone()),
            observation: Some(RuntimeObservation::api_timestamped(last_seen_ms)),
            last_dapc_source: None,
            source: RuntimeSource::Api,
            observed_at_mono_ms: None,
        }
    }
}

#[cfg(test)]
mod operation_ttl_tests {
    use super::*;

    #[test]
    fn discovery_ttl_covers_the_whole_address_space() {
        let presence = 1;
        let random_address = 5 * 3;
        let verify_attempts = (2 + 1) * 4;
        let verify_rereads = 2 * random_address;
        let withdraw = 1;
        let per_address = presence + random_address + verify_attempts + verify_rereads + withdraw;
        assert_eq!(
            DISCOVERY_WORST_FRAMES_PER_ADDRESS, per_address,
            "the scan's worst-case frame count moved; re-derive DISCOVERY_TTL_MS"
        );
        assert_eq!(
            DISCOVERY_TTL_MS,
            DISCOVERY_SCAN_ADDRESSES * per_address * DISCOVERY_FRAME_BUDGET_MS
        );
        const {
            assert!(
                DISCOVERY_TTL_MS > 350_000 * 2,
                "TTL must clear the measured cost of the phases with room for verify"
            )
        };
    }

    #[test]
    fn the_same_ttl_also_covers_the_part_103_scan() {
        const MAX_INSTANCES: u32 = 32;
        let presence = 1;
        let instance_types = MAX_INSTANCES;
        let feedback_probe = 2 * MAX_INSTANCES;
        let instance_facts = 2 * MAX_INSTANCES;
        let declarations = 3;
        let per_device = presence + instance_types + feedback_probe + instance_facts + declarations;
        let worst = DISCOVERY_SCAN_ADDRESSES * per_device;

        const MEASURED_MAX_FRAME_MS: u32 = 100;
        assert!(
            DISCOVERY_TTL_MS > worst * MEASURED_MAX_FRAME_MS,
            "the 103 scan's worst case ({worst} frames) no longer fits the \
             discovery TTL at the measured {MEASURED_MAX_FRAME_MS} ms/frame"
        );
    }

    #[test]
    fn the_ttl_table_answers_for_the_kinds_that_are_not_generic() {
        assert_eq!(operation_ttl_ms(OperationType::Discovery), DISCOVERY_TTL_MS);
        assert_eq!(operation_ttl_ms(OperationType::ConfigWrite), CONFIG_WRITE_TTL_MS);
        assert_eq!(operation_ttl_ms(OperationType::GroupApply), OPERATION_TTL_MS);
        assert_eq!(
            OperationBeginCommand::with_defaults("k", OperationType::Discovery, 0).ttl_ms,
            DISCOVERY_TTL_MS,
            "the begin command must carry the kind's budget, not the generic one"
        );
    }
}

#[cfg(test)]
mod ha_width_tests {
    use super::*;
    use crate::msg::bounded::{fixed_text_32, fixed_text_48, fixed_text_64};

    #[test]
    fn the_declared_ha_caps_match_the_fixed_text_widths_they_bound() {
        let long = "x".repeat(256);
        assert_eq!(
            fixed_text_64(&long).len(),
            HomeAssistantSettingsUpdateCommand::BROKER_HOST_MAX_BYTES
        );
        assert_eq!(
            fixed_text_32(&long).len(),
            HomeAssistantCredentialsUpdateCommand::BROKER_USERNAME_MAX_BYTES
        );
        assert_eq!(
            fixed_text_48(&long).len(),
            HomeAssistantCredentialsUpdateCommand::BROKER_PASSWORD_MAX_BYTES
        );
        assert_eq!(
            fixed_text_32(&long).len(),
            HomeAssistantTopicsUpdateCommand::PREFIX_MAX_BYTES
        );
        assert_eq!(
            fixed_text_32(&long).len(),
            HomeAssistantControllerIdUpdateCommand::CONTROLLER_ID_MAX_BYTES
        );
    }
}
