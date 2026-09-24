use std::collections::HashMap;
use std::sync::Arc;

use super::types::HttpResponse;

pub trait ApiHandler: Send + Sync {
    fn handle_request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse;
}

pub struct SharedApiHandler {
    inner: Arc<dyn ApiHandler>,
}

impl SharedApiHandler {
    pub fn new(inner: Arc<dyn ApiHandler>) -> Self {
        Self { inner }
    }
}

impl ApiHandler for SharedApiHandler {
    fn handle_request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        self.inner.handle_request(method, path, body, params)
    }
}
