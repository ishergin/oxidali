use dali2rust_contracts::msg::{DaliAttributeGroup, DeviceType, PollerSettingsUpdateCommand};
use dali2rust_domain::dali::banks::{DEVICE_TYPE_DIAGNOSTICS, DEVICE_TYPE_ENERGY};
use dali2rust_domain::registry::{
    PollTargetReadPort, PollTargetSelection, PollTargetView, PollerSettingsReadPort,
    PollerSettingsView,
};

use super::persistence_slices::PersistablePollerSettingsSlice;
use super::store::{Inner, RegistryStore};

const MIN_INTERVAL_MS: u32 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PollerSettingsRecord {
    pub enabled: bool,
    pub interval_ms: u32,
    pub attribute_groups_mask: u8,
    pub include_dt8_color: bool,
    pub include_energy: bool,
    pub include_diagnostics: bool,
    pub skip_unbound_virtual_lamps: bool,
}

impl Default for PollerSettingsRecord {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_ms: 5_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: true,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
        }
    }
}

impl PollerSettingsRecord {
    fn to_view(self) -> PollerSettingsView {
        PollerSettingsView {
            enabled: self.enabled,
            interval_ms: self.interval_ms,
            attribute_groups_mask: self.attribute_groups_mask,
            include_dt8_color: self.include_dt8_color,
            include_energy: self.include_energy,
            include_diagnostics: self.include_diagnostics,
            skip_unbound_virtual_lamps: self.skip_unbound_virtual_lamps,
        }
    }

    fn apply_patch(&mut self, body: &PollerSettingsUpdateCommand) {
        if body.patch_mask & PollerSettingsUpdateCommand::PATCH_ENABLED != 0 {
            self.enabled = body.enabled;
        }
        if body.patch_mask & PollerSettingsUpdateCommand::PATCH_INTERVAL_MS != 0 {
            self.interval_ms = body.interval_ms.max(MIN_INTERVAL_MS);
        }
        if body.patch_mask & PollerSettingsUpdateCommand::PATCH_ATTRIBUTE_GROUPS_MASK != 0 {
            self.attribute_groups_mask = body.attribute_groups_mask;
        }
        if body.patch_mask & PollerSettingsUpdateCommand::PATCH_INCLUDE_DT8_COLOR != 0 {
            self.include_dt8_color = body.include_dt8_color;
        }
        if body.patch_mask & PollerSettingsUpdateCommand::PATCH_SKIP_UNBOUND_VIRTUAL_LAMPS != 0 {
            self.skip_unbound_virtual_lamps = body.skip_unbound_virtual_lamps;
        }
        if body.patch_mask & PollerSettingsUpdateCommand::PATCH_INCLUDE_ENERGY != 0 {
            self.include_energy = body.include_energy;
        }
        if body.patch_mask & PollerSettingsUpdateCommand::PATCH_INCLUDE_DIAGNOSTICS != 0 {
            self.include_diagnostics = body.include_diagnostics;
        }
    }
}

impl RegistryStore {
    pub(crate) fn apply_poller_settings_from_command(
        &self,
        body: &PollerSettingsUpdateCommand,
    ) -> PollerSettingsRecord {
        let mut g = self.write_inner();
        g.poller_settings.apply_patch(body);
        let applied = g.poller_settings;
        drop(g);
        self.dirty.mark_poller_settings_dirty();
        applied
    }

    pub(crate) fn poller_settings_row(&self) -> PollerSettingsRecord {
        self.read_inner().poller_settings
    }
}

impl PollerSettingsReadPort for RegistryStore {
    fn poller_settings_view(&self) -> PollerSettingsView {
        self.poller_settings_row().to_view()
    }
}

