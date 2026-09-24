#![allow(
    clippy::derivable_impls,
    clippy::field_reassign_with_default,
    clippy::manual_contains,
    clippy::needless_borrow,
    clippy::new_without_default,
    clippy::too_many_arguments,
    clippy::unnecessary_map_or,
    clippy::unwrap_or_default,
    reason = "Adapter composition, persistence codecs, and runtime workers intentionally keep explicit wiring."
)]

pub mod dali;
pub mod display;
pub mod http;
pub mod ota;
pub mod log;
pub mod runtime;

pub use runtime::{
    build_http_test_stack, build_router_with_bus_and_transport, BusStackRuntime, StackOptions,
    WsHub,
};
pub use dali2rust_api::http::handlers::static_assets::StaticAsset;
pub use dali2rust_display_runtime::{DisplayView, HardwareDisplay};
pub use dali2rust_dali_runtime::{ContentConfirmPolicy, DaliRuntimeConfig};
