use std::sync::Arc;

use dali2rust_domain::health::{HealthService, HealthStatus};
use serde::Serialize;

use crate::http::role::{role_name, ControllerRolePort};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct HealthDto {
    pub status: &'static str,
    pub uptime_seconds: u64,
    pub version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiescent: Option<bool>,
}

pub trait HealthHttpState: Send + Sync {
    fn health_dto(&self) -> HealthDto;
}

pub struct HealthView {
    service: HealthService,
    role: Option<Arc<dyn ControllerRolePort>>,
}

impl HealthView {
    #[must_use]
    pub fn new(service: HealthService, role: Option<Arc<dyn ControllerRolePort>>) -> Self {
        Self { service, role }
    }
}

impl HealthHttpState for HealthView {
    // IEC 62386-103 §9.9.3
    fn health_dto(&self) -> HealthDto {
        let report = self.service.check();
        HealthDto {
            status: status_str(report.status),
            uptime_seconds: report.uptime_secs,
            version: report.version,
            role: self.role.as_ref().map(|port| role_name(port.is_active())),
            quiescent: self
                .role
                .as_ref()
                .map(|port| port.quiescent_remaining_ms().is_some()),
        }
    }
}

#[must_use]
fn status_str(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Ok => "ok",
        HealthStatus::Degraded => "degraded",
        HealthStatus::Down => "down",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dali2rust_platform::clock::{Clock, Uptime};

    use super::*;

    struct Stopped;

    impl Clock for Stopped {
        fn monotonic_ms(&self) -> u64 {
            0
        }
    }

    struct Held(bool);

    impl ControllerRolePort for Held {
        fn is_active(&self) -> bool {
            true
        }

        fn quiescent_remaining_ms(&self) -> Option<u32> {
            self.0.then_some(1_000)
        }
    }

    fn view(role: Option<Arc<dyn ControllerRolePort>>) -> HealthView {
        let clock: Arc<dyn Clock> = Arc::new(Stopped);
        HealthView::new(HealthService::new(Uptime::start(clock), "0.1.0-test"), role)
    }

    fn json(view: &HealthView) -> serde_json::Value {
        serde_json::to_value(view.health_dto()).expect("a health DTO serializes")
    }

    #[test]
    fn a_controller_without_a_role_port_answers_neither_role_nor_quiescence() {
        let body = json(&view(None));
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], "0.1.0-test");
        assert!(body.get("role").is_none(), "{body}");
        assert!(body.get("quiescent").is_none(), "{body}");
    }

    #[test]
    fn every_status_has_its_own_spelling() {
        assert_eq!(status_str(HealthStatus::Ok), "ok");
        assert_eq!(status_str(HealthStatus::Degraded), "degraded");
        assert_eq!(status_str(HealthStatus::Down), "down");
    }

    #[test]
    fn quiescent_mode_is_a_held_deadline_not_a_role() {
        assert_eq!(json(&view(Some(Arc::new(Held(true)))))["quiescent"], true);
        assert_eq!(json(&view(Some(Arc::new(Held(false)))))["quiescent"], false);
        assert_eq!(json(&view(Some(Arc::new(Held(true)))))["role"], "active");
    }
}
