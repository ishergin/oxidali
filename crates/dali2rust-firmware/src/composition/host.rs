use dali2rust_adapters::HardwareDisplay;
use dali2rust_api::http::router::Router;

use std::sync::Arc;

pub fn init_boot() -> (HardwareDisplay, String) {
    (HardwareDisplay::none(), String::new())
}

pub fn mount_http(_router: Arc<Router>, _ws_hub: Arc<dali2rust_adapters::WsHub>) {
    log::info!("HTTP server: not available on host target");
}
