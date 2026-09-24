use dali2rust_contracts::msg::RedundancySettingsUpdateCommand;
use dali2rust_domain::registry::{RedundancySettingsReadPort, RedundancySettingsView};

use super::persistence_slices::PersistableRedundancySettingsSlice;
use super::store::{Inner, RegistryStore};

const MAX_DEVICE_SHORT_ADDRESS: u8 = 63;

pub const MIN_PROBE_INTERVAL_MS: u32 = 250;
pub const MAX_PROBE_INTERVAL_MS: u32 = 60_000;
pub const MIN_TAKEOVER_AFTER_MISSED: u8 = 1;
pub const MAX_TAKEOVER_AFTER_MISSED: u8 = 5;
pub const MAX_BOOT_LISTEN_MS: u32 = 120_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RedundancySettingsRecord {
    pub enabled: bool,
    pub standby_role: bool,
    pub probe_interval_ms: u32,
    pub takeover_after_missed: u8,
    pub boot_listen_ms: u32,
    pub peer_device_short_address: Option<u8>,
    pub peer_url: String,
}

impl Default for RedundancySettingsRecord {
    fn default() -> Self {
        Self {
            enabled: false,
            standby_role: false,
            probe_interval_ms: 400,
            takeover_after_missed: 2,
            boot_listen_ms: 2_000,
            peer_device_short_address: None,
            peer_url: String::new(),
        }
    }
}

impl RedundancySettingsRecord {
    fn to_view(&self) -> RedundancySettingsView {
        RedundancySettingsView {
            enabled: self.enabled,
            standby_role: self.standby_role,
            probe_interval_ms: self.probe_interval_ms,
            takeover_after_missed: self.takeover_after_missed,
            boot_listen_ms: self.boot_listen_ms,
            peer_device_short_address: self.peer_device_short_address,
            peer_url: self.peer_url.clone(),
        }
    }

    fn apply_patch(&mut self, body: &RedundancySettingsUpdateCommand) {
        if body.patch_mask & RedundancySettingsUpdateCommand::PATCH_ENABLED != 0 {
            self.enabled = body.enabled;
        }
        if body.patch_mask & RedundancySettingsUpdateCommand::PATCH_ROLE != 0 {
            self.standby_role = body.standby_role;
        }
        if body.patch_mask & RedundancySettingsUpdateCommand::PATCH_PROBE_INTERVAL_MS != 0 {
            self.probe_interval_ms = body
                .probe_interval_ms
                .clamp(MIN_PROBE_INTERVAL_MS, MAX_PROBE_INTERVAL_MS);
        }
        if body.patch_mask & RedundancySettingsUpdateCommand::PATCH_TAKEOVER_AFTER_MISSED != 0 {
            self.takeover_after_missed = body
                .takeover_after_missed
                .clamp(MIN_TAKEOVER_AFTER_MISSED, MAX_TAKEOVER_AFTER_MISSED);
        }
        if body.patch_mask & RedundancySettingsUpdateCommand::PATCH_BOOT_LISTEN_MS != 0 {
            self.boot_listen_ms = body.boot_listen_ms.min(MAX_BOOT_LISTEN_MS);
        }
        if body.patch_mask & RedundancySettingsUpdateCommand::PATCH_PEER_URL != 0 {
            self.peer_url = body.peer_url.as_str().to_string();
        }
        if body.patch_mask & RedundancySettingsUpdateCommand::PATCH_PEER_SHORT_ADDRESS != 0 {
            self.peer_device_short_address = (body.peer_device_short_address
                <= MAX_DEVICE_SHORT_ADDRESS)
                .then_some(body.peer_device_short_address);
        }
    }
}

impl RegistryStore {
    pub(crate) fn apply_redundancy_settings_from_command(
        &self,
        body: &RedundancySettingsUpdateCommand,
    ) -> RedundancySettingsRecord {
        let mut g = self.write_inner();
        g.redundancy_settings.apply_patch(body);
        let applied = g.redundancy_settings.clone();
        drop(g);
        self.dirty.mark_redundancy_settings_dirty();
        applied
    }

    pub(crate) fn redundancy_settings_row(&self) -> RedundancySettingsRecord {
        self.read_inner().redundancy_settings.clone()
    }
}

