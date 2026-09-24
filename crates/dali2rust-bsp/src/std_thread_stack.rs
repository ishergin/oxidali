#[cfg(target_os = "espidf")]
mod sizes {
    pub const COMMAND_WORKER_STACK: usize = 12 * 1024;

    pub const EVENT_WORKER_STACK: usize = 8 * 1024;

    pub const RING_CONSUMER_STACK: usize = 8 * 1024;

    pub const STATE_WORKER_STACK: usize = 10 * 1024;

    pub const OTA_LISTENER_STACK: usize = 3 * 1024;

    pub const ARBITRATION_SUPERVISOR_STACK: usize = 3 * 1024;

    pub const ARBITRATION_WORKER_STACK: usize = 4 * 1024;

    pub const CENSUS_STACK: usize = 3 * 1024;

    pub const OTA_UPDATE_STACK: usize = 16 * 1024;

    pub const DISPLAY_WORKER: usize = 6 * 1024;

    pub const HYDRATION_WORKER: usize = 48 * 1024;
}

#[cfg(not(target_os = "espidf"))]
mod sizes {
    pub const COMMAND_WORKER_STACK: usize = 0;
    pub const EVENT_WORKER_STACK: usize = 0;
    pub const OTA_LISTENER_STACK: usize = 0;
    pub const ARBITRATION_SUPERVISOR_STACK: usize = 0;
    pub const ARBITRATION_WORKER_STACK: usize = 0;
    pub const CENSUS_STACK: usize = 0;
    pub const OTA_UPDATE_STACK: usize = 0;
    pub const STATE_WORKER_STACK: usize = 0;
    pub const DISPLAY_WORKER: usize = 0;
    pub const HYDRATION_WORKER: usize = 0;
}

pub use sizes::*;
