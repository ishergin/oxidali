use core::mem::MaybeUninit;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bsp::psram::PsramBox;
use dali2rust_bsp::stack_probe::StackLowWater;
use dali2rust_bsp::std_thread_stack;
use dali2rust_platform::cluster_link::{
    ClusterLink, ClusterReceiveBuffer, Received, CLUSTER_RECEIVE_BUFFER_BYTES,
};
use dali2rust_platform::liveness::monotonic_ms;

use super::counters::{bump, ClusterCounters};
use super::shared::ClusterPresence;

pub(crate) const RECEIVE_SLICE: Duration = Duration::from_millis(250);

pub(crate) static RECEIVER_STACK: StackLowWater =
    StackLowWater::new("cluster_rx", std_thread_stack::EVENT_WORKER_STACK);

pub(crate) enum LinkHandoff {
    Bound(Arc<dyn ClusterLink>),
    Unbound,
}

impl LinkHandoff {
    fn into_link(self) -> Option<Arc<dyn ClusterLink>> {
        match self {
            Self::Bound(link) => Some(link),
            Self::Unbound => None,
        }
    }
}

enum Next {
    Keep,
    Switch(LinkHandoff),
    Stop,
}

struct Intake {
    presence: Arc<ClusterPresence>,
    counters: Arc<ClusterCounters>,
    buffer: PsramBox<ClusterReceiveBuffer>,
}

pub(crate) fn run_receiver(
    handoff: Receiver<LinkHandoff>,
    presence: Arc<ClusterPresence>,
    counters: Arc<ClusterCounters>,
) {
    let mut intake = Intake { presence, counters, buffer: receive_buffer() };
    let mut link: Option<Arc<dyn ClusterLink>> = None;
    RECEIVER_STACK.note("start");
    loop {
        let next = match &link {
            None => handoff.recv().map_or(Next::Stop, Next::Switch),
            Some(bound) => intake.serve(bound.as_ref(), &handoff),
        };
        match next {
            Next::Keep => {}
            Next::Switch(handed) => link = handed.into_link(),
            Next::Stop => return,
        }
    }
}

impl Intake {
    fn serve(&mut self, link: &dyn ClusterLink, handoff: &Receiver<LinkHandoff>) -> Next {
        if self.receive_one(link) {
            return match handoff.try_recv() {
                Ok(handed) => Next::Switch(handed),
                Err(TryRecvError::Empty) => Next::Keep,
                Err(TryRecvError::Disconnected) => Next::Stop,
            };
        }
        match handoff.recv_timeout(RECEIVE_SLICE) {
            Ok(handed) => Next::Switch(handed),
            Err(RecvTimeoutError::Timeout) => Next::Keep,
            Err(RecvTimeoutError::Disconnected) => Next::Stop,
        }
    }

    fn receive_one(&mut self, link: &dyn ClusterLink) -> bool {
        let counters = &*self.counters;
        match link.receive(&mut self.buffer, RECEIVE_SLICE) {
            Ok(Received::Datagram { len, source }) => {
                bump(&counters.datagrams_received_total);
                let bytes = self.buffer.get(..len).unwrap_or_default();
                self.presence.take_datagram(bytes, source, counters, monotonic_ms());
                RECEIVER_STACK.note("datagram");
            }
            Ok(Received::Oversize { .. }) => {
                bump(&counters.datagrams_received_total);
                bump(&counters.rejected_frames_total);
            }
            Ok(Received::TimedOut) => {}
            Err(_) => {
                bump(&counters.receive_failures_total);
                return false;
            }
        }
        true
    }
}

fn receive_buffer() -> PsramBox<ClusterReceiveBuffer> {
    // SAFETY: `write_bytes` zeroes all `CLUSTER_RECEIVE_BUFFER_BYTES` bytes, and a zero byte is a valid `u8`.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<ClusterReceiveBuffer>| {
            place.as_mut_ptr().cast::<u8>().write_bytes(0, CLUSTER_RECEIVE_BUFFER_BYTES);
        })
    }
}
