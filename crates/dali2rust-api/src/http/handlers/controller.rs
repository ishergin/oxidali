use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_platform::clock::Clock;
use dali2rust_platform::net::{ipv4_text, mac_text, node_name};
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
    own_topics_refused: Vec<String>,
}

pub struct ControllerFacts {
    pub installation_id: String,
    pub ha_enabled: bool,
    pub ha_connected: bool,
    pub ha_own_topics_refused: Vec<String>,
    pub broker_url: String,
    pub mac: Option<[u8; 6]>,
    pub hostname: Option<String>,
    pub ipv4: Option<[u8; 4]>,
}

pub trait ControllerSummarySource: Send + Sync {
    fn facts(&self) -> ControllerFacts;
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
        let facts = self.source.facts();
        ControllerSummaryBody {
            node_id: facts.mac.map(node_name),
            network: network_body(facts.hostname, facts.ipv4, facts.mac),
            home_assistant: ControllerHaBody {
                enabled: facts.ha_enabled,
                connected: facts.ha_connected,
                broker_url: facts.broker_url,
                own_topics_refused: facts.ha_own_topics_refused,
            },
            controller_id: facts.installation_id,
            firmware_version: self.firmware_version,
            target_mcu: TARGET_MCU,
            uptime_ms: self.clock.monotonic_ms().saturating_sub(self.started_ms),
            cluster: ControllerClusterBody { enabled: false },
            adapter_count: self.adapter_count,
            hydrated: true,
        }
    }
}

fn network_body(
    hostname: Option<String>,
    ipv4: Option<[u8; 4]>,
    mac: Option<[u8; 6]>,
) -> ControllerNetworkBody {
    ControllerNetworkBody {
        hostname,
        ip: ipv4.map(ipv4_text),
        mac: mac.map(mac_text),
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
