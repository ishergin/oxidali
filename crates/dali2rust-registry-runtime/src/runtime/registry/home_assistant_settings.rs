use dali2rust_contracts::msg::{
    HomeAssistantControllerIdUpdateCommand, HomeAssistantCredentialsUpdateCommand,
    HomeAssistantSettingsUpdateCommand, HomeAssistantTopicsUpdateCommand,
};
use dali2rust_domain::registry::{
    HomeAssistantSecretReadPort, HomeAssistantSettingsReadPort, HomeAssistantSettingsView,
};

use super::persistence_slices::PersistableHomeAssistantSettingsSlice;
use super::store::{Inner, RegistryStore};

const DEFAULT_BROKER_PORT: u16 = 1883;
const DEFAULT_DISCOVERY_PREFIX: &str = "homeassistant";
const DEFAULT_STATE_TOPIC_PREFIX: &str = "dali";
const DEFAULT_PUBLISH_QOS: u8 = 1;

const CONTROLLER_ID_FALLBACK: &str = "dali-controller";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HomeAssistantSettingsRecord {
    pub enabled: bool,
    pub broker_host: String,
    pub broker_port: u16,
    pub broker_username: String,
    pub broker_password: String,
    pub discovery_prefix: String,
    pub state_topic_prefix: String,
    pub controller_id: String,
    pub publish_qos: u8,
    pub retain_state: bool,
    pub retain_discovery: bool,
    pub expose_input_devices: bool,
}

impl Default for HomeAssistantSettingsRecord {
    fn default() -> Self {
        Self {
            enabled: false,
            broker_host: String::new(),
            broker_port: DEFAULT_BROKER_PORT,
            broker_username: String::new(),
            broker_password: String::new(),
            discovery_prefix: DEFAULT_DISCOVERY_PREFIX.to_string(),
            state_topic_prefix: DEFAULT_STATE_TOPIC_PREFIX.to_string(),
            controller_id: CONTROLLER_ID_FALLBACK.to_string(),
            publish_qos: DEFAULT_PUBLISH_QOS,
            retain_state: true,
            retain_discovery: true,
            expose_input_devices: true,
        }
    }
}

pub(crate) fn controller_id_from_mac(mac: [u8; 6]) -> String {
    format!("dali-{:02x}{:02x}{:02x}", mac[3], mac[4], mac[5])
}

impl HomeAssistantSettingsRecord {
    fn to_view(&self) -> HomeAssistantSettingsView {
        HomeAssistantSettingsView {
            enabled: self.enabled,
            broker_host: self.broker_host.clone(),
            broker_port: self.broker_port,
            broker_username: self.broker_username.clone(),
            broker_password_set: !self.broker_password.is_empty(),
            discovery_prefix: self.discovery_prefix.clone(),
            state_topic_prefix: self.state_topic_prefix.clone(),
            controller_id: self.controller_id.clone(),
            publish_qos: self.publish_qos,
            retain_state: self.retain_state,
            retain_discovery: self.retain_discovery,
            expose_input_devices: self.expose_input_devices,
        }
    }

    fn apply_settings_patch(&mut self, body: &HomeAssistantSettingsUpdateCommand) {
        if body.patch_mask & HomeAssistantSettingsUpdateCommand::PATCH_ENABLED != 0 {
            self.enabled = body.enabled;
        }
        if body.patch_mask & HomeAssistantSettingsUpdateCommand::PATCH_BROKER_HOST != 0 {
            self.broker_host = body.broker_host.as_str().to_string();
        }
        if body.patch_mask & HomeAssistantSettingsUpdateCommand::PATCH_BROKER_PORT != 0 {
            self.broker_port = body.broker_port;
        }
        if body.patch_mask & HomeAssistantSettingsUpdateCommand::PATCH_PUBLISH_QOS != 0 {
            self.publish_qos = body.publish_qos;
        }
        if body.patch_mask & HomeAssistantSettingsUpdateCommand::PATCH_RETAIN_STATE != 0 {
            self.retain_state = body.retain_state;
        }
        if body.patch_mask & HomeAssistantSettingsUpdateCommand::PATCH_RETAIN_DISCOVERY != 0 {
            self.retain_discovery = body.retain_discovery;
        }
        if body.patch_mask & HomeAssistantSettingsUpdateCommand::PATCH_EXPOSE_INPUT_DEVICES != 0 {
            self.expose_input_devices = body.expose_input_devices;
        }
    }

    fn apply_credentials_patch(&mut self, body: &HomeAssistantCredentialsUpdateCommand) {
        if body.patch_mask & HomeAssistantCredentialsUpdateCommand::PATCH_BROKER_USERNAME != 0 {
            self.broker_username = body.broker_username.as_str().to_string();
        }
        if body.patch_mask & HomeAssistantCredentialsUpdateCommand::PATCH_BROKER_PASSWORD != 0 {
            self.broker_password = body.broker_password.as_str().to_string();
        }
    }

