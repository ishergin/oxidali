use std::collections::BTreeMap;

use dali2rust_contracts::msg::{
    PoliciesUpdateCommand, PolicyException, PolicyExceptionList, MAX_POLICY_EXCEPTIONS,
};
use dali2rust_domain::registry::{GearPolicyLevels, PoliciesReadPort, PoliciesView};
use dali2rust_bsp::psram::PsramBox;

use super::persistence_slices::PersistablePoliciesSlice;
use super::physical_devices::PhysicalDeviceRecord;
use super::replaced_slices::ReplacedEntities;
use super::store::{Inner, RegistryStore, MAX_SHORT_ADDRESSES};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PoliciesRecord {
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub apply_on_discovery: bool,
    pub exceptions: PolicyExceptionList,
}

fn managed(level: u8) -> Option<u8> {
    (level != PoliciesUpdateCommand::UNMANAGED).then_some(level)
}

fn exception(adapter_id: u8, short_address: u8) -> PolicyException {
    PolicyException { adapter_id, short_address }
}

fn sorted_unique(entries: impl IntoIterator<Item = PolicyException>) -> PolicyExceptionList {
    let mut all: Vec<PolicyException> = entries.into_iter().collect();
    all.sort_unstable();
    all.dedup();
    all.into_iter().take(MAX_POLICY_EXCEPTIONS).collect()
}

pub(crate) fn held_levels(record: &PhysicalDeviceRecord) -> GearPolicyLevels {
    let common = &record.attributes.common102;
    GearPolicyLevels {
        system_failure_level: common.system_failure_level.as_ref().map(|o| o.value),
        power_on_level: common.power_on_level.as_ref().map(|o| o.value),
    }
}

impl PoliciesRecord {
    pub(crate) fn to_view(&self) -> PoliciesView {
        PoliciesView {
            system_failure_level: self.system_failure_level,
            power_on_level: self.power_on_level,
            apply_on_discovery: self.apply_on_discovery,
            exceptions: self.exceptions.clone(),
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
        if body.patch_mask & PoliciesUpdateCommand::PATCH_EXCEPTIONS != 0 {
            self.exceptions = sorted_unique(body.exceptions.iter().copied());
        }
    }

    pub(crate) fn excepts(&self, adapter_id: u8, short_address: u8) -> bool {
        self.exceptions.contains(&exception(adapter_id, short_address))
    }

    pub(crate) fn drop_exception(&mut self, adapter_id: u8, short_address: u8) -> bool {
        let before = self.exceptions.len();
        self.exceptions.retain(|entry| *entry != exception(adapter_id, short_address));
        self.exceptions.len() != before
    }

    pub(crate) fn move_exception(&mut self, adapter_id: u8, from: u8, to: u8) -> bool {
        if !self.drop_exception(adapter_id, from) {
            return false;
        }
        let kept = self.exceptions.iter().copied();
        self.exceptions = sorted_unique(kept.chain([exception(adapter_id, to)]));
        true
    }
}

fn drift_flags(inner: &Inner) -> BTreeMap<(u8, u8), Option<bool>> {
    let policies = inner.policies.to_view();
    let flag = |(&(adapter_id, short), record): (&(u8, u8), &PsramBox<PhysicalDeviceRecord>)| {
        let held = held_levels(record);
        ((adapter_id, short), policies.mismatch(exception(adapter_id, short), held))
    };
    inner.physical_devices.iter().map(flag).collect()
}

pub(crate) fn device_policy_mismatch(
    inner: &Inner,
    adapter_id: u8,
    short_address: u8,
    record: &PhysicalDeviceRecord,
) -> Option<bool> {
    let held = held_levels(record);
    inner.policies.to_view().mismatch(exception(adapter_id, short_address), held)
}

impl RegistryStore {
    pub(crate) fn names_unknown_new_exception(&self, body: &PoliciesUpdateCommand) -> bool {
        if body.patch_mask & PoliciesUpdateCommand::PATCH_EXCEPTIONS == 0 {
            return false;
        }
        let g = self.read_inner();
        let unknown = |entry: &PolicyException| {
            !g.policies.exceptions.contains(entry)
                && !g.physical_devices.contains_key(&(entry.adapter_id, entry.short_address))
        };
        body.exceptions.iter().any(unknown)
    }

