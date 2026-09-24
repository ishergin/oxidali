use std::collections::HashMap;
use std::sync::Arc;

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::json_stream_dto;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::stats_state::StatsHttpState;
use crate::http::types::HttpResponse;
use crate::http::handlers::common::{require_get};

declare_handler_shell!(StatsHandler {
    state: Arc<dyn StatsHttpState>,
});

impl ApiHandler for StatsHandler {
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
        json_stream_dto(self.state.stats_dto())
    }
}
