use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::RegistrySliceReloadCommand;
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{json_err, json_stream_dto, require_get};
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::{HttpBody, HttpResponse};

pub trait ConfigTransferPort: Send + Sync {
    fn slice_manifest(&self) -> Vec<SliceManifestEntry>;
    fn export_slice(&self, name: &str) -> Option<Vec<u8>>;
    fn import_slice(&self, name: &str, bytes: &[u8]) -> Result<(), ImportRefusal>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportRefusal {
    UnknownSlice,
    PersistenceDisabled,
    StoreFailed(String),
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SliceManifestEntry {
    pub name: String,
    pub bytes: Option<usize>,
    pub crc32: u32,
}

declare_handler_shell! {
    ConfigManifestHandler {
        transfer: Arc<dyn ConfigTransferPort>,
    }
}

impl ApiHandler for ConfigManifestHandler {
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
        json_stream_dto(self.transfer.slice_manifest())
    }
}

declare_handler_shell! {
    ConfigSliceHandler {
        transfer: Arc<dyn ConfigTransferPort>,
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl ConfigSliceHandler {
    fn export(&self, name: &str) -> HttpResponse {
        match self.transfer.export_slice(name) {
            Some(bytes) => HttpResponse {
                status: 200,
                content_type: "application/octet-stream",
                extra_headers: crate::http::types::NO_EXTRA_HEADERS,
                body: HttpBody::Buffered(bytes),
            },
            None => json_err(404, "not_found"),
        }
    }

    fn import(&self, name: &str, body: &[u8]) -> HttpResponse {
        if body.is_empty() {
            return json_err(422, "empty_slice");
        }
        if let Err(refusal) = self.transfer.import_slice(name, body) {
            return match refusal {
                ImportRefusal::UnknownSlice => json_err(404, "not_found"),
                ImportRefusal::PersistenceDisabled => json_err(409, "persistence_disabled"),
                ImportRefusal::StoreFailed(_) => json_err(503, "store_failed"),
            };
        }
        self.reload(name)
    }

    fn reload(&self, name: &str) -> HttpResponse {
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            None,
            RegistrySliceReloadCommand {
                slice_name: dali2rust_contracts::msg::fixed_text_32(name),
            },
        );
        match crate::http::dispatcher::dispatch_and_wait_for_success(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
        ) {
            Ok(()) => json_stream_dto(self.transfer.slice_manifest()),
            Err(error) => error,
        }
    }
}

impl ApiHandler for ConfigSliceHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let Some(name) = params.get("slice") else {
            return json_err(400, "invalid_resource_id");
        };
        match method {
            "GET" => self.export(name),
            "PUT" => self.import(name, body),
            _ => json_err(405, "method_not_allowed"),
        }
    }
}
