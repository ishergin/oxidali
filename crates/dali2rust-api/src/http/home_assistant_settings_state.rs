use dali2rust_domain::registry::HomeAssistantSettingsView;
use serde::Serialize;

use crate::http::settings_surface::declare_settings_http_surface;

#[derive(Clone, Debug, Serialize)]
pub struct HomeAssistantSettingsDto {
    pub enabled: bool,
    pub broker_host: String,
    pub broker_port: u16,
    pub broker_username: String,
    pub broker_password_set: bool,
    pub broker_url_view: String,
    pub discovery_prefix: String,
    pub state_topic_prefix: String,
    pub controller_id: String,
    pub publish_qos: u8,
    pub retain_state: bool,
    pub retain_discovery: bool,
    pub expose_input_devices: bool,
}

declare_settings_http_surface! {
    dto: HomeAssistantSettingsDto,
    read_port: HomeAssistantSettingsReadPort,
    view_fn: home_assistant_settings_view,
    view_to_dto: home_assistant_settings_view_to_dto,
    watch_port: HomeAssistantSettingsApplyWatchPort,
    watch_load: home_assistant_settings_applied_load,
    http_state: HomeAssistantSettingsHttpState,
    dto_fn: home_assistant_settings_dto,
    apply_watch: HomeAssistantSettingsApplyWatch,
    read_bridge: HomeAssistantSettingsHttpStateBridge,
    watch_bridge: HomeAssistantSettingsApplyWatchBridge,
}

pub(crate) fn home_assistant_settings_view_to_dto(
    view: HomeAssistantSettingsView,
) -> HomeAssistantSettingsDto {
    let broker_url_view = view.broker_url_view();
    HomeAssistantSettingsDto {
        enabled: view.enabled,
        broker_host: view.broker_host,
        broker_port: view.broker_port,
        broker_username: view.broker_username,
        broker_password_set: view.broker_password_set,
        broker_url_view,
        discovery_prefix: view.discovery_prefix,
        state_topic_prefix: view.state_topic_prefix,
        controller_id: view.controller_id,
        publish_qos: view.publish_qos,
        retain_state: view.retain_state,
        retain_discovery: view.retain_discovery,
        expose_input_devices: view.expose_input_devices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_broker_has_no_url_view_rather_than_a_url_to_nowhere() {
        let dto = home_assistant_settings_view_to_dto(HomeAssistantSettingsView::default());
        assert_eq!(dto.broker_url_view, "");
    }

    #[test]
    fn the_url_view_shows_host_and_port_together() {
        let view = HomeAssistantSettingsView {
            broker_host: "mqtt.example.org".to_string(),
            broker_port: 1883,
            ..Default::default()
        };
        assert_eq!(
            home_assistant_settings_view_to_dto(view).broker_url_view,
            "mqtt://mqtt.example.org:1883"
        );
    }

    #[test]
    fn the_serialized_dto_never_carries_a_password() {
        let view = HomeAssistantSettingsView {
            broker_password_set: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&home_assistant_settings_view_to_dto(view)).unwrap();
        assert!(json.contains("broker_password_set"));
        assert!(!json.contains("\"broker_password\""));
    }
}
