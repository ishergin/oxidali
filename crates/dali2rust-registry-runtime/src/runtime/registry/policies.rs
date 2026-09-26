use dali2rust_contracts::msg::PoliciesUpdateCommand;
use dali2rust_domain::registry::{PoliciesReadPort, PoliciesView};

use super::persistence_slices::PersistablePoliciesSlice;
use super::store::{Inner, RegistryStore};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PoliciesRecord {
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub apply_on_discovery: bool,
}

fn managed(level: u8) -> Option<u8> {
    (level != PoliciesUpdateCommand::UNMANAGED).then_some(level)
}

impl PoliciesRecord {
    fn to_view(self) -> PoliciesView {
        PoliciesView {
            system_failure_level: self.system_failure_level,
            power_on_level: self.power_on_level,
            apply_on_discovery: self.apply_on_discovery,
        }
    }

    fn apply_patch(&mut self, body: &PoliciesUpdateCommand) {
        if body.patch_mask & PoliciesUpdateCommand::PATCH_SYSTEM_FAILURE_LEVEL != 0 {
            self.system_failure_level = managed(body.system_failure_level);
        }
        if body.patch_mask & PoliciesUpdateCommand::PATCH_POWER_ON_LEVEL != 0 {
            self.power_on_level = managed(body.power_on_level);
        }
        if body.patch_mask & PoliciesUpdateCommand::PATCH_APPLY_ON_DISCOVERY != 0 {
            self.apply_on_discovery = body.apply_on_discovery;
        }
    }
}

impl RegistryStore {
    pub(crate) fn apply_policies_from_command(
        &self,
        body: &PoliciesUpdateCommand,
    ) -> PoliciesRecord {
        let mut g = self.write_inner();
        g.policies.apply_patch(body);
        let applied = g.policies;
        drop(g);
        self.dirty.mark_policies_dirty();
        applied
    }

    pub(crate) fn policies_row(&self) -> PoliciesRecord {
        self.read_inner().policies
    }
}

impl PoliciesReadPort for RegistryStore {
    fn policies_view(&self) -> PoliciesView {
        self.policies_row().to_view()
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistablePoliciesSlice {
    PersistablePoliciesSlice {
        system_failure_level: inner.policies.system_failure_level,
        power_on_level: inner.policies.power_on_level,
        apply_on_discovery: inner.policies.apply_on_discovery,
    }
}

pub(crate) fn hydrate_policies_inner(inner: &mut Inner, slice: &PersistablePoliciesSlice) {
    inner.policies = PoliciesRecord {
        system_failure_level: slice.system_failure_level.and_then(managed),
        power_on_level: slice.power_on_level.and_then(managed),
        apply_on_discovery: slice.apply_on_discovery,
    };
}

impl dali2rust_domain::registry::PolicyApplyReadPort for RegistryStore {
    fn policy_apply_targets(
        &self,
        adapter_id: u8,
    ) -> Option<Vec<dali2rust_domain::registry::PolicyApplyCell>> {
        let g = self.read_inner();
        let policy = g.policies;
        drop(g);
        if !policy.to_view().manages_anything() {
            return Some(Vec::new());
        }
        let shorts = <Self as dali2rust_domain::registry::PhysicalDeviceReadPort>::
            list_physical_device_short_addresses(self, adapter_id);
        Some(
            shorts
                .into_iter()
                .map(|short_address| dali2rust_domain::registry::PolicyApplyCell {
                    short_address,
                    system_failure_level: policy.system_failure_level,
                    power_on_level: policy.power_on_level,
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(mask: u8, failure: u8, power_on: u8, on_discovery: bool) -> PoliciesUpdateCommand {
        PoliciesUpdateCommand {
            patch_mask: mask,
            system_failure_level: failure,
            power_on_level: power_on,
            apply_on_discovery: on_discovery,
        }
    }

    #[test]
    fn nothing_is_managed_until_an_operator_says_so() {
        let d = PoliciesRecord::default();
        assert_eq!(d.system_failure_level, None);
        assert_eq!(d.power_on_level, None);
        assert!(!d.apply_on_discovery);
    }

    #[test]
    fn a_level_is_stored_and_the_unmanaged_byte_clears_it() {
        let store = RegistryStore::with_adapter_count(1);
        let set = store.apply_policies_from_command(&cmd(
            PoliciesUpdateCommand::PATCH_SYSTEM_FAILURE_LEVEL,
            10,
            0,
            false,
        ));
        assert_eq!(set.system_failure_level, Some(10));
        let cleared = store.apply_policies_from_command(&cmd(
            PoliciesUpdateCommand::PATCH_SYSTEM_FAILURE_LEVEL,
            PoliciesUpdateCommand::UNMANAGED,
            0,
            false,
        ));
        assert_eq!(cleared.system_failure_level, None);
    }

    #[test]
    fn a_mask_read_back_from_another_builds_slice_is_not_obeyed() {
        let store = RegistryStore::with_adapter_count(1);
        let mut inner = store.write_inner();
        hydrate_policies_inner(
            &mut inner,
            &PersistablePoliciesSlice {
                system_failure_level: Some(255),
                power_on_level: Some(200),
                apply_on_discovery: true,
            },
        );
        assert_eq!(inner.policies.system_failure_level, None);
        assert_eq!(inner.policies.power_on_level, Some(200));
        assert!(inner.policies.apply_on_discovery);
    }

    #[test]
    fn a_policy_with_no_level_manages_nothing() {
        assert!(!PoliciesRecord::default().to_view().manages_anything());
        let mut record = PoliciesRecord {
            apply_on_discovery: true,
            ..Default::default()
        };
        assert!(
            !record.to_view().manages_anything(),
            "a discovery flag is not a level"
        );
        record.power_on_level = Some(0);
        assert!(record.to_view().manages_anything());
    }
}
