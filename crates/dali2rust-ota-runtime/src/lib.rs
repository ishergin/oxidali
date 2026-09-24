pub mod runtime;

pub use runtime::boot_verify::spawn_boot_verifier;
pub use runtime::ota_worker::{
    spawn_ota_worker, OtaWorkerSeams, OTA_WORKER_HANDLED_COMMANDS, OTA_WORKER_REQUIRED_EVENTS,
};
pub use runtime::state::{OtaPhase, OtaSnapshot, OtaState};
