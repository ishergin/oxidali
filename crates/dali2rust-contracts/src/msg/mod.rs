pub mod bounded;
pub mod commands;
pub mod envelopes;
pub mod errors;
pub mod events;
pub mod kinds;
pub mod observation_order;
pub(crate) mod payload_macros;
#[cfg(test)]
pub(crate) mod payload_test_samples;
pub mod state;
pub mod wire;

pub use bounded::{
    fixed_text_32, fixed_text_48, fixed_text_64, fixed_text_96, FixedBytes24, FixedBytes32, FixedItems, FixedText,
    FixedText32, FixedText48, FixedText64, FixedText96,
};
pub use commands::{
    MAX_RULES_SOURCE_BYTES, RULE_SOURCE_CHUNK_BYTES,operation_ttl_ms, BusCommandPayload, COMMAND_VARIANT_NAMES};
pub use envelopes::{BusEnvelope, CommandEnvelope, ConfirmationEnvelope, EventEnvelope};
pub use errors::{CompactErrorPayload, ErrorCode, ErrorPayload};
pub use events::{
    RulesActivationEvent, RulesChangedEvent,BusEventPayload, DaliAttributeReadChunk, EVENT_VARIANT_NAMES};
pub use observation_order::{observation_supersedes, OBSERVATION_ORDER_WINDOW_MS};
pub use kinds::{
    AttributeGroupReadOutcome, ChannelKind, ColorMode, ConfigWriteResource, DaliAttributeGroup,
    DaliTargetScope,
    DecodeStatus, DeliveryStatus, DeviceType, DeviceTypeSet, DiscoveryMode, LastDapcSource,
    LevelTransition,
    MemoryBankReadPreset,
    CommissioningStep, IdentifyMechanism, InitialiseScope, InputDeviceLifecycleKind,
    InputEventKind, MessageKind, ObservedFrameWidth, ObservedKind, OperationStatus,
    OperationType, OperationWorkerSignal, Origin, PowerState, RuntimeSource,
};
pub use state::{
    CapabilityFlags, ColorValue, Dt6ReadSnapshot, FailureStatus, Level, LightSetpoint,
    RuntimeObservation, SceneRow, SetpointDimensions, StatusFlags,
};
pub use wire::{DaliCommandPayload, DaliConfirmationPayload, DaliEventPayload};

pub use commands::{
    Dali103CommissionCommand, Dali103FeedbackConfigureCommand, Dali103FeedbackDriveCommand,
    Dali103IdentifyCommand, Dali103InstanceConfigureCommand, MqttPublishCommand, RuleCommitCommand,
    RuleEnableCommand, RuleRunCommand, RuleStageCommand,
    Dali103ScanCommand, InputDeviceMetadataUpdateCommand, InputDeviceNotesUpdateCommand,
    DaliAddressingCommand, DaliCommissioningStepCommand, DaliIdentifyDeviceCommand,
    DaliReplaceDeviceCommand,
    AdapterSettingsUpdateCommand, ConfigWriteCommitCommand, DaliDiscoverDevicesCommand,
    DaliProgramGroupMembershipCommand,
    DaliProgramTarget, DaliReadAttributesCommand, DaliReadMemoryBankCommand,
    DaliSetTargetStateCommand, DaliWriteAttributesCommand, GroupMatrixDesiredPatchCommand,
    GroupMatrixDesiredReplaceCommand, GroupMatrixDesiredRow, GroupMatrixDesiredRowList,
    GroupMembershipAction, GroupMetadataUpdateCommand, MAX_GROUP_MATRIX_ROWS_PER_COMMAND,
    GroupApplyExecuteCommand, OperationBeginCommand, OperationRegistryResetCommand,
    PhysicalDeviceDeleteCommand, PhysicalDeviceNotesUpdateCommand, PhysicalDeviceOverrideCommand,
    RegistryRuntimeUpdateCommand,
    RuntimeRegistryUpdateEntry,
    VirtualLampBindCommand, VirtualLampConfigUpdateCommand, VirtualLampDeleteCommand,
    VirtualLampRebindCommand,
    VirtualLampUnbindCommand, CONFIG_WRITE_TTL_MS, DISCOVERY_FRAME_BUDGET_MS,
    DISCOVERY_SCAN_ADDRESSES, DISCOVERY_TTL_MS, DISCOVERY_WORST_FRAMES_PER_ADDRESS,
    OPERATION_FINISHED_RETENTION_MS, OPERATION_TTL_MS,
};
pub use commands::{
    DaliProgramSceneCommand, DaliRecallSceneCommand, DaliSceneTargetState, SceneMatrixDesiredPatchCommand,
    SceneMatrixDesiredReplaceCommand, SceneMatrixDesiredRow, SceneMatrixDesiredRowList,
    SceneApplyExecuteCommand, SceneMetadataUpdateCommand, SceneProgramAction, MAX_SCENE_MATRIX_ROWS_PER_COMMAND,
};
pub use events::{
    AdapterSettingsChangedEvent, DaliAttributeReadOutcomesEvent, DaliAttributesReadEvent,
    DaliAttributesWrittenEvent, DaliDiscoveryCompletedEvent, DaliDiscoveryFailedEvent,
    DaliDiscoveryProgressEvent, DaliDiscoveryScanReconciledEvent,
    DaliAddressingCompletedEvent, DaliDeviceIdentifiedEvent, DaliDeviceReplacedEvent, DaliGroupMembershipProgrammedEvent,
    MAX_PERSISTENCE_ERRORS, MAX_PERSISTENCE_SLICES,
    DaliMemoryBankReadAbortedEvent, DaliMemoryBankReadEvent, DaliObservedFrameEvent,
    DaliTargetStateAppliedEvent,
    DaliTargetStateFailedEvent, IpAddressAssignedEvent, OperationStatusChangedEvent,
    OperationWorkerSignalEvent, PersistenceLoadResultEvent, PersistenceSliceError,
    PersistenceSliceErrorList, PersistenceSliceKind, PersistenceSliceList, PersistenceSliceOutcome,
    DaliSceneProgrammedEvent, DaliSceneRecalledEvent, PhysicalDeviceChangedEvent, GroupChangedEvent,
    GroupMatrixChangedEvent, RuntimeStateChangedEvent, SceneChangedEvent, SceneMatrixChangedEvent,
    StatsReportedEvent, VirtualLampChangedEvent,
};

