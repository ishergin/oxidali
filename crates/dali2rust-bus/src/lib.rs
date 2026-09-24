#![cfg_attr(not(feature = "std"), no_std)]

mod channels;
mod correlation;
mod counters;
mod frame;
mod publish;
pub mod worker_counters;

#[cfg(feature = "std")]
mod backends;

#[cfg(feature = "std")]
mod bus;

#[cfg(feature = "std")]
mod publish_or_drop;

#[cfg(feature = "std")]
mod publish_required;

#[cfg(feature = "std")]
mod worker_loop;

pub use channels::{Receiver, RecvError, RecvTimeoutError, Sender, TryRecvError};
pub use correlation::CorrelationIdAllocator;
pub use counters::{BusCounters, ChannelCounters, SubscriberCounters};
pub use frame::{BusFrame, MAX_BUS_FRAME_BYTES};
pub use publish::PublishResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BusId(pub u16);

impl Default for BusId {
    fn default() -> Self {
        Self(1)
    }
}

impl From<u16> for BusId {
    fn from(v: u16) -> Self {
        Self(v)
    }
}

impl From<BusId> for u16 {
    fn from(id: BusId) -> u16 {
        id.0
    }
}

#[cfg(feature = "std")]
pub use bus::{
    BusChannel, BusConfig, BusHost, BusPublisher, BusRegistrar, BusSubscriberRx, BusTaskHandle,
    BusTaskStats, CommandArrivalObserver, EventInboxProbe,
    DEFAULT_CONFIRMATION_SLOTS,
};

#[cfg(feature = "std")]
pub use publish_or_drop::publish_or_drop;

#[cfg(feature = "std")]
pub use worker_loop::{recv_then_drain, WorkerTurn};

#[cfg(feature = "std")]
pub use publish_required::{
    publish_required, publish_required_counted, RequiredPublishCounters, RequiredPublishOutcome,
    HANDLER_PUBLISH_BACKOFF_MS, REQUIRED_PUBLISH_BACKOFF_MS, REQUIRED_PUBLISH_UNCAPPED,
};
