use super::handler::ApiHandler;
use super::handlers::health::HealthHandler;
use super::router::{RouteSpec, Router};
use super::types::HttpMethod;
use dali2rust_platform::clock::Clock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteKey {
    Controller,
    Time,
    TimeSet,
    Diagnostics,
    Stats,
    AdaptersList,
    AdapterGet,
    DaliCommand,
    DaliLevel,
    DaliRaw,
    AdapterPatch,
    OperationGet,
    OperationsList,
    PhysicalDevicesList,
    PhysicalDeviceGet,
    PhysicalDeviceAttributes,
    PhysicalDeviceMemoryBanks,
    PhysicalDevicePatch,
    PhysicalDeviceDelete,
    PhysicalDeviceWriteAttrs,
    PhysicalDeviceTargetState,
    AdapterDiscoveryRuns,
    CommissioningIdentify,
    CommissioningAddressChanges,
    CommissioningSteps,
    CommissioningReplacements,
    PhysicalDeviceAttributeReads,
    GroupsList,
    GroupGet,
    GroupPatch,
    GroupMembershipMatrixGet,
    GroupMembershipMatrixPatch,
    GroupMembershipMatrixPut,
    GroupApply,
    GroupTargetState,
    VirtualLampsList,
    VirtualLampGet,
    VirtualLampPatch,
    VirtualLampBindingPut,
    VirtualLampBindingDelete,
    VirtualLampDelete,
    VirtualLampTargetState,
    ScenesList,
    SceneGet,
    ScenePatch,
    SceneMatrixGet,
    SceneMatrixPatch,
    SceneMatrixPut,
    SceneApply,
    SceneRecall,
    HclSchedulesList,
    HclScheduleCreate,
    HclScheduleGet,
    HclSchedulePatch,
    HclScheduleDelete,
    HclOverrideGet,
    HclOverrideDelete,
    SettingsPollerGet,
    SettingsPollerPatch,
    SettingsDaliGet,
    SettingsDaliPatch,
    SettingsRedundancyGet,
    SettingsRedundancyPatch,
    Redundancy,
    RedundancySwitchover,
    PoliciesGet,
    PoliciesPatch,
    PoliciesApply,
    ConfigManifest,
    ConfigSlice,
    ConfigSlicePut,
    SettingsHomeAssistantGet,
    SettingsHomeAssistantPatch,
    SettingsHomeAssistantDiscoveryPublish,
    InputDevicesList,
    InputDeviceGet,
    InputDevicePatch,
    InputDeviceDelete,
    InputDeviceInstancePatch,
    InputDeviceFeedbackPatch,
    RulesGet,
    RulesParse,
    RulesPut,
    RulesRulePatch,
    RulesRuleRun,
    InputDevicesScan,
    InputDevicesCommission,
    InputDeviceIdentify,
    Firmware,
    FirmwareUpdates,
    WebUiRoot,
    WebUiFallback,
}

