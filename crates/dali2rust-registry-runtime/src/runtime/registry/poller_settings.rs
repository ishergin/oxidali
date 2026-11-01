use dali2rust_contracts::msg::{DaliAttributeGroup, DeviceType, PollerSettingsUpdateCommand};
use dali2rust_domain::dali::banks::{DEVICE_TYPE_DIAGNOSTICS, DEVICE_TYPE_ENERGY};
use dali2rust_domain::dali::devices::DeviceType as DomainDeviceType;
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
    pub attribute_groups_mask: u16,
    pub include_dt8_color: bool,
    pub include_energy: bool,
    pub include_diagnostics: bool,
    pub skip_unbound_virtual_lamps: bool,
    pub power_cycle_watch: bool,
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
            power_cycle_watch: false,
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
            power_cycle_watch: self.power_cycle_watch,
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
            self.attribute_groups_mask = DaliAttributeGroup::polled_mask(body.attribute_groups_mask);
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
        if body.patch_mask & PollerSettingsUpdateCommand::PATCH_POWER_CYCLE_WATCH != 0 {
            self.power_cycle_watch = body.power_cycle_watch;
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
        let adapter_enabled = g
            .adapters
            .get(usize::from(adapter_id))
            .is_some_and(|adapter| adapter.enabled);
        if !adapter_enabled {
            return PollTargetSelection {
                targets: Vec::new(),
                excluded_unbound: 0,
                adapter_enabled,
            };
        }
        let (targets, excluded_unbound) = poll_targets(&g, adapter_id);
        PollTargetSelection {
            targets,
            excluded_unbound,
            adapter_enabled,
        }
    }
}

fn poll_targets(g: &Inner, adapter_id: u8) -> (Vec<PollTargetView>, u16) {
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
                declares_switching: declared
                    .is_some_and(|set| set.contains(DomainDeviceType::Switching.code())),
                dali_groups: rec.attributes.groups.membership.as_ref().map_or(0, |seen| seen.value),
            })
        })
        .collect();
    targets.sort_unstable_by_key(|t| t.short_address);
    (targets, excluded_unbound)
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
        power_cycle_watch: r.power_cycle_watch,
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
        power_cycle_watch: slice.power_cycle_watch,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::registry::physical_devices::PhysicalDeviceRecord;
    use dali2rust_bsp::psram::PsramBox;
    use dali2rust_contracts::msg::DeviceTypeSet;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct PollerSettingsSliceWithByteMask {
        enabled: bool,
        interval_ms: u32,
        attribute_groups_mask: u8,
        include_dt8_color: bool,
        skip_unbound_virtual_lamps: bool,
        include_energy: bool,
        include_diagnostics: bool,
        power_cycle_watch: bool,
    }

    #[test]
    fn only_gear_that_declares_device_type_7_is_a_switching_target() {
        const SWITCHING_AND_LED: u64 = (1 << 7) | (1 << 6);
        const LED_ONLY: u64 = 1 << 6;
        let store = RegistryStore::with_adapter_count(1);
        let mut g = store.write_inner();
        g.poller_settings.skip_unbound_virtual_lamps = false;
        for (short, declared) in [(1, Some(SWITCHING_AND_LED)), (2, Some(LED_ONLY)), (3, None)] {
            let mut record = PhysicalDeviceRecord::empty(short);
            record.supported_device_types = declared.map(DeviceTypeSet::from_bits);
            g.physical_devices.insert((0, short), PsramBox::new(record));
        }
        drop(g);

        let switching: Vec<(u8, bool)> = store
            .list_poll_targets(0)
            .targets
            .iter()
            .map(|t| (t.short_address, t.declares_switching))
            .collect();
        assert_eq!(switching, [(1, true), (2, false), (3, false)]);
    }

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
            power_cycle_watch: false,
        }
    }

    fn byte_mask_slice(attribute_groups_mask: u8) -> PollerSettingsSliceWithByteMask {
        PollerSettingsSliceWithByteMask {
            enabled: true,
            interval_ms: 5_000,
            attribute_groups_mask,
            include_dt8_color: true,
            skip_unbound_virtual_lamps: false,
            include_energy: true,
            include_diagnostics: false,
            power_cycle_watch: true,
        }
    }

    #[test]
    fn every_mask_the_poller_could_store_in_a_byte_reads_back_unchanged() {
        for byte in 0..=u8::MAX {
            let stored = DaliAttributeGroup::polled_mask(u16::from(byte));
            let narrow = u8::try_from(stored).expect("a polled byte mask fits a byte");
            let bytes = postcard::to_allocvec(&byte_mask_slice(narrow)).expect("encode");
            let slice: PersistablePollerSettingsSlice = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(slice.attribute_groups_mask, stored, "mask {narrow:#04X}");
            assert_eq!(slice.interval_ms, 5_000, "mask {narrow:#04X}");
            assert!(slice.power_cycle_watch && !slice.include_diagnostics, "mask {narrow:#04X}");
        }
    }

    #[test]
    fn a_byte_wide_image_rejects_a_stored_switching_group_instead_of_shifting_the_fields() {
        let mut rec = PollerSettingsRecord::default();
        let groups = DaliAttributeGroup::RuntimeStatus.mask_bit() | DaliAttributeGroup::Dt7Switching.mask_bit();
        rec.apply_patch(&PollerSettingsUpdateCommand {
            patch_mask: PollerSettingsUpdateCommand::PATCH_ATTRIBUTE_GROUPS_MASK,
            attribute_groups_mask: groups,
            ..interval_patch(5_000)
        });
        let store = RegistryStore::with_adapter_count(1);
        store.write_inner().poller_settings = rec;
        let bytes = postcard::to_allocvec(&persistable_snapshot(&store.read_inner())).expect("encode");
        assert!(postcard::from_bytes::<PollerSettingsSliceWithByteMask>(&bytes).is_err());
    }

    #[test]
    fn the_registry_stores_only_groups_the_poller_reads() {
        let mut rec = PollerSettingsRecord::default();
        let runtime = DaliAttributeGroup::RuntimeStatus.mask_bit();
        rec.apply_patch(&PollerSettingsUpdateCommand {
            patch_mask: PollerSettingsUpdateCommand::PATCH_ATTRIBUTE_GROUPS_MASK,
            attribute_groups_mask: runtime | DaliAttributeGroup::SceneColours.mask_bit(),
            ..interval_patch(5_000)
        });
        assert_eq!(rec.attribute_groups_mask, runtime, "scene_colours is never polled");
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
                power_cycle_watch: false,
            },
        );
        let view = store.poller_settings_view();
        assert_eq!(view.interval_ms, 200, "hydration clamps to the same floor");
        assert!(view.enabled, "the clamp touches nothing else");
    }
}
