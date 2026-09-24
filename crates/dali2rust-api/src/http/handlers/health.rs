use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_domain::health::{HealthService, HealthStatus};
use dali2rust_platform::clock::Clock;

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::json_stream_dto;
use crate::http::types::HttpResponse;

fn status_str(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Ok => "ok",
        HealthStatus::Degraded => "degraded",
        HealthStatus::Down => "down",
    }
}

pub struct HealthHandler {
    service: HealthService,
    clock: Arc<dyn Clock>,
    role: Option<Arc<dyn crate::http::role::ControllerRolePort>>,
}

impl HealthHandler {
    pub fn new(clock: Arc<dyn Clock>, version: &'static str) -> Self {
        Self {
            service: HealthService::new(clock.as_ref(), version),
            clock,
            role: None,
        }
    }

    #[must_use]
    pub fn with_role(mut self, role: Arc<dyn crate::http::role::ControllerRolePort>) -> Self {
        self.role = Some(role);
        self
    }
}

impl ApiHandler for HealthHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if _method != "GET" {
            return HttpResponse::method_not_allowed();
        }
        let report = self.service.check(self.clock.as_ref());
        let mut body = serde_json::json!({
            "status": status_str(report.status),
            "uptime_seconds": report.uptime_secs,
            "version": report.version,
        });
        if let (Some(role), Some(map)) = (self.role.as_ref(), body.as_object_mut()) {
            map.insert(
                "role".to_string(),
                serde_json::Value::String(crate::http::role::role_name(role.is_active()).into()),
            );
        }
        json_stream_dto(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    struct FakeClock {
        now: AtomicU64,
    }

    impl FakeClock {
        fn new(start_ms: u64) -> Self {
            Self {
                now: AtomicU64::new(start_ms),
            }
        }
    }

    impl Clock for FakeClock {
        fn monotonic_ms(&self) -> u64 {
            self.now.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn health_handler_returns_ok() {
        let clock: Arc<dyn Clock> = Arc::new(FakeClock::new(0));
        let handler = HealthHandler::new(clock, "0.1.0-test");
        let resp = handler.handle_request("GET", "/api/v1/health", &[], &HashMap::new());
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/json");

        let parsed: serde_json::Value =
            serde_json::from_slice(&resp.into_body_bytes()).expect("parse JSON response");
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["version"], "0.1.0-test");
    }

    #[test]
    fn health_handler_uptime_increases() {
        let clock: Arc<dyn Clock> = Arc::new(FakeClock::new(0));
        let handler = HealthHandler::new(clock, "test");
        let resp1 = handler.handle_request("GET", "/", &[], &HashMap::new());
        let r1: serde_json::Value =
            serde_json::from_slice(&resp1.into_body_bytes()).expect("parse");
        assert_eq!(r1["uptime_seconds"].as_u64().unwrap(), 0);
    }
}
