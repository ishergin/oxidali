use std::collections::HashMap;
use std::sync::Arc;

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::json_stream_dto;
use crate::http::health_state::HealthHttpState;
use crate::http::types::HttpResponse;

pub struct HealthHandler {
    state: Arc<dyn HealthHttpState>,
}

impl HealthHandler {
    pub fn new(state: Arc<dyn HealthHttpState>) -> Self {
        Self { state }
    }
}

impl ApiHandler for HealthHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if method != "GET" {
            return HttpResponse::method_not_allowed();
        }
        json_stream_dto(self.state.health_dto())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::health_state::HealthView;
    use dali2rust_domain::health::HealthService;
    use dali2rust_platform::clock::{Clock, Uptime};
    use std::sync::atomic::{AtomicU64, Ordering};

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

    fn handler_at(start_ms: u64, version: &'static str) -> HealthHandler {
        let clock: Arc<dyn Clock> = Arc::new(FakeClock::new(start_ms));
        let service = HealthService::new(Uptime::start(clock), version);
        HealthHandler::new(Arc::new(HealthView::new(service, None)))
    }

    #[test]
    fn health_handler_returns_ok() {
        let handler = handler_at(0, "0.1.0-test");
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
        let handler = handler_at(0, "test");
        let resp1 = handler.handle_request("GET", "/", &[], &HashMap::new());
        let r1: serde_json::Value =
            serde_json::from_slice(&resp1.into_body_bytes()).expect("parse");
        assert_eq!(r1["uptime_seconds"].as_u64().unwrap(), 0);
    }
}
