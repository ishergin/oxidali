use std::collections::HashMap;
use std::sync::Arc;

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::json_stream_dto;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;
use dali2rust_domain::registry::OperationReadPort;
use crate::http::handlers::common::{require_get};

declare_handler_shell! {
    OperationGetHandler {
        read: Arc<dyn OperationReadPort>,
    }
}

impl ApiHandler for OperationGetHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if let Err(error) = require_get(method) {
            return error;
        }
        let Some(id) = params.get("id") else {
            return HttpResponse::not_found();
        };
        let Some(view) = self.read.operation_view(id) else {
            return HttpResponse::not_found();
        };
        json_stream_dto(view)
    }
}

declare_handler_shell! {
    OperationsListHandler {
        read: Arc<dyn OperationReadPort>,
    }
}

impl ApiHandler for OperationsListHandler {
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
        let keys = self.read.list_operation_keys();
        json_stream_dto(serde_json::json!({ "operations": keys }))
    }
}
