use std::borrow::Cow;

use dali2rust_contracts::msg::{DeviceTypeSet, OperationType};

use crate::dali::banks::luminaire_write::Bank1Layout;
use crate::dali::banks::IdentityRead;

use super::apply_diff::{GroupApplySnapshot, SceneApplySnapshot};
use super::attributes::{AttributeSectionKind, AttributeSectionView};
use super::home_assistant::{
    HaGroupView, HaInputView, HaLampView, HaSceneView, HomeAssistantSettingsView,
};
use super::limits::{GROUP_COUNT, VIRTUAL_LAMP_COUNT};
use super::operations::OperationView;
use super::physical_device::{
    CapabilityFlagsView, MemoryBankSummaryView, PhysicalDeviceCoreView, PhysicalDeviceSummaryView,
};
use super::product_objects::{
    AdapterSnapshot, AdapterView, GroupMembershipMatrixView, GroupView, SceneMatrixView, SceneView,
    VirtualLampCapabilityView, VirtualLampSnapshot, VirtualLampView,
};
use super::settings::{
    ApplicationActiveMover, DaliSettingsView, HclOverrideView, HclScheduleView, PoliciesView,
    PolicyApplyCell, PollTargetSelection, PollerSettingsView, RedundancySettingsView,
};

pub trait HclScheduleReadPort: Send + Sync {
    fn hcl_schedule_view(&self, schedule_id: &str) -> Option<HclScheduleView>;
    fn list_hcl_schedule_views(&self) -> Vec<HclScheduleView>;

    fn hcl_schedule_id_taken(&self, schedule_id: &str) -> bool {
        self.hcl_schedule_view(schedule_id).is_some()
    }
}

pub trait HclSchedulerReadPort: HclScheduleReadPort + GroupReadPort {}
impl<T: HclScheduleReadPort + GroupReadPort> HclSchedulerReadPort for T {}

pub trait HclOverrideReadPort: Send + Sync {
    fn hcl_override_view(&self, schedule_id: &str) -> HclOverrideView;
}

pub trait PollerSettingsReadPort: Send + Sync {
    fn poller_settings_view(&self) -> PollerSettingsView;
}

pub trait DaliSettingsReadPort: Send + Sync {
    fn dali_settings_view(&self) -> DaliSettingsView;

    fn application_active_moved_by(&self) -> (bool, ApplicationActiveMover) {
        (
            self.dali_settings_view().application_active,
            ApplicationActiveMover::Unmoved,
        )
    }

    fn quiescent_remaining_ms(&self) -> Option<u32> {
        None
    }
}

pub trait RedundancySettingsReadPort: Send + Sync {
    fn redundancy_settings_view(&self) -> RedundancySettingsView;
}

pub trait RedundancySettingsApplyWatchPort: Send + Sync {
    fn redundancy_settings_applied_load(&self) -> u32;
}

pub trait PoliciesReadPort: Send + Sync {
    fn policies_view(&self) -> PoliciesView;
}

pub trait PoliciesApplyWatchPort: Send + Sync {
    fn policies_applied_load(&self) -> u32;
}

pub trait PollTargetReadPort: Send + Sync {
    fn list_poll_targets(&self, adapter_id: u8) -> PollTargetSelection;
}

pub trait PollerReadPort: PollerSettingsReadPort + PollTargetReadPort {}
impl<T: PollerSettingsReadPort + PollTargetReadPort> PollerReadPort for T {}

pub trait HomeAssistantSettingsReadPort: Send + Sync {
    fn home_assistant_settings_view(&self) -> HomeAssistantSettingsView;
}

