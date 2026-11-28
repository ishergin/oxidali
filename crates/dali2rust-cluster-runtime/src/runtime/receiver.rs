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
use super::incoming::CommandIntake;
use super::input_intake::InputIntake;
use super::shared::{Arrived, Caches, ClusterPresence, Delivery};
use crate::info::ClusterInfo;
use crate::inputs::ClusterInputs;
use crate::lamps::ClusterLamps;

pub(crate) const RECEIVE_SLICE: Duration = Duration::from_millis(250);

pub(crate) static RECEIVER_STACK: StackLowWater =
    StackLowWater::new("cluster_rx", std_thread_stack::EVENT_WORKER_STACK);

pub(crate) enum LinkHandoff {
    Bound { link: Arc<dyn ClusterLink>, generation: u32 },
    Unbound,
}

struct HeldLink {
    link: Arc<dyn ClusterLink>,
    generation: u32,
}

impl LinkHandoff {
    fn into_link(self) -> Option<HeldLink> {
        match self {
            Self::Bound { link, generation } => Some(HeldLink { link, generation }),
            Self::Unbound => None,
        }
    }
}

enum Next {
    Keep,
    Switch(LinkHandoff),
    Stop,
}

pub(crate) struct Shared {
    pub(crate) presence: Arc<ClusterPresence>,
    pub(crate) info: Arc<ClusterInfo>,
    pub(crate) lamps: Arc<ClusterLamps>,
    pub(crate) inputs: Arc<ClusterInputs>,
    pub(crate) counters: Arc<ClusterCounters>,
}

pub(crate) struct Intakes {
    pub(crate) commands: CommandIntake,
    pub(crate) inputs: InputIntake,
}

struct Intake {
    shared: Shared,
    commands: CommandIntake,
    inputs: InputIntake,
    buffer: PsramBox<ClusterReceiveBuffer>,
}

pub(crate) fn run_receiver(handoff: Receiver<LinkHandoff>, shared: Shared, intakes: Intakes) {
    let Intakes { commands, inputs } = intakes;
    let mut intake = Intake { shared, commands, inputs, buffer: receive_buffer() };
    let mut link: Option<HeldLink> = None;
    RECEIVER_STACK.note("start");
    loop {
        let next = match &link {
            None => idle(&handoff),
            Some(held) => intake.serve(held, &handoff),
        };
        intake.commands.drain_confirmations();
        match next {
            Next::Keep => {}
            Next::Switch(handed) => link = handed.into_link(),
            Next::Stop => return,
        }
    }
}

fn idle(handoff: &Receiver<LinkHandoff>) -> Next {
    match handoff.recv_timeout(RECEIVE_SLICE) {
        Ok(handed) => Next::Switch(handed),
        Err(RecvTimeoutError::Timeout) => Next::Keep,
        Err(RecvTimeoutError::Disconnected) => Next::Stop,
    }
}

impl Intake {
    fn serve(&mut self, held: &HeldLink, handoff: &Receiver<LinkHandoff>) -> Next {
        if self.receive_one(held) {
            return match handoff.try_recv() {
                Ok(handed) => Next::Switch(handed),
                Err(TryRecvError::Empty) => Next::Keep,
                Err(TryRecvError::Disconnected) => Next::Stop,
            };
        }
        idle(handoff)
    }

    fn receive_one(&mut self, held: &HeldLink) -> bool {
        let counters = &*self.shared.counters;
        match held.link.receive(&mut self.buffer, RECEIVE_SLICE) {
            Ok(Received::Datagram { len, source }) => {
                bump(&counters.datagrams_received_total);
                let delivery = Delivery { source, link: held.generation, now_ms: monotonic_ms() };
                let bytes = self.buffer.get(..len).unwrap_or_default();
                let caches = Caches { info: &self.shared.info, lamps: &self.shared.lamps, inputs: &self.shared.inputs };
                match self.shared.presence.take_datagram(bytes, delivery, counters, caches) {
                    Some(Arrived::Command(heard)) => self.commands.take(heard, counters),
                    Some(Arrived::Input(heard)) => self.inputs.take(heard, counters),
                    None => {}
                }
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
