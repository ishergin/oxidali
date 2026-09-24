use std::convert::Infallible;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Instant;

use dali2rust_dali_phy::{PHY_TICK_US, TX_HALF_BIT_TICKS};
use dali2rust_gear_model::GearFleet;
use dali2rust_platform::dali::{
    DaliTransport, DaliWireCounters, ObservedFrameSender, ObservedRawFrame,
    ObservedRawFrameKind, TransferOutcome, WireLoadWindow,
};

use super::WIRE_LOAD_WINDOW_TICKS;

pub use dali2rust_gear_model::{GearSpec, DEFAULT_RESERVED_SHORT_ADDRESSES};

const HOST_RNG_SEED: u32 = 0x00C0_FFEE;

const FORWARD16_TICKS: u32 = 38 * TX_HALF_BIT_TICKS;
const FORWARD24_TICKS: u32 = 54 * TX_HALF_BIT_TICKS;
const BACKWARD8_TICKS: u32 = 22 * TX_HALF_BIT_TICKS;

#[derive(Debug)]
pub struct SimDaliTransport {
    fleet: GearFleet,
    input_fleet: dali2rust_gear_model::input_device::DeviceFleet,
    sent_frames: Vec<u16>,
    sent_frame_min_idle_us: Vec<u32>,
    sent_frame24_min_idle_us: Vec<u32>,
    sent_frame_times: Vec<Instant>,
    observed_tx: Option<ObservedFrameSender>,
    wire_counters: Option<Arc<DaliWireCounters>>,
    wire_load: WireLoadWindow,
    born: Instant,
    active_ticks: u32,
    tx_ticks: u32,
}

impl SimDaliTransport {
    pub fn new(specs: Vec<GearSpec>) -> Self {
        Self {
            fleet: GearFleet::new(specs, 0, HOST_RNG_SEED),
            input_fleet: dali2rust_gear_model::input_device::DeviceFleet::new(Vec::new()),
            observed_tx: None,
            sent_frames: Vec::new(),
            sent_frame_min_idle_us: Vec::new(),
            sent_frame24_min_idle_us: Vec::new(),
            sent_frame_times: Vec::new(),
            wire_counters: None,
            wire_load: WireLoadWindow::new(WIRE_LOAD_WINDOW_TICKS),
            born: Instant::now(),
            active_ticks: 0,
            tx_ticks: 0,
        }
    }

    pub fn demo_bus() -> Self {
        Self {
            fleet: GearFleet::demo_bus(),
            input_fleet: demo_input_devices(),
            observed_tx: None,
            sent_frames: Vec::new(),
            sent_frame_min_idle_us: Vec::new(),
            sent_frame24_min_idle_us: Vec::new(),
            sent_frame_times: Vec::new(),
            wire_counters: None,
            wire_load: WireLoadWindow::new(WIRE_LOAD_WINDOW_TICKS),
            born: Instant::now(),
            active_ticks: 0,
            tx_ticks: 0,
        }
    }

    pub fn input_fleet_mut(&mut self) -> &mut dali2rust_gear_model::input_device::DeviceFleet {
        &mut self.input_fleet
    }

    pub fn fleet(&self) -> &GearFleet {
        &self.fleet
    }

    pub fn fleet_mut(&mut self) -> &mut GearFleet {
        &mut self.fleet
    }

    pub fn sent_frames(&self) -> Vec<u16> {
        self.sent_frames.clone()
    }

    pub fn sent_frame_settle_us(&self) -> Vec<u32> {
        self.sent_frame_min_idle_us.clone()
    }

    pub fn sent_frame24_settle_us(&self) -> Vec<u32> {
        self.sent_frame24_min_idle_us.clone()
    }

    pub fn forward_frame_gap_ms(&self, a: usize, b: usize) -> Option<u128> {
        let first = self.sent_frame_times.get(a)?;
        let second = self.sent_frame_times.get(b)?;
        Some(second.saturating_duration_since(*first).as_millis())
    }

    pub fn inject_observed_frame(&mut self, bytes: [u8; 3], kind: ObservedRawFrameKind) -> bool {
        let ticks = match kind {
            ObservedRawFrameKind::Forward16 => FORWARD16_TICKS,
            ObservedRawFrameKind::Forward24 => FORWARD24_TICKS,
            ObservedRawFrameKind::Backward8 => BACKWARD8_TICKS,
        };
        self.charge_foreign_wire(ticks);
        let Some(tx) = self.observed_tx.as_ref() else {
            return false;
        };
        tx.try_send(ObservedRawFrame {
            bytes,
            kind,
            observed_at_ms: dali2rust_bsp::unix_clock::unix_wall_clock_millis(),
            observed_at_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms(),
        })
        .is_ok()
    }

