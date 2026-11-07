pub(crate) mod coalesce;
pub mod connection;
pub mod frame;
pub mod hub;
pub mod reader;
pub mod sink;
pub mod worker;

pub use hub::{
    close_code, origin_allowed, ClientId, RegisterRejected, WsHub, WsHubConfig, MAX_WS_CLIENTS,
    SENDER_BATCH_FRAMES,
};
pub use connection::{ReadError, ReadOutcome, WireError, WsConnection, WsWire};
pub use frame::{close_frame, FrameError, Opcode, MAX_CLIENT_FRAME_BYTES};
pub use reader::{serve_client_frames, ClientFrames, ReaderExit};
pub use sink::{WsSink, WsSinkError};
pub use worker::{spawn_ws_worker, WsWorkerPorts, NO_CLIENT_PARK};