const ROUTE_TABLE: &[(RouteKey, HttpMethod, &str)] = &[
    (RouteKey::Controller, HttpMethod::Get, "/api/v1/controller"),
    (RouteKey::Time, HttpMethod::Get, "/api/v1/time"),
    (RouteKey::TimeSet, HttpMethod::Put, "/api/v1/time"),
    (RouteKey::Diagnostics, HttpMethod::Get, "/api/v1/diagnostics"),
    (RouteKey::Stats, HttpMethod::Get, "/api/v1/stats"),
    (RouteKey::AdaptersList, HttpMethod::Get, "/api/v1/adapters"),
    (RouteKey::AdapterGet, HttpMethod::Get, "/api/v1/adapters/{id}"),
    (RouteKey::DaliCommand, HttpMethod::Post, "/api/v1/dali/command"),
    (RouteKey::DaliLevel, HttpMethod::Post, "/api/v1/dali/level"),
    (RouteKey::DaliRaw, HttpMethod::Post, "/api/v1/dali/raw"),
    (RouteKey::AdapterPatch, HttpMethod::Patch, "/api/v1/adapters/{id}"),
    (RouteKey::OperationGet, HttpMethod::Get, "/api/v1/operations/{id}"),
    (RouteKey::OperationsList, HttpMethod::Get, "/api/v1/operations"),
    (
        RouteKey::PhysicalDevicesList,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/physical-devices",
    ),
    (
        RouteKey::PhysicalDeviceGet,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/physical-devices/{short}",
    ),
    (
        RouteKey::PhysicalDeviceAttributes,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/physical-devices/{short}/attributes",
    ),
    (
        RouteKey::PhysicalDeviceMemoryBanks,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/physical-devices/{short}/memory-banks",
    ),
    (
        RouteKey::PhysicalDevicePatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/physical-devices/{short}",
    ),
    (
        RouteKey::PhysicalDeviceDelete,
        HttpMethod::Delete,
        "/api/v1/adapters/{adapter_id}/physical-devices/{short}",
    ),
    (
        RouteKey::PhysicalDeviceWriteAttrs,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/physical-devices/{short}/write-attributes",
    ),
    (
        RouteKey::PhysicalDeviceTargetState,
        HttpMethod::Put,
        "/api/v1/adapters/{adapter_id}/physical-devices/{short}/target-state",
    ),
    (
        RouteKey::AdapterDiscoveryRuns,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/discovery-runs",
    ),
    (
        RouteKey::CommissioningIdentify,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/commissioning/identify",
    ),
    (
        RouteKey::CommissioningAddressChanges,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/commissioning/address-changes",
    ),
    (
        RouteKey::CommissioningSteps,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/commissioning/steps/{step}",
    ),
    (
        RouteKey::CommissioningReplacements,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/commissioning/replacements",
    ),
    (
        RouteKey::PhysicalDeviceAttributeReads,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/physical-devices/{short}/attribute-reads",
    ),
    (
        RouteKey::GroupsList,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/groups",
    ),
    (
        RouteKey::GroupGet,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/groups/{group_id}",
    ),
    (
        RouteKey::GroupPatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/groups/{group_id}",
    ),
    (
        RouteKey::GroupMembershipMatrixGet,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/group-membership-matrix",
    ),
    (
        RouteKey::GroupMembershipMatrixPatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/group-membership-matrix",
    ),
    (
        RouteKey::GroupMembershipMatrixPut,
        HttpMethod::Put,
        "/api/v1/adapters/{adapter_id}/group-membership-matrix",
    ),
    (
        RouteKey::GroupApply,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/groups/apply",
    ),
    (
        RouteKey::GroupTargetState,
        HttpMethod::Put,
        "/api/v1/adapters/{adapter_id}/groups/{group_id}/target-state",
    ),
    (
        RouteKey::VirtualLampsList,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/virtual-lamps",
    ),
    (
        RouteKey::VirtualLampGet,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/virtual-lamps/{lamp_id}",
    ),
    (
        RouteKey::VirtualLampPatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/virtual-lamps/{lamp_id}",
    ),
    (
        RouteKey::VirtualLampBindingPut,
        HttpMethod::Put,
        "/api/v1/adapters/{adapter_id}/virtual-lamps/{lamp_id}/binding",
    ),
    (
        RouteKey::VirtualLampBindingDelete,
        HttpMethod::Delete,
        "/api/v1/adapters/{adapter_id}/virtual-lamps/{lamp_id}/binding",
    ),
    (
        RouteKey::VirtualLampDelete,
        HttpMethod::Delete,
        "/api/v1/adapters/{adapter_id}/virtual-lamps/{lamp_id}",
    ),
    (
        RouteKey::VirtualLampTargetState,
        HttpMethod::Put,
        "/api/v1/adapters/{adapter_id}/virtual-lamps/{lamp_id}/target-state",
    ),
    (
        RouteKey::ScenesList,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/scenes",
    ),
    (
        RouteKey::SceneGet,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/scenes/{scene_id}",
    ),
    (
        RouteKey::ScenePatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/scenes/{scene_id}",
    ),
    (
        RouteKey::SceneMatrixGet,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/scenes/{scene_id}/matrix",
    ),
    (
        RouteKey::SceneMatrixPatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/scenes/{scene_id}/matrix",
    ),
    (
        RouteKey::SceneMatrixPut,
        HttpMethod::Put,
        "/api/v1/adapters/{adapter_id}/scenes/{scene_id}/matrix",
    ),
    (
        RouteKey::SceneApply,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/scenes/{scene_id}/apply",
    ),
    (
        RouteKey::SceneRecall,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/scenes/{scene_id}/recall",
    ),
    (
        RouteKey::HclSchedulesList,
        HttpMethod::Get,
        "/api/v1/hcl-schedules",
    ),
    (
        RouteKey::HclScheduleCreate,
        HttpMethod::Post,
        "/api/v1/hcl-schedules",
    ),
    (
        RouteKey::HclScheduleGet,
        HttpMethod::Get,
        "/api/v1/hcl-schedules/{schedule_id}",
    ),
    (
        RouteKey::HclSchedulePatch,
        HttpMethod::Patch,
        "/api/v1/hcl-schedules/{schedule_id}",
    ),
    (
        RouteKey::HclScheduleDelete,
        HttpMethod::Delete,
        "/api/v1/hcl-schedules/{schedule_id}",
    ),
    (
        RouteKey::HclOverrideGet,
        HttpMethod::Get,
        "/api/v1/hcl-schedules/{schedule_id}/override",
    ),
    (
        RouteKey::HclOverrideDelete,
        HttpMethod::Delete,
        "/api/v1/hcl-schedules/{schedule_id}/override",
    ),
    (
        RouteKey::SettingsPollerGet,
        HttpMethod::Get,
        "/api/v1/settings/poller",
    ),
    (
        RouteKey::SettingsPollerPatch,
        HttpMethod::Patch,
        "/api/v1/settings/poller",
    ),
    (
        RouteKey::SettingsDaliGet,
        HttpMethod::Get,
        "/api/v1/settings/dali",
    ),
    (
        RouteKey::SettingsDaliPatch,
        HttpMethod::Patch,
        "/api/v1/settings/dali",
    ),
    (
        RouteKey::SettingsRedundancyGet,
        HttpMethod::Get,
        "/api/v1/settings/redundancy",
    ),
    (
        RouteKey::SettingsRedundancyPatch,
        HttpMethod::Patch,
        "/api/v1/settings/redundancy",
    ),
    (RouteKey::Redundancy, HttpMethod::Get, "/api/v1/redundancy"),
    (
        RouteKey::RedundancySwitchover,
        HttpMethod::Post,
        "/api/v1/redundancy/switchover",
    ),
    (RouteKey::PoliciesGet, HttpMethod::Get, "/api/v1/policies"),
    (
        RouteKey::PoliciesPatch,
        HttpMethod::Patch,
        "/api/v1/policies",
    ),
    (
        RouteKey::PoliciesApply,
        HttpMethod::Post,
        "/api/v1/policies/apply",
    ),
    (
        RouteKey::ConfigManifest,
        HttpMethod::Get,
        "/api/v1/config/slices",
    ),
    (
        RouteKey::ConfigSlice,
        HttpMethod::Get,
        "/api/v1/config/slices/{slice}",
    ),
    (
        RouteKey::ConfigSlicePut,
        HttpMethod::Put,
        "/api/v1/config/slices/{slice}",
    ),
    (
        RouteKey::SettingsHomeAssistantGet,
        HttpMethod::Get,
        "/api/v1/settings/home-assistant",
    ),
    (
        RouteKey::SettingsHomeAssistantPatch,
        HttpMethod::Patch,
        "/api/v1/settings/home-assistant",
    ),
    (
        RouteKey::SettingsHomeAssistantDiscoveryPublish,
        HttpMethod::Post,
        "/api/v1/settings/home-assistant/discovery-publish",
    ),
    (
        RouteKey::InputDevicesList,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/input-devices",
    ),
    (
        RouteKey::InputDevicesScan,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/input-devices/scan",
    ),
    (
        RouteKey::InputDevicesCommission,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/input-devices/commission",
    ),
    (
        RouteKey::InputDeviceGet,
        HttpMethod::Get,
        "/api/v1/adapters/{adapter_id}/input-devices/{short_address}",
    ),
    (
        RouteKey::InputDevicePatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/input-devices/{short_address}",
    ),
    (
        RouteKey::InputDeviceDelete,
        HttpMethod::Delete,
        "/api/v1/adapters/{adapter_id}/input-devices/{short_address}",
    ),
    (
        RouteKey::InputDeviceIdentify,
        HttpMethod::Post,
        "/api/v1/adapters/{adapter_id}/input-devices/{short_address}/identify",
    ),
    (
        RouteKey::InputDeviceInstancePatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/input-devices/{short_address}/instances/{instance_number}",
    ),
    (
        RouteKey::InputDeviceFeedbackPatch,
        HttpMethod::Patch,
        "/api/v1/adapters/{adapter_id}/input-devices/{short_address}/instances/{instance_number}/feedback",
    ),
    (RouteKey::Firmware, HttpMethod::Get, "/api/v1/firmware"),
    (RouteKey::FirmwareUpdates, HttpMethod::Post, "/api/v1/firmware/updates"),
    (RouteKey::RulesGet, HttpMethod::Get, "/api/v1/rules"),
    (RouteKey::RulesPut, HttpMethod::Put, "/api/v1/rules"),
    (RouteKey::RulesParse, HttpMethod::Post, "/api/v1/rules/parse"),
    (
        RouteKey::RulesRulePatch,
        HttpMethod::Patch,
        "/api/v1/rules/{rule_name}",
    ),
    (
        RouteKey::RulesRuleRun,
        HttpMethod::Post,
        "/api/v1/rules/{rule_name}/run",
    ),
    (RouteKey::WebUiRoot, HttpMethod::Get, "/"),
    (RouteKey::WebUiFallback, HttpMethod::Get, "/{*rest}"),
];

