use dali2rust_contracts::msg::DisplaySettingsUpdateCommand;
use dali2rust_domain::registry::{DisplaySettingsReadPort, DisplaySettingsView};

use super::persistence_slices::PersistableDisplaySettingsSlice;
use super::store::{Inner, RegistryStore};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DisplaySettingsRecord(pub DisplaySettingsView);

impl DisplaySettingsRecord {
    fn apply_patch(&mut self, body: &DisplaySettingsUpdateCommand) {
        let row = &mut self.0;
        if body.patch_mask & DisplaySettingsUpdateCommand::PATCH_ENABLED != 0 {
            row.enabled = body.enabled;
        }
        if body.patch_mask & DisplaySettingsUpdateCommand::PATCH_I2C_ADDRESS != 0 {
            row.i2c_address = body.i2c_address;
        }
        if body.patch_mask & DisplaySettingsUpdateCommand::PATCH_UPSIDE_DOWN != 0 {
            row.upside_down = body.upside_down;
        }
        if body.patch_mask & DisplaySettingsUpdateCommand::PATCH_SCREENSAVER_TIMEOUT_S != 0 {
            row.screensaver_timeout_s = body.screensaver_timeout_s;
        }
        if body.patch_mask & DisplaySettingsUpdateCommand::PATCH_PAGES_MASK != 0 {
            row.pages_mask = body.pages_mask;
        }
        self.0 = row.sanitized();
    }
}

impl RegistryStore {
    pub(crate) fn apply_display_settings_from_command(
        &self,
        body: &DisplaySettingsUpdateCommand,
    ) -> DisplaySettingsRecord {
        let mut g = self.write_inner();
        g.display_settings.apply_patch(body);
        let applied = g.display_settings;
        drop(g);
        self.dirty.mark_display_settings_dirty();
        applied
    }

    pub(crate) fn display_settings_row(&self) -> DisplaySettingsRecord {
        self.read_inner().display_settings
    }
}

impl DisplaySettingsReadPort for RegistryStore {
    fn display_settings_view(&self) -> DisplaySettingsView {
        self.display_settings_row().0
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistableDisplaySettingsSlice {
    let row = inner.display_settings.0;
    PersistableDisplaySettingsSlice {
        enabled: row.enabled,
        i2c_address: row.i2c_address,
        upside_down: row.upside_down,
        screensaver_timeout_s: row.screensaver_timeout_s,
        pages_mask: row.pages_mask,
    }
}

pub(crate) fn hydrate_display_settings_inner(
    inner: &mut Inner,
    slice: &PersistableDisplaySettingsSlice,
) {
    let stored = DisplaySettingsView {
        enabled: slice.enabled,
        i2c_address: slice.i2c_address,
        upside_down: slice.upside_down,
        screensaver_timeout_s: slice.screensaver_timeout_s,
        pages_mask: slice.pages_mask,
    };
    inner.display_settings = DisplaySettingsRecord(stored.sanitized());
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::DisplayPage;

    fn patch(patch_mask: u8) -> DisplaySettingsUpdateCommand {
        DisplaySettingsUpdateCommand {
            patch_mask,
            enabled: false,
            i2c_address: 0x3D,
            upside_down: true,
            screensaver_timeout_s: 60,
            pages_mask: DisplayPage::Bus.mask_bit(),
        }
    }

    #[test]
    fn a_patch_moves_only_the_fields_its_mask_names() {
        let mut record = DisplaySettingsRecord::default();
        record.apply_patch(&patch(DisplaySettingsUpdateCommand::PATCH_UPSIDE_DOWN));
        let expected = DisplaySettingsView { upside_down: true, ..DisplaySettingsView::default() };
        assert_eq!(record.0, expected);
    }

    #[test]
    fn a_value_outside_its_domain_never_lands_in_the_record() {
        let mut record = DisplaySettingsRecord::default();
        let mut body = patch(
            DisplaySettingsUpdateCommand::PATCH_I2C_ADDRESS
                | DisplaySettingsUpdateCommand::PATCH_PAGES_MASK
                | DisplaySettingsUpdateCommand::PATCH_SCREENSAVER_TIMEOUT_S,
        );
        body.i2c_address = 0x3E;
        body.pages_mask = 0;
        body.screensaver_timeout_s = 3;
        record.apply_patch(&body);
        assert_eq!(record.0, DisplaySettingsView::default());
    }
}
