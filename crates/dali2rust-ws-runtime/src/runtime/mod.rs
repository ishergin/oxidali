pub(crate) mod coalesce;
pub mod handoff;
pub mod hub;
pub mod sink;
pub mod worker;

pub use handoff::{HandoffOutcome, SendSlot, SlotPool};
pub use hub::{
    close_code, origin_allowed, ClientId, RegisterRejected, WsHub, WsHubConfig, WsSessions, MAX_WS_CLIENTS,
    WS_SENDER_SLOTS,
};
pub use sink::{WsSink, WsSinkError};
pub use worker::{spawn_ws_worker, WsWorkerPorts, NO_CLIENT_PARK};