impl RedundancySettingsReadPort for RegistryStore {
    fn redundancy_settings_view(&self) -> RedundancySettingsView {
        self.redundancy_settings_row().to_view()
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistableRedundancySettingsSlice {
    PersistableRedundancySettingsSlice {
        enabled: inner.redundancy_settings.enabled,
        standby_role: inner.redundancy_settings.standby_role,
        probe_interval_ms: inner.redundancy_settings.probe_interval_ms,
        takeover_after_missed: inner.redundancy_settings.takeover_after_missed,
        boot_listen_ms: inner.redundancy_settings.boot_listen_ms,
        peer_device_short_address: inner.redundancy_settings.peer_device_short_address,
        peer_url: inner.redundancy_settings.peer_url.clone(),
    }
}

pub(crate) fn hydrate_redundancy_settings_inner(
    inner: &mut Inner,
    slice: &PersistableRedundancySettingsSlice,
) {
    inner.redundancy_settings = RedundancySettingsRecord {
        enabled: slice.enabled,
        standby_role: slice.standby_role,
        probe_interval_ms: slice
            .probe_interval_ms
            .clamp(MIN_PROBE_INTERVAL_MS, MAX_PROBE_INTERVAL_MS),
        takeover_after_missed: slice
            .takeover_after_missed
            .clamp(MIN_TAKEOVER_AFTER_MISSED, MAX_TAKEOVER_AFTER_MISSED),
        boot_listen_ms: slice.boot_listen_ms.min(MAX_BOOT_LISTEN_MS),
        peer_device_short_address: slice
            .peer_device_short_address
            .filter(|a| *a <= MAX_DEVICE_SHORT_ADDRESS),
        peer_url: slice.peer_url.clone(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(mask: u8, mutate: impl FnOnce(&mut RedundancySettingsUpdateCommand)) -> RedundancySettingsUpdateCommand {
        let mut cmd = RedundancySettingsUpdateCommand {
            patch_mask: mask,
            enabled: false,
            standby_role: false,
            probe_interval_ms: 400,
            takeover_after_missed: 2,
            boot_listen_ms: 2_000,
            peer_device_short_address: 255,
            peer_url: dali2rust_contracts::msg::fixed_text_64(""),
        };
        mutate(&mut cmd);
        cmd
    }

    #[test]
    fn redundancy_is_off_and_primary_until_an_operator_says_otherwise() {
        let d = RedundancySettingsRecord::default();
        assert!(!d.enabled);
        assert!(!d.standby_role);
    }

    #[test]
    fn a_mask_touches_only_what_it_names() {
        let store = RegistryStore::with_adapter_count(1);
        let applied = store.apply_redundancy_settings_from_command(&patch(
            RedundancySettingsUpdateCommand::PATCH_ENABLED,
            |c| c.enabled = true,
        ));
        assert!(applied.enabled);
        assert!(!applied.standby_role);
        assert_eq!(applied.probe_interval_ms, 400);
    }

    #[test]
    fn a_stored_interval_out_of_band_is_clamped_rather_than_obeyed() {
        let store = RegistryStore::with_adapter_count(1);
        let fast = store.apply_redundancy_settings_from_command(&patch(
            RedundancySettingsUpdateCommand::PATCH_PROBE_INTERVAL_MS,
            |c| c.probe_interval_ms = 1,
        ));
        assert_eq!(fast.probe_interval_ms, MIN_PROBE_INTERVAL_MS);
        let slow = store.apply_redundancy_settings_from_command(&patch(
            RedundancySettingsUpdateCommand::PATCH_PROBE_INTERVAL_MS,
            |c| c.probe_interval_ms = u32::MAX,
        ));
        assert_eq!(slow.probe_interval_ms, MAX_PROBE_INTERVAL_MS);
    }

    #[test]
    fn zero_missed_probes_is_refused_by_the_clamp_not_accepted_as_instant() {
        let store = RegistryStore::with_adapter_count(1);
        let applied = store.apply_redundancy_settings_from_command(&patch(
            RedundancySettingsUpdateCommand::PATCH_TAKEOVER_AFTER_MISSED,
            |c| c.takeover_after_missed = 0,
        ));
        assert_eq!(applied.takeover_after_missed, MIN_TAKEOVER_AFTER_MISSED);
    }

    #[test]
    fn an_out_of_range_peer_address_reads_as_unset() {
        let store = RegistryStore::with_adapter_count(1);
        let applied = store.apply_redundancy_settings_from_command(&patch(
            RedundancySettingsUpdateCommand::PATCH_PEER_SHORT_ADDRESS,
            |c| c.peer_device_short_address = 200,
        ));
        assert_eq!(applied.peer_device_short_address, None);
        let set = store.apply_redundancy_settings_from_command(&patch(
            RedundancySettingsUpdateCommand::PATCH_PEER_SHORT_ADDRESS,
            |c| c.peer_device_short_address = 9,
        ));
        assert_eq!(set.peer_device_short_address, Some(9));
    }

    #[test]
    fn hydration_clamps_the_same_way_a_patch_does() {
        let store = RegistryStore::with_adapter_count(1);
        let mut inner = store.write_inner();
        hydrate_redundancy_settings_inner(
            &mut inner,
            &PersistableRedundancySettingsSlice {
                enabled: true,
                standby_role: true,
                probe_interval_ms: 0,
                takeover_after_missed: 99,
                boot_listen_ms: u32::MAX,
                peer_device_short_address: Some(200),
                peer_url: "http://peer.example".to_string(),
            },
        );
        assert_eq!(inner.redundancy_settings.probe_interval_ms, MIN_PROBE_INTERVAL_MS);
        assert_eq!(
            inner.redundancy_settings.takeover_after_missed,
            MAX_TAKEOVER_AFTER_MISSED
        );
        assert_eq!(inner.redundancy_settings.boot_listen_ms, MAX_BOOT_LISTEN_MS);
        assert_eq!(inner.redundancy_settings.peer_device_short_address, None);
        assert_eq!(inner.redundancy_settings.peer_url, "http://peer.example");
    }
}