    fn apply_topics_patch(&mut self, body: &HomeAssistantTopicsUpdateCommand) {
        if body.patch_mask & HomeAssistantTopicsUpdateCommand::PATCH_DISCOVERY_PREFIX != 0 {
            self.discovery_prefix = body.discovery_prefix.as_str().to_string();
        }
        if body.patch_mask & HomeAssistantTopicsUpdateCommand::PATCH_STATE_TOPIC_PREFIX != 0 {
            self.state_topic_prefix = body.state_topic_prefix.as_str().to_string();
        }
    }
}

impl RegistryStore {
    pub fn seed_home_assistant_controller_id(&self, mac: [u8; 6]) {
        let mut g = self.write_inner();
        g.home_assistant_settings.controller_id = controller_id_from_mac(mac);
    }

    pub(crate) fn apply_home_assistant_settings_from_command(
        &self,
        body: &HomeAssistantSettingsUpdateCommand,
    ) -> HomeAssistantSettingsRecord {
        self.mutate_home_assistant(|rec| rec.apply_settings_patch(body))
    }

    pub(crate) fn apply_home_assistant_credentials_from_command(
        &self,
        body: &HomeAssistantCredentialsUpdateCommand,
    ) -> HomeAssistantSettingsRecord {
        self.mutate_home_assistant(|rec| rec.apply_credentials_patch(body))
    }

    pub(crate) fn apply_home_assistant_topics_from_command(
        &self,
        body: &HomeAssistantTopicsUpdateCommand,
    ) -> HomeAssistantSettingsRecord {
        self.mutate_home_assistant(|rec| rec.apply_topics_patch(body))
    }

    pub(crate) fn apply_home_assistant_controller_id_from_command(
        &self,
        body: &HomeAssistantControllerIdUpdateCommand,
    ) -> HomeAssistantSettingsRecord {
        self.mutate_home_assistant(|rec| {
            rec.controller_id = body.controller_id.as_str().to_string();
        })
    }

    fn mutate_home_assistant(
        &self,
        patch: impl FnOnce(&mut HomeAssistantSettingsRecord),
    ) -> HomeAssistantSettingsRecord {
        let mut g = self.write_inner();
        patch(&mut g.home_assistant_settings);
        let applied = g.home_assistant_settings.clone();
        drop(g);
        self.dirty.mark_home_assistant_settings_dirty();
        applied
    }

}

impl HomeAssistantSettingsReadPort for RegistryStore {
    fn home_assistant_settings_view(&self) -> HomeAssistantSettingsView {
        self.read_inner().home_assistant_settings.to_view()
    }
}

impl HomeAssistantSecretReadPort for RegistryStore {
    fn home_assistant_broker_password(&self) -> String {
        self.read_inner().home_assistant_settings.broker_password.clone()
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistableHomeAssistantSettingsSlice {
    let r = &inner.home_assistant_settings;
    PersistableHomeAssistantSettingsSlice {
        enabled: r.enabled,
        broker_host: r.broker_host.clone(),
        broker_port: r.broker_port,
        broker_username: r.broker_username.clone(),
        broker_password: r.broker_password.clone(),
        discovery_prefix: r.discovery_prefix.clone(),
        state_topic_prefix: r.state_topic_prefix.clone(),
        controller_id: r.controller_id.clone(),
        publish_qos: r.publish_qos,
        retain_state: r.retain_state,
        retain_discovery: r.retain_discovery,
        expose_input_devices: r.expose_input_devices,
    }
}

pub(crate) fn hydrate_home_assistant_settings_inner(
    inner: &mut Inner,
    slice: &PersistableHomeAssistantSettingsSlice,
) {
    inner.home_assistant_settings = HomeAssistantSettingsRecord {
        enabled: slice.enabled,
        broker_host: slice.broker_host.clone(),
        broker_port: slice.broker_port,
        broker_username: slice.broker_username.clone(),
        broker_password: slice.broker_password.clone(),
        discovery_prefix: slice.discovery_prefix.clone(),
        state_topic_prefix: slice.state_topic_prefix.clone(),
        controller_id: slice.controller_id.clone(),
        publish_qos: slice.publish_qos,
        retain_state: slice.retain_state,
        retain_discovery: slice.retain_discovery,
        expose_input_devices: slice.expose_input_devices,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_controller_id_comes_from_the_last_three_mac_bytes_in_lowercase_hex() {
        assert_eq!(
            controller_id_from_mac([0x3c, 0x84, 0x27, 0xA1, 0xB2, 0xC3]),
            "dali-a1b2c3"
        );
    }

    #[test]
    fn a_mac_derived_controller_id_is_always_topic_safe() {
        for byte in 0u8..=255 {
            let id = controller_id_from_mac([0, 0, 0, byte, byte, byte]);
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "unsafe controller_id: {id}"
            );
        }
    }

    #[test]
    fn the_view_reports_that_a_password_is_set_without_carrying_it() {
        let mut rec = HomeAssistantSettingsRecord::default();
        assert!(!rec.to_view().broker_password_set);
        rec.broker_password = "hunter2".to_string();
        assert!(rec.to_view().broker_password_set);
    }
}
