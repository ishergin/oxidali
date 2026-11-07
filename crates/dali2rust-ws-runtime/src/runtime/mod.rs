pub(crate) mod coalesce;
pub mod connection;
pub mod frame;
pub mod handoff;
pub mod hub;
pub mod reader;
pub mod sink;
pub mod worker;

pub use handoff::{HandoffOutcome, SendSlot, SlotPool};
pub use hub::{
    close_code, origin_allowed, ClientId, RegisterRejected, WsHub, WsHubConfig, WsSessions, MAX_WS_CLIENTS,
    SENDER_BATCH_FRAMES,
};
pub use sink::{WsSink, WsSinkError};
pub use worker::{spawn_ws_worker, WsWorkerPorts, NO_CLIENT_PARK};