pub struct AppBuilder {
    version: &'static str,
    clock: Arc<dyn Clock>,
    handlers: HashMap<RouteKey, Box<dyn ApiHandler>>,
    role: Option<Arc<dyn crate::http::role::ControllerRolePort>>,
}

impl AppBuilder {
    pub fn new(version: &'static str, clock: Arc<dyn Clock>) -> Self {
        Self {
            version,
            clock,
            handlers: HashMap::new(),
            role: None,
        }
    }

    pub fn with_handler(mut self, key: RouteKey, handler: Box<dyn ApiHandler>) -> Self {
        self.handlers.insert(key, handler);
        self
    }

    pub fn with_role_port(
        mut self,
        role: Arc<dyn crate::http::role::ControllerRolePort>,
    ) -> Self {
        self.role = Some(role);
        self
    }

    pub fn build(mut self) -> Router {
        let role = self.role.take();
        let health = match role.as_ref() {
            Some(port) => HealthHandler::new(Arc::clone(&self.clock), self.version)
                .with_role(Arc::clone(port)),
            None => HealthHandler::new(Arc::clone(&self.clock), self.version),
        };
        let mut router = match role {
            Some(role) => Router::new().with_role_port(role),
            None => Router::new(),
        };
        router
            .register(RouteSpec::get("/api/v1/health", Box::new(health)))
            .expect("health route registration");
        for (key, method, path) in ROUTE_TABLE {
            let Some(handler) = self.handlers.remove(key) else {
                continue;
            };
            router
                .register(RouteSpec::new(*method, path, handler))
                .unwrap_or_else(|e| panic!("route registration for {method:?} {path}: {e:?}"));
        }
        assert!(
            self.handlers.is_empty(),
            "handler registered for a RouteKey missing from ROUTE_TABLE"
        );
        router
    }
}

