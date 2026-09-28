use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_platform::clock::Clock;
use dali2rust_platform::net::{ipv4_text, mac_text, node_name, NetworkLink};
use serde::Serialize;

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::json_stream_dto;
use crate::http::types::HttpResponse;
use crate::http::handlers::common::{require_get};

#[derive(Serialize)]
struct ControllerSummaryBody {
    controller_id: String,
    node_id: Option<String>,
    firmware_version: &'static str,
    #[serde(rename = "target_mcu")]
    target_mcu: &'static str,
    uptime_ms: u64,
    network: ControllerNetworkBody,
    home_assistant: ControllerHaBody,
    cluster: ControllerClusterBody,
    adapter_count: u8,
    hydrated: bool,
}

#[derive(Serialize)]
struct ControllerNetworkBody {
    hostname: Option<String>,
    ip: Option<String>,
    mac: Option<String>,
}

#[derive(Serialize)]
struct ControllerHaBody {
    enabled: bool,
    connected: bool,
    broker_url: String,
}

pub trait ControllerSummarySource: Send + Sync {
    fn installation_id(&self) -> String;
    fn ha_enabled_and_url(&self) -> (bool, String);
    fn ha_connected(&self) -> bool;
    fn network_link(&self) -> Option<&dyn NetworkLink>;
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
    source: Arc<dyn ControllerSummarySource>,
    started_ms: u64,
}

impl ControllerSummaryHandler {
    pub fn new(
        firmware_version: &'static str,
        adapter_count: u8,
        clock: Arc<dyn Clock>,
        source: Arc<dyn ControllerSummarySource>,
    ) -> Self {
        let started_ms = clock.monotonic_ms();
        Self {
            firmware_version,
            adapter_count,
            clock,
            source,
            started_ms,
        }
    }

    fn summary_body(&self) -> ControllerSummaryBody {
        let link = self.source.network_link();
        let (enabled, broker_url) = self.source.ha_enabled_and_url();
        ControllerSummaryBody {
            controller_id: self.source.installation_id(),
            node_id: link.and_then(|l| l.hardware_address()).map(node_name),
            firmware_version: self.firmware_version,
            target_mcu: TARGET_MCU,
            uptime_ms: self.clock.monotonic_ms().saturating_sub(self.started_ms),
            network: network_body(link),
            home_assistant: ControllerHaBody {
                enabled,
                connected: self.source.ha_connected(),
                broker_url,
            },
            cluster: ControllerClusterBody { enabled: false },
            adapter_count: self.adapter_count,
            hydrated: true,
        }
    }
}

fn network_body(link: Option<&dyn NetworkLink>) -> ControllerNetworkBody {
    ControllerNetworkBody {
        hostname: link.and_then(|l| l.hostname()),
        ip: link.and_then(|l| l.status().ipv4).map(ipv4_text),
        mac: link.and_then(|l| l.hardware_address()).map(mac_text),
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
        json_stream_dto(self.summary_body())
    }
}