pub trait HaPublishReadPort: Send + Sync {
    fn ha_lamp_views(&self, adapter_id: u8) -> Vec<HaLampView>;
    fn ha_input_views(&self, adapter_id: u8) -> Vec<HaInputView>;
    fn ha_input_view(&self, adapter_id: u8, short_address: u8) -> Option<HaInputView>;
    fn ha_group_views(&self, adapter_id: u8) -> Vec<HaGroupView>;
    fn ha_scene_views(&self, adapter_id: u8) -> Vec<HaSceneView>;
    fn ha_lamp_view(&self, adapter_id: u8, virtual_lamp_id: u8) -> Option<HaLampView>;
    fn ha_group_view(&self, adapter_id: u8, group_id: u8) -> Option<HaGroupView>;
    fn ha_lamp_ids_for_short(&self, adapter_id: u8, short_address: u8) -> Vec<u8>;
    fn ha_retracted_lamp_ids(&self, adapter_id: u8) -> Vec<u8>;
    fn ha_retracted_group_ids(&self, adapter_id: u8) -> Vec<u8>;
    fn ha_retracted_input_views(&self, adapter_id: u8) -> Vec<HaInputView>;
    fn ha_active_scene(&self, adapter_id: u8) -> Option<u8>;
}

pub trait HomeAssistantSecretReadPort: Send + Sync {
    fn home_assistant_broker_password(&self) -> String;
}