impl PollTargetReadPort for RegistryStore {
    fn list_poll_targets(&self, adapter_id: u8) -> PollTargetSelection {
        let g = self.read_inner();
        let skip_unbound = g.poller_settings.skip_unbound_virtual_lamps;
        let bound: u64 = g
            .lamps
            .iter()
            .filter(|((a, _), _)| *a == adapter_id)
            .filter_map(|(_, rec)| rec.binding_short)
            .fold(0u64, |mask, short| mask | (1u64 << short.min(63)));
        let mut excluded_unbound: u16 = 0;
        let mut targets: Vec<PollTargetView> = g
            .physical_devices
            .iter()
            .filter(|((a, _), _)| *a == adapter_id)
            .filter_map(|((_, short_address), rec)| {
                if skip_unbound && bound & (1u64 << (*short_address).min(63)) == 0 {
                    excluded_unbound = excluded_unbound.saturating_add(1);
                    return None;
                }
                let declared = rec.supported_device_types;
                Some(PollTargetView {
                    short_address: *short_address,
                    is_dt8: rec.effective_device_type() == DeviceType::Dt8Color,
                    is_dt6: rec.effective_device_type() == DeviceType::Dt6Led,
                    declares_energy: declared.is_some_and(|set| set.contains(DEVICE_TYPE_ENERGY)),
                    declares_diagnostics: declared
                        .is_some_and(|set| set.contains(DEVICE_TYPE_DIAGNOSTICS)),
                })
            })
            .collect();
        targets.sort_unstable_by_key(|t| t.short_address);
        PollTargetSelection {
            targets,
            excluded_unbound,
        }
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistablePollerSettingsSlice {
    let r = inner.poller_settings;
    PersistablePollerSettingsSlice {
        enabled: r.enabled,
        interval_ms: r.interval_ms,
        attribute_groups_mask: r.attribute_groups_mask,
        include_dt8_color: r.include_dt8_color,
        include_energy: r.include_energy,
        include_diagnostics: r.include_diagnostics,
        skip_unbound_virtual_lamps: r.skip_unbound_virtual_lamps,
    }
}

pub(crate) fn hydrate_poller_settings_inner(inner: &mut Inner, slice: &PersistablePollerSettingsSlice) {
    inner.poller_settings = PollerSettingsRecord {
        enabled: slice.enabled,
        interval_ms: slice.interval_ms.max(MIN_INTERVAL_MS),
        attribute_groups_mask: slice.attribute_groups_mask,
        include_dt8_color: slice.include_dt8_color,
        include_energy: slice.include_energy,
        include_diagnostics: slice.include_diagnostics,
        skip_unbound_virtual_lamps: slice.skip_unbound_virtual_lamps,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval_patch(interval_ms: u32) -> PollerSettingsUpdateCommand {
        PollerSettingsUpdateCommand {
            patch_mask: PollerSettingsUpdateCommand::PATCH_INTERVAL_MS,
            enabled: false,
            interval_ms,
            attribute_groups_mask: 0,
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
        }
    }

    #[test]
    fn a_patch_below_the_rest_floor_is_clamped_not_stored() {
        let mut rec = PollerSettingsRecord::default();
        rec.apply_patch(&interval_patch(0));
        assert_eq!(rec.interval_ms, 200, "clamped to the REST floor");
        rec.apply_patch(&interval_patch(60_000));
        assert_eq!(rec.interval_ms, 60_000, "the floor is a floor, not a reset");
    }

    #[test]
    fn a_stale_slice_below_the_floor_hydrates_clamped() {
        let store = RegistryStore::with_adapter_count(1);
        hydrate_poller_settings_inner(
            &mut store.write_inner(),
            &PersistablePollerSettingsSlice {
                enabled: true,
                interval_ms: 0,
                attribute_groups_mask: 1,
                include_dt8_color: false,
                skip_unbound_virtual_lamps: false,
                include_energy: false,
                include_diagnostics: false,
            },
        );
        let view = store.poller_settings_view();
        assert_eq!(view.interval_ms, 200, "hydration clamps to the same floor");
        assert!(view.enabled, "the clamp touches nothing else");
    }
}
