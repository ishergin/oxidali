#![allow(
    clippy::too_many_arguments,
    reason = "Worker dispatch arms mirror the routed bus command surface."
)]

pub mod runtime;

pub use runtime::priority::{wire_priority, yield_granularity, WireArrivalObserver};
pub use runtime::clock::StdClock;
pub use runtime::config::{ContentConfirmPolicy, DaliRuntimeConfig};
pub use runtime::controller::DaliController;
pub use runtime::dali_worker::{
    spawn_dali_worker, DaliWorkerCounters, DALI_WORKER_HANDLED_COMMANDS,
    DALI_WORKER_REQUIRED_EVENTS,
};
pub use runtime::dali_worker::dev103_patch_mask;

pub mod dev103_feedback_mask {
    pub use crate::runtime::executor::dev103_feedback::{
        FB_PATCH_ACTIVE_BRIGHTNESS, FB_PATCH_ACTIVE_COLOUR, FB_PATCH_INACTIVE_BRIGHTNESS,
        FB_PATCH_INACTIVE_COLOUR, FB_PATCH_TIMING,
    };
}