#[cfg(test)]
mod route_table_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn route_table_is_complete_and_unique() {
        assert_eq!(ROUTE_TABLE.len(), 92, "route table row count");
        let mut keys = HashSet::new();
        let mut routes = HashSet::new();
        for (key, method, path) in ROUTE_TABLE {
            assert!(keys.insert(*key), "duplicate RouteKey {key:?}");
            assert!(
                routes.insert((*method, *path)),
                "duplicate route {method:?} {path}"
            );
        }
    }

    struct FixedClock;

    impl Clock for FixedClock {
        fn monotonic_ms(&self) -> u64 {
            0
        }
    }

    struct NoopHandler;

    impl ApiHandler for NoopHandler {
        fn handle_request(
            &self,
            _method: &str,
            _path: &str,
            _body: &[u8],
            _params: &HashMap<String, String>,
        ) -> super::super::types::HttpResponse {
            super::super::types::HttpResponse::no_content()
        }
    }

    #[test]
    fn full_route_table_registers_without_conflicts() {
        let mut builder = AppBuilder::new("0.0.0-test", Arc::new(FixedClock));
        for (key, _, _) in ROUTE_TABLE {
            builder = builder.with_handler(*key, Box::new(NoopHandler));
        }
        let router = builder.build();
        assert_eq!(router.dispatch("GET", "/api/v1/health", &[]).status, 200);
    }
}

#[cfg(test)]
mod build_router_tests {
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_adapters::{build_router_with_bus_and_transport, HardwareDisplay, StackOptions};
    use std::sync::{Arc, Mutex};

    #[test]
    fn bus_stack_wires_health_dali_routes() {
        let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
        let (router, _ws, _stack) = build_router_with_bus_and_transport(
            "9.9.9-test",
            transport,
            HardwareDisplay::none(),
            StackOptions::default(),
        );

        let health = router.dispatch("GET", "/api/v1/health", &[]);
        assert_eq!(health.status, 200);
        assert_eq!(health.content_type, "application/json");

        let json_body = br#"{"wire_address":0,"command":254}"#;
        let dali = router.dispatch("POST", "/api/v1/dali/command", json_body);
        assert_eq!(dali.status, 200);
        let parsed: serde_json::Value =
            serde_json::from_slice(&dali.into_body_bytes()).expect("parse JSON response");
        assert!(
            parsed["success"].as_bool().unwrap(),
            "mock transport should report success for non-query command"
        );

        let level_body = br#"{"wire_address":0,"level":128}"#;
        let level = router.dispatch("POST", "/api/v1/dali/level", level_body);
        assert_eq!(level.status, 200);
        assert_eq!(level.content_type, "application/json");

        let raw_body = br#"{"frame":256}"#;
        let raw = router.dispatch("POST", "/api/v1/dali/raw", raw_body);
        assert_eq!(raw.status, 200);
        assert_eq!(raw.content_type, "application/json");

        let removed = router.dispatch(
            "POST",
            "/api/v1/adapters/0/physical-devices/0/memory-bank-reads",
            br#"{"bank":0,"start":0,"length":1}"#,
        );
        assert_eq!(removed.status, 404);
    }
}
