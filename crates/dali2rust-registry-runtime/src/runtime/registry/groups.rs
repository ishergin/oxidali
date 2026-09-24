use crate::runtime::registry::physical_devices::write_groups_membership_read;
use crate::runtime::registry::store::{Inner, RegistryStore};
use dali2rust_contracts::msg::{fixed_text_64, GroupMatrixDesiredRow, FixedText64};
use dali2rust_domain::registry::{
    CapabilityFlagsView, GroupApplyRowView, GroupApplySnapshot, GroupMatrixGroupView,
    GroupMembershipMatrixRowView, GroupMembershipMatrixView, GroupReadPort, GroupView,
};

pub(crate) use dali2rust_domain::registry::{GROUP_COUNT, VIRTUAL_LAMP_COUNT};

#[derive(Clone, Debug)]
pub(crate) struct GroupRecord {
    pub name: FixedText64,
    pub ha_entity_enabled: bool,
}

impl Default for GroupRecord {
    fn default() -> Self {
        Self {
            name: FixedText64::new(),
            ha_entity_enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GroupMembershipRowRecord {
    pub desired_groups_mask: u16,
    pub desired_seeded: bool,
    pub desired_from_operator: bool,
}

fn mask_bit(group_id: u8) -> u16 {
    1u16 << group_id
}

fn mask_has_group(mask: u16, group_id: u8) -> bool {
    mask & mask_bit(group_id) != 0
}

fn mask_to_bools(mask: u16) -> [bool; 16] {
    std::array::from_fn(|idx| mask & (1u16 << idx) != 0)
}

fn adapter_exists(inner: &Inner, adapter_id: u8) -> bool {
    inner.adapter_exists(adapter_id)
}

fn union_capabilities(out: &mut CapabilityFlagsView, caps: &CapabilityFlagsView) {
    out.brightness |= caps.brightness;
    out.cct |= caps.cct;
    out.xy |= caps.xy;
    out.rgb |= caps.rgb;
    out.rgbwaf |= caps.rgbwaf;
    out.scenes |= caps.scenes;
    out.groups |= caps.groups;
}

fn group_row(inner: &Inner, adapter_id: u8, virtual_lamp_id: u8) -> GroupMembershipRowRecord {
    inner
        .group_matrix
        .get(&(adapter_id, virtual_lamp_id))
        .copied()
        .unwrap_or_default()
}

fn derived_applied_mask(inner: &Inner, adapter_id: u8, virtual_lamp_id: u8) -> u16 {
    let binding = inner
        .lamps
        .get(&(adapter_id, virtual_lamp_id))
        .and_then(|lamp| lamp.binding_short);
    let Some(short) = binding else {
        return 0;
    };
    inner
        .physical_devices
        .get(&(adapter_id, short))
        .and_then(|pd| pd.attributes.groups.membership.as_ref())
        .map(|obs| obs.value)
        .unwrap_or(0)
}

pub(crate) fn group_record(inner: &Inner, adapter_id: u8, group_id: u8) -> GroupRecord {
    inner
        .groups
        .get(&(adapter_id, group_id))
        .cloned()
        .unwrap_or_default()
}

fn group_member_capabilities(inner: &Inner, adapter_id: u8, virtual_lamp_id: u8) -> CapabilityFlagsView {
    inner.vl_capabilities(adapter_id, virtual_lamp_id)
}

fn group_dirty(inner: &Inner, adapter_id: u8, group_id: u8) -> bool {
    (0..VIRTUAL_LAMP_COUNT).any(|virtual_lamp_id| {
        let row = group_row(inner, adapter_id, virtual_lamp_id);
        let applied = derived_applied_mask(inner, adapter_id, virtual_lamp_id);
        mask_has_group(row.desired_groups_mask, group_id)
            != mask_has_group(applied, group_id)
    })
}

pub(crate) fn group_members(inner: &Inner, adapter_id: u8, group_id: u8) -> Vec<u8> {
    (0..VIRTUAL_LAMP_COUNT)
        .filter(|vlid| mask_has_group(group_row(inner, adapter_id, *vlid).desired_groups_mask, group_id))
        .collect()
}

pub(crate) fn group_members_applied(inner: &Inner, adapter_id: u8, group_id: u8) -> Vec<u8> {
    (0..VIRTUAL_LAMP_COUNT)
        .filter(|vlid| mask_has_group(derived_applied_mask(inner, adapter_id, *vlid), group_id))
        .collect()
}

impl RegistryStore {
    pub(crate) fn note_group_commanded(&self, adapter_id: u8, group_id: u8, on: bool) {
        let mut inner = self.write_inner();
        if on {
            inner.group_commanded.insert((adapter_id, group_id));
        } else {
            inner.group_commanded.remove(&(adapter_id, group_id));
        }
    }

    pub(crate) fn note_broadcast_commanded(&self, adapter_id: u8, on: bool) {
        let mut inner = self.write_inner();
        for group_id in 0..GROUP_COUNT {
            if on {
                inner.group_commanded.insert((adapter_id, group_id));
            } else {
                inner.group_commanded.remove(&(adapter_id, group_id));
            }
        }
    }
}

fn group_counts(inner: &Inner, adapter_id: u8, group_id: u8) -> (u8, u8) {
    let mut desired = 0u8;
    let mut applied = 0u8;
    for virtual_lamp_id in 0..VIRTUAL_LAMP_COUNT {
        let row = group_row(inner, adapter_id, virtual_lamp_id);
        let applied_mask = derived_applied_mask(inner, adapter_id, virtual_lamp_id);
        desired += u8::from(mask_has_group(row.desired_groups_mask, group_id));
        applied += u8::from(mask_has_group(applied_mask, group_id));
    }
    (desired, applied)
}

pub(crate) fn group_capabilities(inner: &Inner, adapter_id: u8, group_id: u8) -> CapabilityFlagsView {
    let mut caps = CapabilityFlagsView::default();
    for virtual_lamp_id in 0..VIRTUAL_LAMP_COUNT {
        let row = group_row(inner, adapter_id, virtual_lamp_id);
        if mask_has_group(row.desired_groups_mask, group_id) {
            union_capabilities(
                &mut caps,
                &group_member_capabilities(inner, adapter_id, virtual_lamp_id),
            );
        }
    }
    caps
}

fn build_group_view(inner: &Inner, adapter_id: u8, group_id: u8) -> GroupView {
    let record = group_record(inner, adapter_id, group_id);
    let (member_count_desired, member_count_applied) = group_counts(inner, adapter_id, group_id);
    GroupView {
        adapter_id,
        group_id,
        name: record.name.as_str().to_string(),
        ha_entity_enabled: record.ha_entity_enabled,
        capabilities_summary: group_capabilities(inner, adapter_id, group_id),
        dirty: group_dirty(inner, adapter_id, group_id),
        member_count_desired,
        member_count_applied,
    }
}

fn seed_desired_for_short(inner: &mut Inner, adapter_id: u8, short_address: u8, membership: u16) -> bool {
    let mut changed = false;
    for virtual_lamp_id in 0..VIRTUAL_LAMP_COUNT {
        let binding = inner
            .lamps
            .get(&(adapter_id, virtual_lamp_id))
            .and_then(|lamp| lamp.binding_short);
        if binding != Some(short_address) {
            continue;
        }
        let entry = inner.group_matrix.entry((adapter_id, virtual_lamp_id)).or_default();
        if entry.desired_seeded {
            continue;
        }
        entry.desired_groups_mask = membership;
        entry.desired_seeded = true;
        entry.desired_from_operator = false;
        changed = true;
    }
    changed
}

pub(super) fn seed_desired_for_new_binding(
    inner: &mut Inner,
    adapter_id: u8,
    virtual_lamp_id: u8,
    short_address: u8,
) -> bool {
    let Some(membership) = inner
        .physical_devices
        .get(&(adapter_id, short_address))
        .and_then(|pd| pd.attributes.groups.membership.as_ref())
        .map(|obs| obs.value)
    else {
        return false;
    };
    let entry = inner
        .group_matrix
        .entry((adapter_id, virtual_lamp_id))
        .or_default();
    if entry.desired_seeded {
        return false;
    }
    entry.desired_groups_mask = membership;
    entry.desired_seeded = true;
    entry.desired_from_operator = false;
    true
}

pub(super) fn forget_adopted_desired_on_binding_change(
    inner: &mut Inner,
    adapter_id: u8,
    virtual_lamp_id: u8,
) -> bool {
    let Some(entry) = inner.group_matrix.get_mut(&(adapter_id, virtual_lamp_id)) else {
        return false;
    };
    if !entry.desired_seeded || entry.desired_from_operator {
        return false;
    }
    entry.desired_seeded = false;
    entry.desired_groups_mask = 0;
    true
}

fn apply_group_rows(inner: &mut Inner, adapter_id: u8, rows: &[GroupMatrixDesiredRow]) -> bool {
    let mut changed = false;
    for row in rows {
        let entry = inner
            .group_matrix
            .entry((adapter_id, row.virtual_lamp_id))
            .or_default();
        if entry.desired_groups_mask != row.desired_groups_mask {
            entry.desired_groups_mask = row.desired_groups_mask;
            changed = true;
        }
        if !entry.desired_seeded {
            entry.desired_seeded = true;
            changed = true;
        }
        if !entry.desired_from_operator {
            entry.desired_from_operator = true;
            changed = true;
        }
    }
    changed
}

impl RegistryStore {
    pub(crate) fn apply_group_metadata_patch(
        &self,
        adapter_id: u8,
        group_id: u8,
        patch_mask: u8,
        name: Option<&str>,
        ha_entity_enabled: bool,
    ) -> bool {
        use dali2rust_contracts::msg::GroupMetadataUpdateCommand;

        let mut inner = self.write_inner();
        if !adapter_exists(&inner, adapter_id) || group_id >= GROUP_COUNT {
            return false;
        }
        let entry = inner.groups.entry((adapter_id, group_id)).or_default();
        let mut changed = false;
        if patch_mask & GroupMetadataUpdateCommand::PATCH_NAME != 0 {
            let next = fixed_text_64(name.unwrap_or(""));
            if entry.name != next {
                entry.name = next;
                changed = true;
            }
        }
        if patch_mask & GroupMetadataUpdateCommand::PATCH_HA_ENTITY_ENABLED != 0 && entry.ha_entity_enabled != ha_entity_enabled
        {
            entry.ha_entity_enabled = ha_entity_enabled;
            changed = true;
        }
        if changed {
            inner.groups_metadata_revision = inner.groups_metadata_revision.wrapping_add(1);
        }
        drop(inner);
        if changed {
            self.dirty.mark_groups_dirty(adapter_id);
        }
        changed
    }

    pub(crate) fn apply_group_matrix_rows(&self, adapter_id: u8, rows: &[GroupMatrixDesiredRow]) -> bool {
        let mut inner = self.write_inner();
        if !adapter_exists(&inner, adapter_id) {
            return false;
        }
        let changed = apply_group_rows(&mut inner, adapter_id, rows);
        if changed {
            inner.group_matrix_revision = inner.group_matrix_revision.wrapping_add(1);
        }
        drop(inner);
        if changed {
            self.dirty.mark_groups_dirty(adapter_id);
        }
        changed
    }

    pub(crate) fn apply_group_membership_readback(
        &self,
        adapter_id: u8,
        short_address: u8,
        membership: u16,
    ) -> bool {
        let mut inner = self.write_inner();
        if !adapter_exists(&inner, adapter_id) {
            return false;
        }
        let Some(record) = inner.physical_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        let now = crate::runtime::registry::store::registry_unix_ms();
        let membership_changed = write_groups_membership_read(&mut record.attributes, membership, now);
        let seeded = seed_desired_for_short(&mut inner, adapter_id, short_address, membership);
        let changed = membership_changed || seeded;
        if changed {
            inner.group_matrix_revision = inner.group_matrix_revision.wrapping_add(1);
            inner.physical_devices_revision = inner.physical_devices_revision.saturating_add(1);
        }
        drop(inner);
        if changed {
            self.dirty.mark_groups_dirty(adapter_id);
            self.dirty.mark_physical_devices_dirty(adapter_id);
        }
        changed
    }

    pub(crate) fn seed_group_matrix_from_membership_read(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> bool {
        let mut inner = self.write_inner();
        if !adapter_exists(&inner, adapter_id) {
            return false;
        }
        let membership = inner
            .physical_devices
            .get(&(adapter_id, short_address))
            .and_then(|pd| pd.attributes.groups.membership.as_ref())
            .map(|obs| obs.value)
            .unwrap_or(0);
        let changed = seed_desired_for_short(&mut inner, adapter_id, short_address, membership);
        if changed {
            inner.group_matrix_revision = inner.group_matrix_revision.wrapping_add(1);
        }
        drop(inner);
        if changed {
            self.dirty.mark_groups_dirty(adapter_id);
        }
        changed
    }

    fn group_view_internal(&self, adapter_id: u8, group_id: u8) -> Option<GroupView> {
        let inner = self.read_inner();
        if !adapter_exists(&inner, adapter_id) || group_id >= GROUP_COUNT {
            return None;
        }
        Some(build_group_view(&inner, adapter_id, group_id))
    }

    fn group_matrix_view_internal(&self, adapter_id: u8) -> Option<GroupMembershipMatrixView> {
        let inner = self.read_inner();
        if !adapter_exists(&inner, adapter_id) {
            return None;
        }
        let groups = (0..GROUP_COUNT)
            .map(|group_id| {
                let record = group_record(&inner, adapter_id, group_id);
                GroupMatrixGroupView {
                    group_id,
                    name: record.name.as_str().to_string(),
                    dirty: group_dirty(&inner, adapter_id, group_id),
                }
            })
            .collect();
        let rows = (0..VIRTUAL_LAMP_COUNT)
            .map(|virtual_lamp_id| {
                let row = group_row(&inner, adapter_id, virtual_lamp_id);
                let applied_mask = derived_applied_mask(&inner, adapter_id, virtual_lamp_id);
                let name = inner
                    .lamps
                    .get(&(adapter_id, virtual_lamp_id))
                    .map(|lamp| lamp.name.as_str().to_string())
                    .unwrap_or_default();
                GroupMembershipMatrixRowView {
                    virtual_lamp_id,
                    name,
                    desired: mask_to_bools(row.desired_groups_mask),
                    applied: mask_to_bools(applied_mask),
                }
            })
            .collect();
        let dirty = (0..GROUP_COUNT).any(|group_id| group_dirty(&inner, adapter_id, group_id));
        Some(GroupMembershipMatrixView {
            adapter_id,
            groups,
            rows,
            dirty,
        })
    }

    pub(crate) fn group_apply_snapshot_internal(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
        let inner = self.read_inner();
        if !adapter_exists(&inner, adapter_id) {
            return None;
        }
        let rows = (0..VIRTUAL_LAMP_COUNT)
            .map(|virtual_lamp_id| {
                let row = group_row(&inner, adapter_id, virtual_lamp_id);
                let applied_mask = derived_applied_mask(&inner, adapter_id, virtual_lamp_id);
                let binding_short = inner
                    .lamps
                    .get(&(adapter_id, virtual_lamp_id))
                    .and_then(|lamp| lamp.binding_short);
                GroupApplyRowView {
                    virtual_lamp_id,
                    desired_groups_mask: row.desired_groups_mask,
                    applied_groups_mask: applied_mask,
                    binding_short,
                }
            })
            .collect();
        Some(GroupApplySnapshot { adapter_id, rows })
    }
}

impl GroupReadPort for RegistryStore {
    fn group_view(&self, adapter_id: u8, group_id: u8) -> Option<GroupView> {
        self.group_view_internal(adapter_id, group_id)
    }

    fn list_group_views(&self, adapter_id: u8) -> Vec<GroupView> {
        (0..GROUP_COUNT)
            .filter_map(|group_id| self.group_view_internal(adapter_id, group_id))
            .collect()
    }

    fn group_membership_matrix_view(&self, adapter_id: u8) -> Option<GroupMembershipMatrixView> {
        self.group_matrix_view_internal(adapter_id)
    }

    fn applied_group_member_mask(&self, adapter_id: u8, group_id: u8) -> Option<u64> {
        if group_id >= GROUP_COUNT {
            return None;
        }
        let bit = 1u16 << group_id;
        let inner = self.read_inner();
        if !adapter_exists(&inner, adapter_id) {
            return None;
        }
        let mut mask = 0u64;
        for virtual_lamp_id in 0..VIRTUAL_LAMP_COUNT {
            if derived_applied_mask(&inner, adapter_id, virtual_lamp_id) & bit != 0 {
                mask |= 1u64 << virtual_lamp_id;
            }
        }
        Some(mask)
    }

    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
        self.group_apply_snapshot_internal(adapter_id)
    }
}
