use std::borrow::Cow;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationErrorView {
    pub code: Cow<'static, str>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationGroupApplyOutcomeView {
    pub virtual_lamp_id: u8,
    pub group_id: u8,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_short_address: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationGroupApplyResultView {
    pub programmed: Vec<OperationGroupApplyOutcomeView>,
    pub skipped: Vec<OperationGroupApplyOutcomeView>,
    pub failed: Vec<OperationGroupApplyOutcomeView>,
    #[serde(default)]
    pub programmed_total: u16,
    #[serde(default)]
    pub skipped_total: u16,
    #[serde(default)]
    pub failed_total: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAttributeReadOutcomesView {
    pub identity: String,
    pub runtime_status: String,
    pub common_102: String,
    pub dt8_color: String,
    pub dt6_led: String,
    pub dt7_switching: String,
    pub groups: String,
    pub scenes: String,
    pub extended: String,
    pub memory_banks: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationSceneApplyOutcomeView {
    pub virtual_lamp_id: u8,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_short_address: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationSceneApplyResultView {
    pub written: Vec<OperationSceneApplyOutcomeView>,
    pub updated: Vec<OperationSceneApplyOutcomeView>,
    pub cleared: Vec<OperationSceneApplyOutcomeView>,
    pub skipped: Vec<OperationSceneApplyOutcomeView>,
    pub failed: Vec<OperationSceneApplyOutcomeView>,
    #[serde(default)]
    pub written_total: u16,
    #[serde(default)]
    pub updated_total: u16,
    #[serde(default)]
    pub cleared_total: u16,
    #[serde(default)]
    pub skipped_total: u16,
    #[serde(default)]
    pub failed_total: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationPolicyApplyOutcomeView {
    pub short_address: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationPolicyApplyResultView {
    pub written: Vec<OperationPolicyApplyOutcomeView>,
    pub skipped: Vec<OperationPolicyApplyOutcomeView>,
    pub failed: Vec<OperationPolicyApplyOutcomeView>,
    pub written_total: u16,
    pub skipped_total: u16,
    pub failed_total: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OperationApplyResultView {
    GroupApply(OperationGroupApplyResultView),
    SceneApply(OperationSceneApplyResultView),
    Identify(OperationIdentifyResultView),
    AddressChange(OperationAddressChangeResultView),
    ReplaceDevice(OperationReplaceDeviceResultView),
    HaDiscoveryPublish(OperationHaDiscoveryResultView),
    PolicyApply(OperationPolicyApplyResultView),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationHaDiscoveryResultView {
    pub entities_published: u16,
    pub entities_failed: u16,
}

impl OperationApplyResultView {
    pub fn as_ha_discovery_publish(&self) -> Option<&OperationHaDiscoveryResultView> {
        match self {
            Self::HaDiscoveryPublish(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_group_apply(&self) -> Option<&OperationGroupApplyResultView> {
        match self {
            Self::GroupApply(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_scene_apply(&self) -> Option<&OperationSceneApplyResultView> {
        match self {
            Self::SceneApply(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_identify(&self) -> Option<&OperationIdentifyResultView> {
        match self {
            Self::Identify(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_address_change(&self) -> Option<&OperationAddressChangeResultView> {
        match self {
            Self::AddressChange(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_replace_device(&self) -> Option<&OperationReplaceDeviceResultView> {
        match self {
            Self::ReplaceDevice(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_policy_apply(&self) -> Option<&OperationPolicyApplyResultView> {
        match self {
            Self::PolicyApply(result) => Some(result),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationIdentifyResultView {
    pub short_address: u8,
    pub identify_mechanism: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAddressChangeResultView {
    pub old_short_address: u8,
    pub new_short_address: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRestoredSlicesView {
    pub metadata_and_overrides: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationReplaceDeviceResultView {
    pub failed_short_address: u8,
    pub replacement_short_address: u8,
    pub restored: OperationRestoredSlicesView,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationView {
    pub operation_id: String,
    #[serde(rename = "type")]
    pub operation_type: Cow<'static, str>,
    pub status: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationErrorView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<OperationApplyResultView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute_read_outcomes: Option<OperationAttributeReadOutcomesView>,
}