    pub fn inject_foreign_frame(&mut self, frame: u16) -> bool {
        let bytes = [(frame >> 8) as u8, (frame & 0xFF) as u8, 0];
        self.inject_observed_frame(bytes, ObservedRawFrameKind::Forward16)
    }

    pub fn press_button(&mut self, device: usize, instance: usize, pressed: bool) -> usize {
        let frames = self.input_fleet.button_edge(device, instance, pressed);
        self.inject_all(&frames)
    }

    pub fn occupancy_transition(
        &mut self,
        device: usize,
        instance: usize,
        input_value: u8,
        event_info: u16,
    ) -> usize {
        let frames = self
            .input_fleet
            .occupancy_transition(device, instance, input_value, event_info);
        self.inject_all(&frames)
    }

    fn inject_all(&mut self, frames: &[[u8; 3]]) -> usize {
        frames
            .iter()
            .filter(|bytes| self.inject_observed_frame(**bytes, ObservedRawFrameKind::Forward24))
            .count()
    }

    fn charge_foreign_wire(&mut self, ticks: u32) {
        self.active_ticks = self.active_ticks.wrapping_add(ticks);
        self.publish_wire();
    }

    fn record_and_exchange(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> TransferOutcome {
        self.sent_frames.push(frame);
        self.sent_frame_min_idle_us.push(min_idle_us);
        self.sent_frame_times.push(Instant::now());
        let outcome = self.fleet.exchange(frame, expects_backward);
        self.charge_wire(FORWARD16_TICKS, &outcome);
        outcome
    }

    fn charge_wire(&mut self, forward_ticks: u32, outcome: &TransferOutcome) {
        let answered = matches!(outcome, TransferOutcome::Answer(_));
        let ticks = forward_ticks + if answered { BACKWARD8_TICKS } else { 0 };
        self.active_ticks = self.active_ticks.wrapping_add(ticks);
        self.tx_ticks = self.tx_ticks.wrapping_add(forward_ticks);
        self.publish_wire();
    }

    fn publish_wire(&mut self) {
        let Some(w) = self.wire_counters.as_ref() else {
            return;
        };
        let total = elapsed_ticks(self.born);
        w.wire_ticks_total.store(total, Relaxed);
        w.wire_ticks_active.store(self.active_ticks, Relaxed);
        w.wire_ticks_tx.store(self.tx_ticks, Relaxed);
        if let Some(load) = self
            .wire_load
            .sample(total, self.active_ticks, self.tx_ticks)
        {
            w.load_permille.store(u32::from(load.permille), Relaxed);
            w.load_own_permille
                .store(u32::from(load.own_permille), Relaxed);
        }
    }
}

fn elapsed_ticks(since: Instant) -> u32 {
    (since.elapsed().as_micros() / u128::from(PHY_TICK_US)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_button_press_reaches_the_observed_channel() {
        let mut sim = SimDaliTransport::demo_bus();
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        sim.set_observed_frame_sender(tx);

        let sent = sim.press_button(0, 3, true);

        assert_eq!(sent, 1, "the demo panel's button instance emits one event");
        let frame = rx.try_recv().expect("the event reaches the translator");
        assert_eq!(frame.kind, ObservedRawFrameKind::Forward24);
        assert_ne!(frame.bytes, [0, 0, 0]);
        assert_eq!(sim.press_button(0, 3, false), 1, "and so does the release");
    }

    #[test]
    fn an_injection_with_no_listener_says_so() {
        let mut sim = SimDaliTransport::demo_bus();

        assert!(!sim.inject_foreign_frame(0xFE80));
        assert_eq!(sim.press_button(0, 0, true), 0);
    }

    #[test]
    fn foreign_traffic_is_active_wire_but_not_our_own() {
        let mut sim = SimDaliTransport::demo_bus();
        let counters = Arc::new(DaliWireCounters::default());
        sim.set_wire_counters(Arc::clone(&counters));
        let (tx, _rx) = std::sync::mpsc::sync_channel(8);
        sim.set_observed_frame_sender(tx);

        assert!(sim.inject_foreign_frame(0xFE80));

        assert_eq!(counters.wire_ticks_active.load(Relaxed), FORWARD16_TICKS);
        assert_eq!(counters.wire_ticks_tx.load(Relaxed), 0,
                   "another master's frame must never count as ours");
    }

    #[test]
    fn exchanges_charge_the_simulated_wire() {
        let mut sim = SimDaliTransport::demo_bus();
        let counters = Arc::new(DaliWireCounters::default());
        sim.set_wire_counters(Arc::clone(&counters));

        let _ = sim.exchange_frame(0xFE80, false);
        assert_eq!(counters.wire_ticks_tx.load(Relaxed), FORWARD16_TICKS);
        assert_eq!(counters.wire_ticks_active.load(Relaxed), FORWARD16_TICKS);

        let outcome = sim.exchange_frame(0x0190, true);
        assert!(matches!(outcome, Ok(TransferOutcome::Answer(_))), "{outcome:?}");
        assert_eq!(counters.wire_ticks_tx.load(Relaxed), 2 * FORWARD16_TICKS);
        assert_eq!(
            counters.wire_ticks_active.load(Relaxed),
            2 * FORWARD16_TICKS + BACKWARD8_TICKS
        );

        let _ = sim.exchange_frame24([0xFF, 0xFE, 0x00], false);
        assert_eq!(
            counters.wire_ticks_tx.load(Relaxed),
            2 * FORWARD16_TICKS + FORWARD24_TICKS
        );
    }
}

fn demo_input_devices() -> dali2rust_gear_model::input_device::DeviceFleet {
    use dali2rust_gear_model::input_device::{
        DeviceFleet, FeedbackDialect, FeedbackSpec, InputDeviceSpec, InstanceSpec,
    };
    let button = |dialect| InstanceSpec {
        instance_type: 1,
        feedback: Some(FeedbackSpec {
            dialect,
            capability: 0x07,
            colour_capability: 0x1F,
        }),
    };
    let mut fleet = DeviceFleet::new(vec![
        InputDeviceSpec {
            short_address: Some(0),
            random_address: 0x0A11CE,
            instances: (0..8).map(|_| button(FeedbackDialect::DiiaCorrected)).collect(),
        },
        InputDeviceSpec {
            short_address: Some(1),
            random_address: 0x0B0B00,
            instances: (0..4).map(|_| button(FeedbackDialect::Ed1)).collect(),
        },
    ]);
    for (device, instances) in [(0usize, 8usize), (1, 4)] {
        for instance in 0..instances {
            fleet.commission_instance(device, instance, EVENT_SCHEME_DEVICE_INSTANCE,
                                      EVENT_FILTER_BUTTON_EDGES);
        }
    }
    fleet
}

const EVENT_SCHEME_DEVICE_INSTANCE: u8 = 2;
const EVENT_FILTER_BUTTON_EDGES: u8 = 0x03;

impl DaliTransport for SimDaliTransport {
    type Error = Infallible;

    fn exchange_frame24(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
    ) -> Result<
        dali2rust_platform::dali::TransferOutcome,
        dali2rust_platform::dali::Frame24Error<Self::Error>,
    > {
        self.exchange_frame24_with_settle(frame, expects_backward, 0)
    }

    fn exchange_frame24_with_settle(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<
        dali2rust_platform::dali::TransferOutcome,
        dali2rust_platform::dali::Frame24Error<Self::Error>,
    > {
        self.sent_frame24_min_idle_us.push(min_idle_us);
        let outcome = self.input_fleet.exchange24(frame, expects_backward);
        self.charge_wire(FORWARD24_TICKS, &outcome);
        Ok(outcome)
    }

    fn supports_frame24(&self) -> bool {
        true
    }

    fn set_wire_counters(&mut self, sink: Arc<DaliWireCounters>) {
        self.wire_counters = Some(sink);
    }

    fn set_observed_frame_sender(&mut self, sender: ObservedFrameSender) {
        self.observed_tx = Some(sender);
    }

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        let _ = self.record_and_exchange(frame, false, 0);
        Ok(())
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        Ok(None)
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        Ok(true)
    }

    fn exchange_frame(
        &mut self,
        frame: u16,
        expects_backward: bool,
    ) -> Result<TransferOutcome, Self::Error> {
        Ok(self.record_and_exchange(frame, expects_backward, 0))
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        Ok(self.record_and_exchange(frame, expects_backward, min_idle_us))
    }

    fn honours_settle(&self) -> bool {
        false
    }
}
