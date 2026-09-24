#![allow(
    clippy::manual_contains,
    clippy::needless_as_bytes,
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "HTTP and bus_codec surfaces carry many parameters and stable wiring patterns."
)]

pub mod bus_codec;
pub mod coalesce;
pub mod confirmation_bridge;
pub mod constants;
pub mod contracts;
pub mod ha;
pub mod http;
pub mod ws;
