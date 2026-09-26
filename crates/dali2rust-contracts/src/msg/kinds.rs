use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ChannelKind {
    Commands = 0,
    Confirmations = 1,
    Events = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageKind {
    Command = 0,
    Confirmation = 1,
    Event = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DeliveryStatus {
    Ok = 0,
    DeliveryRejected = 1,
    ExecutionFailed = 2,
    TimedOut = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Origin {
    Api = 0,
    Mqtt = 1,
    Hcl = 2,
    Cluster = 3,
    AdapterProxy = 4,
    Sniffer = 5,
    Poller = 6,
    Registry = 7,
    #[default]
    Internal = 8,
    Rules = 9,
}

impl Origin {
    pub const fn is_background(self) -> bool {
        matches!(self, Self::Poller)
    }
}

// IEC 62386-103 §9.5.1
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceCommandScope {
    Broadcast,
    Unaddressed,
    Short(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DaliTargetScope {
    VirtualLamp = 0,
    Short = 1,
    Group = 2,
    Broadcast = 3,
    AddressRange = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ConfigWriteResource {
    GroupMatrix = 0,
    SceneMatrix = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DiscoveryMode {
    ScanKnownShortAddresses = 0,
    CommissionUnaddressed = 1,
    RefreshKnown = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DaliAttributeGroup {
    RuntimeStatus = 0,
    Common102 = 1,
    Dt8Color = 2,
    Dt6Led = 3,
    Groups = 4,
    Scenes = 5,
    Extended = 6,
    SceneColours = 7,
}

impl DaliAttributeGroup {
    pub const ALL: [Self; 8] = [
        Self::RuntimeStatus,
        Self::Common102,
        Self::Dt8Color,
        Self::Dt6Led,
        Self::Groups,
        Self::Scenes,
        Self::Extended,
        Self::SceneColours,
    ];

    pub const fn mask_bit(self) -> u8 {
        1 << (self as u8)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeStatus => "runtime_status",
            Self::Common102 => "common_102",
            Self::Dt8Color => "dt8_color",
            Self::Dt6Led => "dt6_led",
            Self::Groups => "groups",
            Self::Scenes => "scenes",
            Self::Extended => "extended",
            Self::SceneColours => "scene_colours",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.as_str() == s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MemoryBankReadPreset {
    #[default]
    None = 0,
    Identity = 1,
    Profile = 2,
    All = 3,
    Power = 4,
    Energy = 5,
    Diagnostics = 6,
    LuminaireData = 7,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AttributeGroupReadOutcome {
    #[default]
    NotRequested = 0,
    Success = 1,
    ContendedAbort = 2,
    TransportAbort = 3,
    NotAttempted = 4,
    Preempted = 5,
    DeviceAbsent = 6,
    SequenceIncomplete = 7,
}

impl AttributeGroupReadOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AttributeGroupReadOutcome::NotRequested => "not_requested",
            AttributeGroupReadOutcome::Success => "success",
            AttributeGroupReadOutcome::ContendedAbort => "contended_abort",
            AttributeGroupReadOutcome::TransportAbort => "transport_abort",
            AttributeGroupReadOutcome::NotAttempted => "not_attempted",
            AttributeGroupReadOutcome::Preempted => "preempted",
            AttributeGroupReadOutcome::DeviceAbsent => "device_absent",
            AttributeGroupReadOutcome::SequenceIncomplete => "sequence_incomplete",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum OperationType {
    Discovery = 0,
    AttributeRead = 1,
    MemoryBankRead = 2,
    GroupApply = 3,
    SceneApply = 4,
    HaDiscoveryPublish = 5,
    CommissioningIdentify = 6,
    CommissioningAddressChange = 7,
    CommissioningReplaceDevice = 8,
    AttributeWrite = 9,
    ConfigWrite = 10,
    FirmwareUpdate = 11,
    PolicyApply = 12,
}

impl OperationType {
    pub const fn rest_name(self) -> &'static str {
        match self {
            Self::ConfigWrite => "config_write",
            Self::FirmwareUpdate => "firmware_update",
            Self::Discovery => "discovery",
            Self::AttributeRead => "attribute_read",
            Self::MemoryBankRead => "memory_bank_read",
            Self::GroupApply => "group_apply",
            Self::SceneApply => "scene_apply",
            Self::HaDiscoveryPublish => "ha_discovery_publish",
            Self::CommissioningIdentify => "commissioning_identify",
            Self::CommissioningAddressChange => "commissioning_address_change",
            Self::CommissioningReplaceDevice => "commissioning_replace_device",
            Self::AttributeWrite => "attribute_write",
            Self::PolicyApply => "policy_apply",
        }
    }

    pub const fn coalesces_per_adapter(self) -> bool {
        !matches!(self, Self::ConfigWrite)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum OperationStatus {
    Accepted = 0,
    Running = 1,
    Succeeded = 2,
    Failed = 3,
    TimedOut = 4,
    Cancelled = 5,
}

impl OperationStatus {
    pub const fn rest_name(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }

    pub fn from_rest_name(name: &str) -> Option<Self> {
        [
            Self::Accepted,
            Self::Running,
            Self::Succeeded,
            Self::Failed,
            Self::TimedOut,
            Self::Cancelled,
        ]
        .into_iter()
        .find(|status| status.rest_name() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum OperationWorkerSignal {
    WorkerStarted = 0,
    WorkerSucceeded = 1,
    WorkerFailed = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum LastDapcSource {
    #[default]
    Unknown = 0,
    Sniffer = 1,
    Scene = 2,
    Group = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum RuntimeSource {
    Poller = 0,
    Sniffer = 1,
    Api = 2,
    Mqtt = 3,
    Hcl = 4,
    Cluster = 5,
    AdapterProxy = 6,
    Rules = 7,
    Readback = 8,
}

impl RuntimeSource {
    pub fn from_origin(origin: Origin) -> Option<Self> {
        match origin {
            Origin::Api => Some(Self::Api),
            Origin::Mqtt => Some(Self::Mqtt),
            Origin::Hcl => Some(Self::Hcl),
            Origin::Cluster => Some(Self::Cluster),
            Origin::AdapterProxy => Some(Self::AdapterProxy),
            Origin::Sniffer => Some(Self::Sniffer),
            Origin::Poller => Some(Self::Poller),
            Origin::Rules => Some(Self::Rules),
            Origin::Registry | Origin::Internal => None,
        }
    }

    pub const fn rest_name(self) -> &'static str {
        match self {
            Self::Poller => "poller",
            Self::Sniffer => "sniffer",
            Self::Api => "api",
            Self::Mqtt => "mqtt",
            Self::Hcl => "hcl",
            Self::Cluster => "cluster",
            Self::AdapterProxy => "adapter_proxy",
            Self::Rules => "rules",
            Self::Readback => "readback",
        }
    }
}

impl LastDapcSource {
    pub const fn rest_name(self) -> Option<&'static str> {
        match self {
            Self::Unknown => None,
            Self::Sniffer => Some("sniffer"),
            Self::Scene => Some("scene"),
            Self::Group => Some("group"),
        }
    }
}

impl PowerState {
    pub const fn rest_name(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Unknown => "unknown",
        }
    }
}

impl ColorMode {
    pub const fn rest_name(self) -> &'static str {
        match self {
            Self::Brightness => "brightness",
            Self::Cct => "cct",
            Self::Xy => "xy",
            Self::Rgb => "rgb",
            Self::Rgbwaf => "rgbwaf",
            Self::None | Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum PowerState {
    Off = 0,
    On = 1,
    #[default]
    Unknown = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum DeviceType {
    #[default]
    Unknown = 0,
    Dt0Fluorescent = 1,
    Dt1Emergency = 2,
    Dt2Discharge = 3,
    Dt3LowVoltageHalogen = 4,
    Dt4Incandescent = 5,
    Dt5Dc = 6,
    Dt6Led = 7,
    Dt7Switching = 8,
    Dt8Color = 9,
    Other = 10,
}

impl DeviceType {
    pub const fn dali_code(self) -> Option<u8> {
        match self {
            Self::Dt0Fluorescent => Some(0),
            Self::Dt1Emergency => Some(1),
            Self::Dt2Discharge => Some(2),
            Self::Dt3LowVoltageHalogen => Some(3),
            Self::Dt4Incandescent => Some(4),
            Self::Dt5Dc => Some(5),
            Self::Dt6Led => Some(6),
            Self::Dt7Switching => Some(7),
            Self::Dt8Color => Some(8),
            Self::Unknown | Self::Other => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct DeviceTypeSet(u64);

impl DeviceTypeSet {
    pub const MAX_TYPE: u8 = 63;

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, device_type: u8) -> bool {
        device_type <= Self::MAX_TYPE && (self.0 >> device_type as u32) & 1 == 1
    }

    pub fn insert(&mut self, device_type: u8) -> bool {
        if device_type > Self::MAX_TYPE {
            return false;
        }
        self.0 |= 1u64 << device_type;
        true
    }

    pub fn iter(self) -> impl Iterator<Item = u8> {
        (0..=Self::MAX_TYPE).filter(move |device_type| self.contains(*device_type))
    }
}

#[cfg(test)]
mod device_type_set_tests {
    use super::{DeviceType, DeviceTypeSet};

    #[test]
    fn bit_n_is_device_type_n_across_the_allocated_space() {
        let mut set = DeviceTypeSet::default();
        for device_type in [0, 6, 8, 23, 52] {
            assert!(set.insert(device_type));
        }
        assert!(set.iter().eq([0, 6, 8, 23, 52]));
        assert!(set.contains(52));
        assert!(!set.contains(51));
        assert!(!set.is_empty());
    }

    #[test]
    fn a_code_past_the_mask_is_refused_rather_than_dropped() {
        let mut set = DeviceTypeSet::default();
        assert!(set.insert(63));
        assert!(!set.insert(64));
        assert!(!set.contains(64));
        assert!(set.iter().eq([63]));
    }

    #[test]
    fn the_dali_code_is_not_the_discriminant() {
        assert_eq!(DeviceType::Dt6Led.dali_code(), Some(6));
        assert_eq!(DeviceType::Dt8Color.dali_code(), Some(8));
        assert_eq!(DeviceType::Dt0Fluorescent.dali_code(), Some(0));
        assert_eq!(DeviceType::Unknown.dali_code(), None);
        assert_eq!(DeviceType::Other.dali_code(), None);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum ColorMode {
    #[default]
    None = 0,
    Brightness = 1,
    Cct = 2,
    Xy = 3,
    Rgb = 4,
    Rgbwaf = 5,
    Unknown = 6,
}

impl ColorMode {
    #[must_use]
    pub const fn states_a_colour(self) -> bool {
        !matches!(self, Self::None | Self::Unknown)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CommissioningStep {
    Initialise = 0,
    Randomise = 1,
    SearchAddress = 2,
    Compare = 3,
    ProgramShortAddress = 4,
    VerifyShortAddress = 5,
    QueryShortAddress = 6,
    Withdraw = 7,
    Terminate = 8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InitialiseScope {
    All = 0,
    Unaddressed = 1,
    Short = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum IdentifyMechanism {
    BlinkRecallMaxMin = 0,
    IdentifyDevice = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ObservedKind {
    TargetStateObserved = 0,
    SceneRecallObserved = 1,
    UnknownObserved = 2,
    LevelTransitionObserved = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum LevelTransition {
    RecallMaxLevel = 0,
    RecallMinLevel = 1,
    StepUp = 2,
    StepDown = 3,
    StepDownAndOff = 4,
    OnAndStepUp = 5,
    GoToLastActiveLevel = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DecodeStatus {
    Decoded = 0,
    Unsupported = 1,
    Ambiguous = 2,
    Invalid = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ObservedFrameWidth {
    Forward16 = 0,
    Forward24 = 1,
    Backward8 = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InputEventKind {
    Generic = 0,
    Button = 1,
    Occupancy = 2,
    Position = 3,
    Illuminance = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InputDeviceLifecycleKind {
    PowerCycle = 0,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum HclAlgorithm {
    Stepped = 0,
    Interpolated = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum HclTimeRef {
    Absolute = 0,
    Sunrise = 1,
    Sunset = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum HclLevelMode {
    Absolute = 0,
    LastActive = 1,
    None = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum HclTargetScope {
    Group = 0,
    Broadcast = 1,
}

#[cfg(test)]
mod status_name_tests {
    use super::OperationStatus;

    #[test]
    fn every_status_round_trips_through_its_rest_name() {
        for status in [
            OperationStatus::Accepted,
            OperationStatus::Running,
            OperationStatus::Succeeded,
            OperationStatus::Failed,
            OperationStatus::TimedOut,
            OperationStatus::Cancelled,
        ] {
            assert_eq!(
                OperationStatus::from_rest_name(status.rest_name()),
                Some(status),
                "{} did not round-trip",
                status.rest_name()
            );
        }
        assert_eq!(OperationStatus::from_rest_name("running "), None);
        assert_eq!(OperationStatus::from_rest_name(""), None);
    }

    #[test]
    fn only_the_four_ended_states_are_terminal() {
        assert!(!OperationStatus::Accepted.is_terminal());
        assert!(!OperationStatus::Running.is_terminal());
        for status in [
            OperationStatus::Succeeded,
            OperationStatus::Failed,
            OperationStatus::TimedOut,
            OperationStatus::Cancelled,
        ] {
            assert!(status.is_terminal(), "{}", status.rest_name());
        }
    }
}