pub use commands::{
    DaliRecallLastActiveLevelCommand, HclOverrideClearCommand, HclPointList, HclSchedulePointRow,
    HclScheduleDeleteCommand, HclScheduleUpsertCommand, HclTargetList, HclTargetRow,
    MAX_HCL_POINTS_PER_COMMAND, MAX_HCL_TARGETS_PER_COMMAND,
};
pub use events::HclScheduleChangedEvent;
pub use kinds::{HclAlgorithm, HclLevelMode, HclTargetScope, HclTimeRef};

pub use commands::DaliSettingsUpdateCommand;
pub use commands::{
    Dali103ArbitrationProbeCommand, Dali103HandoverCommand, PoliciesUpdateCommand,
    PolicyApplyExecuteCommand, RedundancySettingsUpdateCommand, RegistrySliceReloadCommand,
};
pub use commands::DaliBusHealthProbeCommand;
pub use commands::DaliStopFadeCommand;
pub use commands::RegistryLevelTransitionCommand;
pub use commands::PollerSettingsUpdateCommand;
pub use events::DaliSettingsChangedEvent;
pub use events::{
    Dali103ArbitrationProbedEvent, PoliciesChangedEvent, Dali103HandoverSentEvent, RegistrySliceReloadedEvent, APPLICATION_ACTIVE_UNMOVED, APPLICATION_ACTIVE_MOVED_BY_COMMAND, APPLICATION_ACTIVE_MOVED_BY_WIRE, RedundancySettingsChangedEvent,
    RedundancyTransitionEvent,
};
pub use events::{
    BusHealthVerdict, Dali103ApplicationControlObservedEvent, Dali103InstanceConfiguredEvent,
    Dali103ScanProgressEvent,
    Dali103ScanStartedEvent,
    DaliBusHealthProbedEvent, DaliInputDeviceLifecycleEvent, DaliInputEventObservedEvent,
    InputDeviceChangedEvent,
};
pub use events::PollerSettingsChangedEvent;
pub use commands::{
    HomeAssistantControllerIdUpdateCommand, HomeAssistantCredentialsUpdateCommand,
    HomeAssistantSettingsUpdateCommand, HomeAssistantTopicsUpdateCommand,
};
pub use commands::HomeAssistantDiscoveryPublishCommand;
pub use events::{HomeAssistantDiscoveryPublishedEvent, HomeAssistantSettingsChangedEvent};
