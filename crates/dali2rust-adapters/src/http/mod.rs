pub use dali2rust_api::http::router::Router;

mod wire_method;

#[cfg(not(target_os = "espidf"))]
pub mod host;

#[cfg(not(target_os = "espidf"))]
pub mod host_ws;

#[cfg(target_os = "espidf")]
pub mod esp_idf;

#[cfg(target_os = "espidf")]
pub mod esp_ws;

#[cfg(target_os = "espidf")]
mod esp_ws_send;
