use dali2rust_contracts::msg::DaliSettingsUpdateCommand;
use dali2rust_domain::registry::{ApplicationActiveMover, DaliSettingsReadPort, DaliSettingsView};

use super::persistence_slices::PersistableDaliSettingsSlice;
use super::store::{Inner, RegistryStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DaliSettingsRecord {
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub application_active: bool,
    pub device_short_address: Option<u8>,
    pub application_active_moved_by: ApplicationActiveMover,
}

impl Default for DaliSettingsRecord {
    fn default() -> Self {
        Self {
            dt8_auto_activation_repair: true,
            dt8_rgbwaf_control_assert: true,
            application_active: true,
            device_short_address: None,
            application_active_moved_by: ApplicationActiveMover::Unmoved,
        }
    }
}

impl DaliSettingsRecord {
    fn to_view(self) -> DaliSettingsView {
        DaliSettingsView {
            dt8_auto_activation_repair: self.dt8_auto_activation_repair,
            dt8_rgbwaf_control_assert: self.dt8_rgbwaf_control_assert,
            application_active: self.application_active,
            device_short_address: self.device_short_address,
        }
    }

    fn apply_patch(&mut self, body: &DaliSettingsUpdateCommand) {
        let was_active = self.application_active;
        if body.patch_mask & DaliSettingsUpdateCommand::PATCH_DT8_AUTO_ACTIVATION_REPAIR != 0 {
            self.dt8_auto_activation_repair = body.dt8_auto_activation_repair;
        }
        if body.patch_mask & DaliSettingsUpdateCommand::PATCH_DT8_RGBWAF_CONTROL_ASSERT != 0 {
            self.dt8_rgbwaf_control_assert = body.dt8_rgbwaf_control_assert;
        }
        if body.patch_mask & DaliSettingsUpdateCommand::PATCH_APPLICATION_ACTIVE != 0 {
            self.application_active = body.application_active;
        }
        if body.patch_mask & DaliSettingsUpdateCommand::PATCH_DEVICE_SHORT_ADDRESS != 0 {
            self.device_short_address =
                (body.device_short_address <= 63).then_some(body.device_short_address);
        }
        if self.application_active != was_active {
            self.application_active_moved_by = ApplicationActiveMover::Command;
        }
    }
}

impl RegistryStore {
    pub(crate) fn apply_dali_settings_from_command(
        &self,
        body: &DaliSettingsUpdateCommand,
    ) -> DaliSettingsRecord {
        let mut g = self.write_inner();
        g.dali_settings.apply_patch(body);
        let applied = g.dali_settings;
        drop(g);
        self.dirty.mark_dali_settings_dirty();
        applied
    }

    pub(crate) fn dali_settings_row(&self) -> DaliSettingsRecord {
        self.read_inner().dali_settings
    }
}

impl DaliSettingsReadPort for RegistryStore {
    fn dali_settings_view(&self) -> DaliSettingsView {
        self.dali_settings_row().to_view()
    }

    fn application_active_moved_by(&self) -> (bool, ApplicationActiveMover) {
        let row = self.dali_settings_row();
        (row.application_active, row.application_active_moved_by)
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistableDaliSettingsSlice {
    PersistableDaliSettingsSlice {
        dt8_auto_activation_repair: inner.dali_settings.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: inner.dali_settings.dt8_rgbwaf_control_assert,
        application_active: inner.dali_settings.application_active,
        device_short_address: inner.dali_settings.device_short_address,
    }
}

pub(crate) fn hydrate_dali_settings_inner(
    inner: &mut Inner,
    slice: &PersistableDaliSettingsSlice,
) {
    inner.dali_settings = DaliSettingsRecord {
        dt8_auto_activation_repair: slice.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: slice.dt8_rgbwaf_control_assert,
        application_active: slice.application_active,
        device_short_address: slice.device_short_address,
        application_active_moved_by: ApplicationActiveMover::Unmoved,
    };
}

impl RegistryStore {
    pub(crate) fn apply_application_control(
        &self,
        scope_broadcast: bool,
        short_address: u8,
        enable: bool,
    ) -> Option<DaliSettingsRecord> {
        let mut g = self.write_inner();
        let addressed_to_us = scope_broadcast
            || g.dali_settings.device_short_address == Some(short_address);
        if !addressed_to_us || g.dali_settings.application_active == enable {
            return None;
        }
        g.dali_settings.application_active = enable;
        g.dali_settings.application_active_moved_by = ApplicationActiveMover::Wire;
        let applied = g.dali_settings;
        drop(g);
        self.dirty.mark_dali_settings_dirty();
        Some(applied)
    }
}

#[cfg(test)]
mod application_control_tests {
    use super::*;

    #[test]
    fn scope_is_decided_here_and_only_our_address_counts() {
        let store = RegistryStore::with_adapter_count(1);
        assert!(store.apply_application_control(true, 0, false).is_some());
        assert!(!store.read_inner().dali_settings.application_active);
        assert!(store.apply_application_control(false, 5, true).is_none());
        assert!(!store.read_inner().dali_settings.application_active);
        store.write_inner().dali_settings.device_short_address = Some(5);
        assert!(store.apply_application_control(false, 5, true).is_some());
        assert!(store.read_inner().dali_settings.application_active);
        assert!(store.apply_application_control(false, 6, false).is_none());
        assert!(store.read_inner().dali_settings.application_active);
    }

    #[test]
    fn a_no_op_pair_is_not_a_change() {
        let store = RegistryStore::with_adapter_count(1);
        assert!(store.apply_application_control(true, 0, true).is_none());
    }
}