pub trait RegistryReadPort: InputInstanceTypeReadPort {
    fn application_controller_active(&self) -> bool;
    fn controller_quiescent(&self) -> bool;
    fn virtual_lamp_snapshot(&self, adapter_id: u8, virtual_lamp_id: u8) -> VirtualLampSnapshot;
    fn virtual_lamp_binding_short(&self, adapter_id: u8, virtual_lamp_id: u8) -> Option<u8>;
    fn adapter_snapshot(&self, adapter_id: u8) -> AdapterSnapshot;
    fn known_physical_short_addresses(&self, adapter_id: u8) -> Vec<u8>;
    fn first_free_short_address(&self, adapter_id: u8) -> Option<u8>;
    fn physical_dt8_gear_features(&self, adapter_id: u8, short_address: u8) -> Option<u8>;
    fn physical_dt8_auto_activation_repair_allowed(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> bool;
    fn apply_on_discovery_armed(&self, adapter_id: u8) -> bool;
    fn physical_dt8_rgbwaf_control_assert_allowed(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> bool;
    fn physical_device_identity(&self, adapter_id: u8, short_address: u8) -> IdentityRead;
}

pub trait AdapterEnabledReadPort: Send + Sync {
    fn adapter_enabled(&self, adapter_id: u8) -> bool;
}

pub trait AdapterReadPort: Send + Sync {
    fn adapter_count(&self) -> u8;
    fn adapter_view(&self, adapter_id: u8) -> Option<AdapterView>;
    fn list_adapter_views(&self) -> Vec<AdapterView>;
}

pub trait PhysicalDeviceReadPort: AdapterReadPort {
    fn physical_device_summary_view(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceSummaryView>;

    fn physical_device_core_view(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceCoreView>;

    fn physical_device_attribute_section(
        &self,
        adapter_id: u8,
        short_address: u8,
        kind: AttributeSectionKind,
    ) -> Option<AttributeSectionView>;

    fn physical_device_memory_banks(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<Vec<MemoryBankSummaryView>>;

    fn physical_device_capabilities(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<CapabilityFlagsView>;

    fn physical_device_supported_types(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<DeviceTypeSet>;

    fn physical_device_known_types(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<DeviceTypeSet>;

    fn physical_device_bank1_layout(&self, adapter_id: u8, short_address: u8) -> Option<Bank1Layout>;

    fn physical_device_identity(&self, adapter_id: u8, short_address: u8) -> IdentityRead;

    fn physical_device_exists(&self, adapter_id: u8, short_address: u8) -> bool;

    fn list_physical_device_short_addresses(&self, adapter_id: u8) -> Vec<u8>;
}

pub trait VirtualLampReadPort: PhysicalDeviceReadPort {
    fn virtual_lamp_view(&self, adapter_id: u8, lamp_id: u8) -> VirtualLampView;
    fn list_virtual_lamp_ids(&self, adapter_id: u8) -> Vec<u8>;
    fn list_virtual_lamp_views(&self, adapter_id: u8) -> Vec<VirtualLampView>;
    fn physical_short_on_other_adapter(&self, adapter_id: u8, short_address: u8) -> bool;
    fn virtual_lamp_bound_to_short(&self, adapter_id: u8, short_address: u8) -> Option<u8>;
}

pub trait VirtualLampCapabilityReadPort: Send + Sync {
    fn virtual_lamp_capability_view(
        &self,
        adapter_id: u8,
        lamp_id: u8,
    ) -> VirtualLampCapabilityView;
    fn list_virtual_lamp_capability_views(&self, adapter_id: u8)
        -> Vec<VirtualLampCapabilityView>;
}

pub trait GroupReadPort: AdapterReadPort {
    fn group_view(&self, adapter_id: u8, group_id: u8) -> Option<GroupView>;
    fn list_group_views(&self, adapter_id: u8) -> Vec<GroupView>;
    fn group_membership_matrix_view(&self, adapter_id: u8) -> Option<GroupMembershipMatrixView>;
    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot>;

    fn applied_group_member_mask(&self, adapter_id: u8, group_id: u8) -> Option<u64> {
        if group_id >= GROUP_COUNT {
            return None;
        }
        let bit = 1u16 << group_id;
        let snapshot = self.group_apply_snapshot(adapter_id)?;
        Some(snapshot.rows.iter().fold(0u64, |mask, row| {
            if row.virtual_lamp_id < VIRTUAL_LAMP_COUNT && row.applied_groups_mask & bit != 0 {
                mask | (1u64 << row.virtual_lamp_id)
            } else {
                mask
            }
        }))
    }
}

pub trait OperationReadPort: Send + Sync {
    fn operation_status(&self, operation_id: &str) -> Option<Cow<'static, str>>;
    fn operation_view(&self, operation_id: &str) -> Option<OperationView>;
    fn list_operation_keys(&self) -> Vec<String>;
    fn has_active_operation(&self, operation_type: OperationType, adapter_id: u8) -> bool;
}

pub trait AdapterApplyWatchPort: Send + Sync {
    fn adapter_settings_applied_load(&self) -> u32;
}

pub trait PollerSettingsApplyWatchPort: Send + Sync {
    fn poller_settings_applied_load(&self) -> u32;
}

pub trait DaliSettingsApplyWatchPort: Send + Sync {
    fn dali_settings_applied_load(&self) -> u32;
}

pub trait HomeAssistantSettingsApplyWatchPort: Send + Sync {
    fn home_assistant_settings_applied_load(&self) -> u32;
}

pub trait PhysicalDeviceApplyWatchPort: Send + Sync {
    fn physical_override_applied_load(&self) -> u32;
}

pub trait VirtualLampMetadataWatchPort: Send + Sync {
    fn virtual_lamp_metadata_applied_load(&self) -> u32;
}

pub trait VirtualLampBindingWatchPort: Send + Sync {
    fn virtual_lamp_binding_applied_load(&self) -> u32;
}

pub trait GroupMetadataWatchPort: Send + Sync {
    fn group_metadata_applied_load(&self) -> u32;
}

pub trait SceneReadPort: AdapterReadPort {
    fn scene_view(&self, adapter_id: u8, scene_id: u8) -> Option<SceneView>;
    fn list_scene_views(&self, adapter_id: u8) -> Vec<SceneView>;
    fn scene_matrix_view(&self, adapter_id: u8, scene_id: u8) -> Option<SceneMatrixView>;
    fn scene_apply_snapshot(&self, adapter_id: u8, scene_id: u8) -> Option<SceneApplySnapshot>;
}

pub trait ApplyReadPort: GroupReadPort + SceneReadPort + PolicyApplyReadPort {}
impl<T: GroupReadPort + SceneReadPort + PolicyApplyReadPort> ApplyReadPort for T {}

pub trait PolicyApplyReadPort: Send + Sync {
    fn policy_apply_targets(&self, adapter_id: u8) -> Option<Vec<PolicyApplyCell>>;
}

pub trait ProjectorReadPort:
    VirtualLampCapabilityReadPort + GroupReadPort + SceneReadPort
{
}
impl<T: VirtualLampCapabilityReadPort + GroupReadPort + SceneReadPort> ProjectorReadPort for T {}

pub trait InputInstanceTypeReadPort: Send + Sync {
    fn input_instance_type(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> Option<u8>;
}

pub trait SceneMetadataWatchPort: Send + Sync {
    fn scene_metadata_applied_load(&self) -> u32;
}
