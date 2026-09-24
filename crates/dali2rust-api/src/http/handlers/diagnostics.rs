use std::collections::HashMap;
use std::sync::Arc;

use crate::http::diagnostics_state::DiagnosticsHttpState;
use crate::http::handler::ApiHandler;
use crate::http::handlers::common::json_stream_dto;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;
use crate::http::handlers::common::{require_get};

declare_handler_shell!(DiagnosticsHandler {
    state: Arc<dyn DiagnosticsHttpState>,
});

impl ApiHandler for DiagnosticsHandler {
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
        json_stream_dto(self.state.diagnostics_dto())
    }
}
