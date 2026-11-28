use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dali2rust_platform::arbitration::AnswerInputs;
use dali2rust_platform::bus_unit::BusUnitIdentity;
use dali2rust_platform::dali::{
    DaliTransport, DaliWireCounters, Frame24Error, ObservedFrameSender, PhySnifferCounters, TransferOutcome,
};

pub type Ear<T> = fn(&mut T, u16) -> bool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    Tap(usize),
    Foreign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireFrame {
    pub speaker: Speaker,
    pub frame: u16,
}

pub struct SharedWire<T> {
    taps: Mutex<Vec<Arc<Mutex<T>>>>,
    frames: Mutex<Vec<WireFrame>>,
    ear: Ear<T>,
}

fn locked<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<T> SharedWire<T> {
    pub fn new(ear: Ear<T>) -> Arc<Self> {
        Arc::new(Self { taps: Mutex::new(Vec::new()), frames: Mutex::new(Vec::new()), ear })
    }

    pub fn tap(self: &Arc<Self>, transport: T) -> WireTap<T> {
        let transport = Arc::new(Mutex::new(transport));
        let mut taps = locked(&self.taps);
        let index = taps.len();
        taps.push(Arc::clone(&transport));
        WireTap { wire: Arc::clone(self), index, transport }
    }

    pub fn foreign(&self, frame: u16) -> usize {
        self.carry(Speaker::Foreign, frame)
    }

    pub fn frames(&self) -> Vec<WireFrame> {
        locked(&self.frames).clone()
    }

    pub fn sent_by(&self, tap: usize) -> Vec<u16> {
        let frames = locked(&self.frames);
        frames.iter().filter(|heard| heard.speaker == Speaker::Tap(tap)).map(|heard| heard.frame).collect()
    }

    fn carry(&self, speaker: Speaker, frame: u16) -> usize {
        locked(&self.frames).push(WireFrame { speaker, frame });
        let listeners: Vec<Arc<Mutex<T>>> = {
            let taps = locked(&self.taps);
            taps.iter().enumerate().filter(|(index, _)| speaker != Speaker::Tap(*index)).map(|(_, tap)| Arc::clone(tap)).collect()
        };
        listeners.iter().filter(|listener| (self.ear)(&mut locked(listener), frame)).count()
    }
}

pub struct WireTap<T> {
    wire: Arc<SharedWire<T>>,
    index: usize,
    transport: Arc<Mutex<T>>,
}

impl<T> WireTap<T> {
    pub const fn index(&self) -> usize {
        self.index
    }

    pub fn transport(&self) -> Arc<Mutex<T>> {
        Arc::clone(&self.transport)
    }

    fn inner(&self) -> MutexGuard<'_, T> {
        locked(&self.transport)
    }

    fn spoke<R>(&self, frame: u16, result: R) -> R {
        self.wire.carry(Speaker::Tap(self.index), frame);
        result
    }
}

impl<T: DaliTransport> DaliTransport for WireTap<T> {
    type Error = T::Error;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        let sent = self.inner().send_forward_frame(frame);
        self.spoke(frame, sent)
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        self.inner().receive_backward_frame()
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        self.inner().is_bus_idle()
    }

    fn exchange_frame(&mut self, frame: u16, expects_backward: bool) -> Result<TransferOutcome, Self::Error> {
        let outcome = self.inner().exchange_frame(frame, expects_backward);
        self.spoke(frame, outcome)
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        let outcome = self.inner().exchange_frame_with_settle(frame, expects_backward, min_idle_us);
        self.spoke(frame, outcome)
    }

    fn exchange_frame24(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
    ) -> Result<TransferOutcome, Frame24Error<Self::Error>> {
        self.inner().exchange_frame24(frame, expects_backward)
    }

    fn exchange_frame24_with_settle(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Frame24Error<Self::Error>> {
        self.inner().exchange_frame24_with_settle(frame, expects_backward, min_idle_us)
    }

    fn supports_frame24(&self) -> bool {
        self.inner().supports_frame24()
    }

    fn foreign_activity(&self) -> u32 {
        self.inner().foreign_activity()
    }

    fn honours_settle(&self) -> bool {
        self.inner().honours_settle()
    }

    fn set_observed_frame_sender(&mut self, sender: ObservedFrameSender) {
        self.inner().set_observed_frame_sender(sender);
    }

    fn set_sniffer_counters(&mut self, sink: Arc<PhySnifferCounters>) {
        self.inner().set_sniffer_counters(sink);
    }

    fn set_wire_counters(&mut self, sink: Arc<DaliWireCounters>) {
        self.inner().set_wire_counters(sink);
    }

    fn set_answer_inputs(&mut self, inputs: Arc<AnswerInputs>) {
        self.inner().set_answer_inputs(inputs);
    }

    fn set_bus_unit_identity(&mut self, identity: BusUnitIdentity) {
        self.inner().set_bus_unit_identity(identity);
    }
}

#[cfg(test)]
mod tests;