    pub(crate) fn apply_policies_from_command(
        &self,
        body: &PoliciesUpdateCommand,
    ) -> PoliciesRecord {
        let mut g = self.write_inner();
        let before = drift_flags(&g);
        g.policies.apply_patch(body);
        let after = drift_flags(&g);
        let applied = g.policies.clone();
        drop(g);
        self.dirty.mark_policies_dirty();
        let moved = after.into_iter().filter(|(key, flag)| before.get(key) != Some(flag));
        self.queue_announcements(ReplacedEntities::devices(moved.map(|(key, _)| key)));
        applied
    }

    pub(crate) fn note_exceptions_changed(&self) {
        self.dirty.mark_policies_dirty();
        self.queue_announcements(ReplacedEntities::policies());
    }

    pub(crate) fn policies_row(&self) -> PoliciesRecord {
        self.read_inner().policies.clone()
    }
}

impl PoliciesReadPort for RegistryStore {
    fn policies_view(&self) -> PoliciesView {
        self.read_inner().policies.to_view()
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistablePoliciesSlice {
    PersistablePoliciesSlice {
        system_failure_level: inner.policies.system_failure_level,
        power_on_level: inner.policies.power_on_level,
        apply_on_discovery: inner.policies.apply_on_discovery,
        exceptions: inner.policies.exceptions.to_vec(),
    }
}

pub(crate) fn hydrate_policies_inner(inner: &mut Inner, slice: &PersistablePoliciesSlice) {
    let addressable =
        |entry: &&PolicyException| usize::from(entry.short_address) < MAX_SHORT_ADDRESSES;
    inner.policies = PoliciesRecord {
        system_failure_level: slice.system_failure_level.and_then(managed),
        power_on_level: slice.power_on_level.and_then(managed),
        apply_on_discovery: slice.apply_on_discovery,
        exceptions: sorted_unique(slice.exceptions.iter().filter(addressable).copied()),
    };
}

impl dali2rust_domain::registry::PolicyApplyReadPort for RegistryStore {
    fn policy_apply_targets(
        &self,
        adapter_id: u8,
    ) -> Option<Vec<dali2rust_domain::registry::PolicyApplyCell>> {
        let policy = self.policies_row();
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
                    known_device_types: <Self as dali2rust_domain::registry::PhysicalDeviceReadPort>::
                        physical_device_known_types(self, adapter_id, short_address),
                    excepted: policy.excepts(adapter_id, short_address),
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
            exceptions: PolicyExceptionList::new(),
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
                exceptions: Vec::new(),
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

#[cfg(test)]
mod exception_tests {
    use super::*;
    use crate::runtime::registry::physical_device_banks::short_bit;
    use crate::runtime::registry::physical_devices::DISCOVERY_EVICT_MISS_THRESHOLD;
    use dali2rust_domain::registry::{PhysicalDeviceReadPort, PolicyApplyReadPort};

    const VERIFIED: u32 = 0x00AB_CDEF;
    const MANAGED_LEVEL: u8 = 100;
    const DRIFTED_LEVEL: u8 = 50;

    fn at(short_address: u8) -> PolicyException {
        exception(0, short_address)
    }

    fn excepting(shorts: &[u8]) -> PoliciesUpdateCommand {
        PoliciesUpdateCommand {
            patch_mask: PoliciesUpdateCommand::PATCH_EXCEPTIONS,
            system_failure_level: PoliciesUpdateCommand::UNMANAGED,
            power_on_level: PoliciesUpdateCommand::UNMANAGED,
            apply_on_discovery: false,
            exceptions: shorts.iter().map(|short| at(*short)).collect(),
        }
    }

    fn managing_power_on(level: u8) -> PoliciesUpdateCommand {
        PoliciesUpdateCommand {
            patch_mask: PoliciesUpdateCommand::PATCH_POWER_ON_LEVEL,
            power_on_level: level,
            ..excepting(&[])
        }
    }

    fn store_with(shorts: &[u8], random_address: Option<u32>) -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        for short in shorts {
            assert!(store.seed_discovered_dt8(*short, random_address));
        }
        store
    }

    fn stored(store: &RegistryStore) -> Vec<u8> {
        store.policies_row().exceptions.iter().map(|entry| entry.short_address).collect()
    }

    fn holds_power_on(store: &RegistryStore, short: u8, level: u8) {
        let (mirek, levels) = ((None, None), (None, None));
        let po = Some(level);
        assert!(store.apply_attributes_written(0, short, None, None, po, None, None, mirek, levels, None));
    }

    fn mismatch(store: &RegistryStore, short: u8) -> Option<bool> {
        store.physical_device_core_view(0, short).expect("record").policy_mismatch
    }

    #[test]
    fn an_exception_list_is_stored_sorted_and_without_duplicates() {
        let store = store_with(&[2, 5], Some(VERIFIED));
        store.apply_policies_from_command(&excepting(&[5, 2, 5]));
        assert_eq!(stored(&store), vec![2, 5]);
        store.apply_policies_from_command(&managing_power_on(MANAGED_LEVEL));
        assert_eq!(stored(&store), vec![2, 5], "a patch without the exceptions bit keeps the list");
    }

    #[test]
    fn only_a_new_entry_naming_no_record_is_unknown() {
        let store = store_with(&[2, 5], Some(VERIFIED));
        assert!(store.names_unknown_new_exception(&excepting(&[2, 9])));
        assert!(!store.names_unknown_new_exception(&excepting(&[2, 5])));
        assert!(!store.names_unknown_new_exception(&PoliciesUpdateCommand {
            patch_mask: PoliciesUpdateCommand::PATCH_POWER_ON_LEVEL,
            ..excepting(&[9])
        }), "entries outside the patch mask are not read");
        let mut g = store.write_inner();
        g.policies.exceptions.push(at(9)).expect("capacity");
        drop(g);
        assert!(!store.names_unknown_new_exception(&excepting(&[2, 9])), "a stored entry may be resent");
    }

    #[test]
    fn an_exception_follows_its_record_to_a_new_address() {
        let store = store_with(&[2, 5], Some(VERIFIED));
        store.apply_policies_from_command(&excepting(&[2]));
        store.apply_address_change(0, 2, 7).expect("moved");
        assert_eq!(stored(&store), vec![7]);
        store.apply_address_change(0, 5, 8).expect("moved");
        assert_eq!(stored(&store), vec![7], "a record without an exception moves none");
    }

    #[test]
    fn a_leftover_entry_excepts_whatever_record_comes_to_hold_its_address() {
        let store = store_with(&[2, 5], Some(VERIFIED));
        store.apply_policies_from_command(&managing_power_on(MANAGED_LEVEL));
        store.write_inner().policies.exceptions.push(at(9)).expect("capacity");
        store.apply_address_change(0, 5, 9).expect("moved");
        assert_eq!(stored(&store), vec![9], "the entry reserves the address, not a device");
        let targets = store.policy_apply_targets(0).expect("adapter");
        let marked: Vec<(u8, bool)> = targets.iter().map(|c| (c.short_address, c.excepted)).collect();
        assert_eq!(marked, vec![(2, false), (9, true)]);
    }

    #[test]
    fn forgetting_or_evicting_a_record_drops_its_exception() {
        let store = store_with(&[2, 5], Some(VERIFIED));
        assert!(store.seed_discovered_dt8(9, None));
        store.apply_policies_from_command(&excepting(&[2, 5, 9]));
        store.apply_physical_device_forget(0, 2).expect("forgotten");
        assert_eq!(stored(&store), vec![5, 9]);
        let confirmed = short_bit(5);
        (0..DISCOVERY_EVICT_MISS_THRESHOLD).for_each(|_| {
            store.reconcile_discovery_scan(0, confirmed);
        });
        assert_eq!(stored(&store), vec![5]);
    }

    #[test]
    fn a_replacement_keeps_the_role_exception_or_brings_its_own() {
        let store = store_with(&[2, 5], Some(VERIFIED));
        store.apply_policies_from_command(&excepting(&[2, 5]));
        store.apply_device_replacement(0, (2, 5), false).expect("replaced");
        assert_eq!(stored(&store), vec![2], "the role keeps its exception, the old address none");
        let store = store_with(&[2, 5], Some(VERIFIED));
        store.apply_policies_from_command(&excepting(&[5]));
        store.apply_device_replacement(0, (2, 5), false).expect("replaced");
        assert_eq!(stored(&store), vec![2], "the replacement brings its exception to the role");
        let store = store_with(&[2, 5], Some(VERIFIED));
        store.apply_device_replacement(0, (2, 5), false).expect("replaced");
        assert!(stored(&store).is_empty());
    }

    #[test]
    fn an_excepted_device_is_marked_in_the_apply_targets() {
        let store = store_with(&[2, 5], Some(VERIFIED));
        store.apply_policies_from_command(&managing_power_on(MANAGED_LEVEL));
        store.apply_policies_from_command(&excepting(&[5]));
        let targets = store.policy_apply_targets(0).expect("adapter");
        let marked: Vec<(u8, bool)> = targets.iter().map(|c| (c.short_address, c.excepted)).collect();
        assert_eq!(marked, vec![(2, false), (5, true)]);
    }

    #[test]
    fn hydration_keeps_only_addressable_unique_sorted_entries() {
        let store = RegistryStore::with_adapter_count(1);
        let mut inner = store.write_inner();
        let slice = PersistablePoliciesSlice {
            system_failure_level: None,
            power_on_level: None,
            apply_on_discovery: false,
            exceptions: vec![at(9), at(64), at(2), at(9)],
        };
        hydrate_policies_inner(&mut inner, &slice);
        let shorts: Vec<u8> = inner.policies.exceptions.iter().map(|e| e.short_address).collect();
        assert_eq!(shorts, vec![2, 9]);
        assert_eq!(persistable_snapshot(&inner).exceptions, vec![at(2), at(9)]);
    }

    #[test]
    fn the_drift_flag_compares_the_held_level_with_the_policy() {
        let store = store_with(&[2], Some(VERIFIED));
        holds_power_on(&store, 2, DRIFTED_LEVEL);
        assert_eq!(mismatch(&store, 2), None, "nothing is stated while the policy manages nothing");
        store.apply_policies_from_command(&managing_power_on(MANAGED_LEVEL));
        assert_eq!(mismatch(&store, 2), Some(true));
        holds_power_on(&store, 2, MANAGED_LEVEL);
        assert_eq!(mismatch(&store, 2), Some(false), "a confirmed write that matches clears it");
        holds_power_on(&store, 2, DRIFTED_LEVEL);
        store.apply_policies_from_command(&excepting(&[2]));
        assert_eq!(mismatch(&store, 2), None, "an excepted device is never compared");
    }

    #[test]
    fn a_policy_change_announces_only_the_devices_whose_flag_moved() {
        let store = store_with(&[2, 5], Some(VERIFIED));
        holds_power_on(&store, 2, DRIFTED_LEVEL);
        store.apply_policies_from_command(&managing_power_on(MANAGED_LEVEL));
        assert_eq!(store.announcements_queued(), 1, "device 5 was never read and states nothing");
        store.apply_policies_from_command(&managing_power_on(MANAGED_LEVEL));
        assert_eq!(store.announcements_queued(), 1, "an unchanged flag announces nothing new");
    }

    #[test]
    fn a_moved_exception_announces_the_policies() {
        let store = store_with(&[2], Some(VERIFIED));
        store.apply_policies_from_command(&excepting(&[2]));
        assert_eq!(store.announcements_queued(), 0);
        store.apply_address_change(0, 2, 7).expect("moved");
        assert_eq!(store.announcements_queued(), 1);
    }
}
