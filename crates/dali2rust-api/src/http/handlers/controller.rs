use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_platform::clock::Clock;
use serde::Serialize;

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::json_stream_dto;
use crate::http::types::HttpResponse;
use crate::http::handlers::common::{require_get};

#[derive(Serialize)]
struct ControllerSummaryBody<'a> {
    controller_id: &'a str,
    firmware_version: &'a str,
    #[serde(rename = "target_mcu")]
    target_mcu: &'a str,
    uptime_ms: u64,
    network: ControllerNetworkBody<'a>,
    home_assistant: ControllerHaBody,
    cluster: ControllerClusterBody,
    adapter_count: u8,
    hydrated: bool,
}

#[derive(Serialize)]
struct ControllerNetworkBody<'a> {
    hostname: &'a str,
    ip: &'a str,
}

#[derive(Serialize)]
struct ControllerHaBody {
    enabled: bool,
    connected: bool,
    broker_url: String,
}

pub trait ControllerHaSummary: Send + Sync {
    fn ha_enabled_and_url(&self) -> (bool, String);
    fn ha_connected(&self) -> bool;
}

#[derive(Serialize)]
struct ControllerClusterBody {
    enabled: bool,
}

const TARGET_MCU: &str = if cfg!(target_arch = "riscv32") {
    "esp32p4"
} else {
    "host"
};

pub struct ControllerSummaryHandler {
    firmware_version: &'static str,
    adapter_count: u8,
    clock: Arc<dyn Clock>,
    home_assistant: Arc<dyn ControllerHaSummary>,
    started_ms: u64,
}

impl ControllerSummaryHandler {
    pub fn new(
        firmware_version: &'static str,
        adapter_count: u8,
        clock: Arc<dyn Clock>,
        home_assistant: Arc<dyn ControllerHaSummary>,
    ) -> Self {
        let started_ms = clock.monotonic_ms();
        Self {
            firmware_version,
            adapter_count,
            clock,
            home_assistant,
            started_ms,
        }
    }
}

impl ApiHandler for ControllerSummaryHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if let Err(error) = require_get(method) {
            return error;
        }
        let body = ControllerSummaryBody {
            controller_id: "local",
            firmware_version: self.firmware_version,
            target_mcu: TARGET_MCU,
            uptime_ms: self.clock.monotonic_ms().saturating_sub(self.started_ms),
            network: ControllerNetworkBody { hostname: "", ip: "" },
            home_assistant: {
                let (enabled, broker_url) = self.home_assistant.ha_enabled_and_url();
                ControllerHaBody {
                    enabled,
                    connected: self.home_assistant.ha_connected(),
                    broker_url,
                }
            },
            cluster: ControllerClusterBody { enabled: false },
            adapter_count: self.adapter_count,
            hydrated: true,
        };
        json_stream_dto(body)
    }
}
