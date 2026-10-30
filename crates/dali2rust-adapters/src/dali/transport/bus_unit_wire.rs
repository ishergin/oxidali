use std::hash::{BuildHasher, RandomState};
use std::sync::Arc;

use dali2rust_dali_codec::rx_decode::SniffedFrame;
use dali2rust_dali_phy::PHY_TICK_US;
use dali2rust_domain::dali::dev103::bus_unit::{Device103Unit, UnitFrame};
use dali2rust_platform::answer_table::AnswerTableImage;
use dali2rust_platform::arbitration::AnswerInputs;
use dali2rust_platform::bus_unit::{BusUnitEvidence, BusUnitIdentity, BusUnitNvm};
use dali2rust_platform::dali::{
    ObservedFrameSender, ObservedItem, ObservedRawFrame, ObservedRawFrameKind,
};

use super::MICROS_PER_MILLI;

pub const BUS_UNIT_RESEND_MS: u32 = 1_000;
pub const HARDWARE_VERSION: [u8; 2] = [1, 0];
const MAC_AT: usize = 2;

#[derive(Debug)]
pub struct BusUnitWire {
    unit: Device103Unit,
    adopted: Option<BusUnitNvm>,
    unacknowledged: Option<BusUnitEvidence>,
    reported_at_ms: u32,
    entropy: fn() -> u32,
}

impl BusUnitWire {
    #[must_use]
    pub fn new(identity: &BusUnitIdentity, entropy: fn() -> u32) -> Self {
        Self {
            unit: Device103Unit::new(identity),
            adopted: None,
            unacknowledged: None,
            reported_at_ms: 0,
            entropy,
        }
    }

    // IEC 62386-103 §9.2.3
    pub fn observe(&mut self, frame: UnitFrame, at_ms: u32) -> Option<BusUnitEvidence> {
        let entropy = self.entropy;
        if !self.unit.feed(frame, at_ms, &mut || entropy()).nvm_changed {
            return None;
        }
        self.unacknowledged = self.evidence();
        self.reported_at_ms = at_ms;
        self.unacknowledged
    }

    pub fn poll(&mut self, adopt: Option<&AnswerInputs>, now_ms: u32) -> Option<BusUnitEvidence> {
        if let Some(inputs) = adopt {
            self.hydrate(inputs);
        }
        self.unit.tick(now_ms);
        let due = now_ms.wrapping_sub(self.reported_at_ms) >= BUS_UNIT_RESEND_MS;
        let resend = self.unacknowledged.filter(|_| due)?;
        self.reported_at_ms = now_ms;
        Some(resend)
    }

    pub fn lost_capture(&mut self) {
        self.unit.lost_capture();
    }

    #[must_use]
    pub fn answer_table(&self) -> Option<AnswerTableImage> {
        self.unit.is_hydrated().then(|| self.unit.answer_table())
    }

    fn hydrate(&mut self, inputs: &AnswerInputs) {
        let Ok(Some(snapshot)) = inputs.try_bus_unit() else {
            return;
        };
        if snapshot.acknowledged_wire_sequence == self.unit.wire_sequence() {
            self.unacknowledged = None;
        }
        if self.unit.hydrate(&snapshot) {
            self.adopted = Some(self.unit.nvm());
        }
    }

    fn evidence(&self) -> Option<BusUnitEvidence> {
        Some(BusUnitEvidence {
            adopted: self.adopted?,
            adopted_revision: self.unit.adopted_revision()?,
            observed: self.unit.nvm(),
            wire_sequence: self.unit.wire_sequence(),
        })
    }
}

// IEC 62386-103 Table 18
#[must_use]
pub fn controller_identity(version: &str, hardware_address: Option<[u8; 6]>) -> BusUnitIdentity {
    let mut parts = version.split(['.', '+']).map(|part| part.parse::<u8>().unwrap_or(0));
    let firmware_version = [parts.next().unwrap_or(0), parts.next().unwrap_or(0)];
    let mut identification_number = [0; 8];
    if let Some(mac) = hardware_address {
        identification_number[MAC_AT..].copy_from_slice(&mac);
    }
    BusUnitIdentity { firmware_version, hardware_version: HARDWARE_VERSION, identification_number }
}

#[must_use]
pub const fn unit_frame(kind: ObservedRawFrameKind, bytes: [u8; 3]) -> UnitFrame {
    match kind {
        ObservedRawFrameKind::Forward24 => UnitFrame::Forward24(bytes),
        ObservedRawFrameKind::Forward16 | ObservedRawFrameKind::Backward8 => UnitFrame::Other,
    }
}

#[must_use]
pub const fn sniffed_unit_frame(frame: SniffedFrame) -> UnitFrame {
    match frame {
        SniffedFrame::Forward24(bytes) => UnitFrame::Forward24(bytes),
        SniffedFrame::Forward16(_)
        | SniffedFrame::Backward8(_)
        | SniffedFrame::UnsupportedLength(_)
        | SniffedFrame::DecodeFailed => UnitFrame::Other,
    }
}

pub fn host_entropy() -> u32 {
    let hash = RandomState::new().hash_one(0u8);
    u32::try_from(hash & u64::from(u32::MAX)).unwrap_or_default()
}

#[derive(Debug, Default)]
pub struct TickClock {
    last_tick: Option<u32>,
    micros: u32,
    millis: u32,
}

impl TickClock {
    pub fn at(&mut self, tick: u32) -> u32 {
        let Some(last) = self.last_tick else {
            self.last_tick = Some(tick);
            return self.millis;
        };
        let elapsed = tick.wrapping_sub(last);
        if elapsed > u32::MAX / 2 {
            return self.millis;
        }
        self.last_tick = Some(tick);
        let micros = u64::from(self.micros) + u64::from(elapsed) * u64::from(PHY_TICK_US);
        let whole = micros / u64::from(MICROS_PER_MILLI);
        self.millis = self.millis.wrapping_add(u32::try_from(whole & u64::from(u32::MAX)).unwrap_or_default());
        self.micros = u32::try_from(micros % u64::from(MICROS_PER_MILLI)).unwrap_or_default();
        self.millis
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub after_gap: bool,
    pub epoch: u8,
    pub forward_length: bool,
    pub frame: Option<UnitFrame>,
    pub at_ms: u32,
}

#[derive(Debug, Default)]
pub struct UnitTap {
    wire: Option<BusUnitWire>,
    last_epoch: Option<u8>,
}

impl UnitTap {
    pub fn attach(&mut self, identity: Option<&BusUnitIdentity>, entropy: fn() -> u32) {
        if let (None, Some(identity)) = (&self.wire, identity) {
            self.wire = Some(BusUnitWire::new(identity, entropy));
        }
    }

    // IEC 62386-101 §9.3
    pub fn on_capture(&mut self, capture: Capture) -> Option<BusUnitEvidence> {
        let ours_between = self.ours_between(&capture);
        let wire = self.wire.as_mut()?;
        if capture.after_gap {
            wire.lost_capture();
        }
        if ours_between {
            wire.observe(UnitFrame::Other, capture.at_ms);
        }
        wire.observe(capture.frame?, capture.at_ms)
    }

    pub fn on_poll(
        &mut self,
        inputs: Option<&AnswerInputs>,
        now_ms: u32,
        caught_up: bool,
    ) -> (Option<BusUnitEvidence>, Option<AnswerTableImage>) {
        let Some(wire) = self.wire.as_mut() else {
            return (None, None);
        };
        let resend = wire.poll(inputs.filter(|_| caught_up), now_ms);
        (resend, wire.answer_table())
    }

    fn ours_between(&mut self, capture: &Capture) -> bool {
        let expected = self
            .last_epoch
            .map(|last| if capture.forward_length { last.wrapping_add(1) } else { last });
        self.last_epoch = Some(capture.epoch);
        expected.is_some_and(|expected| expected != capture.epoch)
    }
}

#[derive(Debug, Default)]
pub struct InjectedWire {
    tx: Option<ObservedFrameSender>,
    inputs: Option<Arc<AnswerInputs>>,
    tap: UnitTap,
    epoch: u8,
}

impl InjectedWire {
    pub fn set_sender(&mut self, tx: ObservedFrameSender) {
        self.tx = Some(tx);
    }

    pub fn set_answer_inputs(&mut self, inputs: Arc<AnswerInputs>) {
        self.inputs = Some(inputs);
    }

    pub fn set_identity(&mut self, identity: &BusUnitIdentity) {
        self.tap.attach(Some(identity), host_entropy);
    }

    pub fn note_own_forward(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub fn inject(&mut self, bytes: [u8; 3], kind: ObservedRawFrameKind) -> bool {
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        let at_ms = dali2rust_bsp::monotonic_clock::observation_stamp_ms();
        let sent = tx
            .try_send(ObservedItem::Frame(ObservedRawFrame {
                bytes,
                kind,
                observed_at_ms: dali2rust_bsp::unix_clock::unix_wall_clock_millis(),
                observed_at_mono_ms: at_ms,
            }))
            .is_ok();
        let forward_length = !matches!(kind, ObservedRawFrameKind::Backward8);
        if forward_length {
            self.epoch = self.epoch.wrapping_add(1);
        }
        let (resend, _) = self.tap.on_poll(self.inputs.as_deref(), at_ms, true);
        let frame = Some(unit_frame(kind, bytes));
        let capture = Capture { after_gap: false, epoch: self.epoch, forward_length, frame, at_ms };
        for evidence in [resend, self.tap.on_capture(capture)].into_iter().flatten() {
            let _ = tx.try_send(ObservedItem::BusUnit(evidence));
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;

    use dali2rust_domain::dali::dev103::bus_unit::FACTORY_NVM;
    use dali2rust_domain::dali::dev103::{Device103Address, Device103Command, Special103Command};
    use dali2rust_platform::bus_unit::BusUnitSnapshot;
    use dali2rust_platform::dali::DaliTransport;

    use super::super::mock::MockDaliTransport;
    use super::super::sim::SimDaliTransport;
    use super::*;

    const IDENTITY: BusUnitIdentity = BusUnitIdentity {
        firmware_version: [1, 4],
        hardware_version: HARDWARE_VERSION,
        identification_number: [0, 0, 0x30, 0xED, 0xA0, 0x12, 0x34, 0x56],
    };
    const START_MS: u32 = 1_000;
    const PAIR_GAP_MS: u32 = 20;
    const OPERATOR_SHORT: u8 = 7;
    const WIRE_SHORT: u8 = 5;
    const ADOPTED_REVISION: u32 = 3;

    const fn entropy() -> u32 {
        0x0012_3456
    }

    fn operator_nvm() -> BusUnitNvm {
        BusUnitNvm { short_address: Some(OPERATOR_SHORT), ..FACTORY_NVM }
    }

    fn inputs_at(nvm: BusUnitNvm, revision: u32, acknowledged_wire_sequence: u32) -> AnswerInputs {
        let inputs = AnswerInputs::new();
        inputs.publish_bus_unit(BusUnitSnapshot { nvm, revision, acknowledged_wire_sequence });
        inputs
    }

    fn dtr0(value: u8) -> UnitFrame {
        UnitFrame::Forward24(Special103Command::Dtr0.frame(value).as_bytes())
    }

    fn broadcast(command: Device103Command) -> UnitFrame {
        UnitFrame::Forward24(command.frame(Device103Address::Broadcast).as_bytes())
    }

    fn set_short_address() -> UnitFrame {
        broadcast(Device103Command::SetShortAddress)
    }

    fn hydrated(inputs: &AnswerInputs) -> BusUnitWire {
        let mut wire = BusUnitWire::new(&IDENTITY, entropy);
        assert_eq!(wire.poll(Some(inputs), START_MS - 1), None);
        wire
    }

    fn readdress(wire: &mut BusUnitWire) -> Option<BusUnitEvidence> {
        assert_eq!(wire.observe(dtr0(WIRE_SHORT), START_MS), None);
        assert_eq!(wire.observe(set_short_address(), START_MS + PAIR_GAP_MS), None);
        wire.observe(set_short_address(), START_MS + 2 * PAIR_GAP_MS)
    }

    fn reported_at() -> u32 {
        START_MS + 2 * PAIR_GAP_MS
    }

    fn short_address_of(wire: &BusUnitWire) -> Option<u8> {
        wire.answer_table().map(|table| table.short_address)
    }

    struct Tap {
        tap: UnitTap,
        epoch: u8,
        at_ms: u32,
    }

    impl Tap {
        fn hydrated(inputs: &AnswerInputs) -> Self {
            let mut tap = UnitTap::default();
            tap.attach(Some(&IDENTITY), entropy);
            assert_eq!(tap.on_poll(Some(inputs), START_MS - 1, true).0, None);
            Self { tap, epoch: 0, at_ms: START_MS }
        }

        fn capture(&mut self, frame: Option<UnitFrame>, after_gap: bool, forward_length: bool) -> Option<BusUnitEvidence> {
            if forward_length {
                self.epoch = self.epoch.wrapping_add(1);
            }
            self.at_ms += PAIR_GAP_MS;
            let capture = Capture { after_gap, epoch: self.epoch, forward_length, frame, at_ms: self.at_ms };
            self.tap.on_capture(capture)
        }

        fn forward(&mut self, frame: UnitFrame) -> Option<BusUnitEvidence> {
            self.capture(Some(frame), false, true)
        }

        fn ours(&mut self) {
            self.epoch = self.epoch.wrapping_add(1);
        }

        fn short_address(&mut self, inputs: &AnswerInputs, caught_up: bool) -> Option<u8> {
            self.tap.on_poll(Some(inputs), self.at_ms, caught_up).1.map(|table| table.short_address)
        }
    }

    #[test]
    fn a_unit_the_registry_never_described_publishes_no_table_and_reports_nothing() {
        let inputs = AnswerInputs::new();
        let mut wire = hydrated(&inputs);
        assert_eq!(readdress(&mut wire), None);
        assert_eq!(wire.poll(Some(&inputs), reported_at() + BUS_UNIT_RESEND_MS), None);
        assert_eq!(wire.answer_table(), None);
    }

    #[test]
    fn a_set_short_address_pair_reports_what_it_moved_from_and_to() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut wire = hydrated(&inputs);
        let evidence = readdress(&mut wire).expect("the pair moved the address");
        assert_eq!(evidence.adopted, operator_nvm());
        assert_eq!(evidence.adopted_revision, ADOPTED_REVISION);
        assert_eq!(evidence.observed, BusUnitNvm { short_address: Some(WIRE_SHORT), ..operator_nvm() });
        assert_eq!(evidence.wire_sequence, 1);
        assert_eq!(short_address_of(&wire), Some(WIRE_SHORT));
    }

    #[test]
    fn a_copy_published_between_polls_is_adopted_after_the_frames_captured_before_the_poll() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut wire = hydrated(&inputs);
        let later = BusUnitNvm { short_address: Some(9), ..operator_nvm() };
        inputs.publish_bus_unit(BusUnitSnapshot {
            nvm: later,
            revision: ADOPTED_REVISION + 1,
            acknowledged_wire_sequence: 0,
        });
        let evidence = readdress(&mut wire).expect("the pair moved the address");
        assert_eq!((evidence.adopted, evidence.adopted_revision), (operator_nvm(), ADOPTED_REVISION));
    }

    #[test]
    fn an_unacknowledged_report_is_resent_once_a_second() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut wire = hydrated(&inputs);
        let evidence = readdress(&mut wire);
        assert_eq!(wire.poll(Some(&inputs), reported_at() + BUS_UNIT_RESEND_MS - 1), None);
        assert_eq!(wire.poll(Some(&inputs), reported_at() + BUS_UNIT_RESEND_MS), evidence);
        assert_eq!(wire.poll(Some(&inputs), reported_at() + 2 * BUS_UNIT_RESEND_MS - 1), None);
        assert_eq!(wire.poll(Some(&inputs), reported_at() + 2 * BUS_UNIT_RESEND_MS), evidence);
    }

    #[test]
    fn the_registry_acknowledging_a_report_stops_its_resend_and_becomes_the_next_base() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut wire = hydrated(&inputs);
        let evidence = readdress(&mut wire).expect("the pair moved the address");
        let next = ADOPTED_REVISION + 1;
        inputs.publish_bus_unit(BusUnitSnapshot {
            nvm: evidence.observed,
            revision: next,
            acknowledged_wire_sequence: evidence.wire_sequence,
        });
        assert_eq!(wire.poll(Some(&inputs), reported_at() + BUS_UNIT_RESEND_MS), None);
        let at_ms = reported_at() + 2 * BUS_UNIT_RESEND_MS;
        assert_eq!(wire.observe(dtr0(OPERATOR_SHORT), at_ms), None);
        assert_eq!(wire.observe(set_short_address(), at_ms + PAIR_GAP_MS), None);
        let again = wire.observe(set_short_address(), at_ms + 2 * PAIR_GAP_MS);
        let again = again.expect("the second pair moved the address back");
        assert_eq!((again.adopted, again.adopted_revision), (evidence.observed, next));
    }

    #[test]
    fn a_lost_capture_breaks_the_pair_it_fell_inside() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut wire = hydrated(&inputs);
        let disable = broadcast(Device103Command::DisableApplicationController);
        assert_eq!(wire.observe(disable, START_MS), None);
        wire.lost_capture();
        assert_eq!(wire.observe(disable, START_MS + PAIR_GAP_MS), None, "no DTR operand: only the pair split stops it");
    }

    #[test]
    fn a_lost_capture_forgets_the_operand_an_intact_pair_would_use() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut wire = hydrated(&inputs);
        assert_eq!(wire.observe(dtr0(WIRE_SHORT), START_MS), None);
        wire.lost_capture();
        assert_eq!(wire.observe(set_short_address(), START_MS + PAIR_GAP_MS), None);
        assert_eq!(wire.observe(set_short_address(), START_MS + 2 * PAIR_GAP_MS), None);
        assert_eq!(short_address_of(&wire), Some(OPERATOR_SHORT));
    }

    #[test]
    fn a_frame_of_ours_between_the_halves_splits_a_foreign_pair() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut tap = Tap::hydrated(&inputs);
        let disable = broadcast(Device103Command::DisableApplicationController);
        assert_eq!(tap.forward(disable), None);
        tap.ours();
        assert_eq!(tap.forward(disable), None, "every other unit saw X, ours, X");
        assert!(tap.forward(disable).is_some(), "the next two halves are a pair again");
    }

    #[test]
    fn a_backward_capture_after_a_frame_of_ours_splits_the_pair_too() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut tap = Tap::hydrated(&inputs);
        let disable = broadcast(Device103Command::DisableApplicationController);
        assert_eq!(tap.forward(disable), None);
        tap.ours();
        assert_eq!(tap.capture(None, false, false), None);
        assert_eq!(tap.forward(disable), None);
    }

    #[test]
    fn a_gap_before_a_frame_is_handled_before_that_frame_is_fed() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut tap = Tap::hydrated(&inputs);
        assert_eq!(tap.capture(Some(dtr0(WIRE_SHORT)), true, true), None);
        assert_eq!(tap.forward(set_short_address()), None);
        assert!(tap.forward(set_short_address()).is_some(), "the DTR0 after the gap is kept");
        assert_eq!(tap.short_address(&inputs, true), Some(WIRE_SHORT));
    }

    #[test]
    fn a_gap_reported_on_a_capture_too_short_to_decode_still_splits_the_pair() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut tap = Tap::hydrated(&inputs);
        let disable = broadcast(Device103Command::DisableApplicationController);
        assert_eq!(tap.forward(disable), None);
        assert_eq!(tap.capture(None, true, false), None);
        assert_eq!(tap.forward(disable), None);
    }

    #[test]
    fn a_newer_copy_waits_for_a_poll_no_capture_is_waiting_for() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut tap = Tap::hydrated(&inputs);
        inputs.publish_bus_unit(BusUnitSnapshot {
            nvm: BusUnitNvm { short_address: Some(WIRE_SHORT), ..operator_nvm() },
            revision: ADOPTED_REVISION + 1,
            acknowledged_wire_sequence: 0,
        });
        assert_eq!(tap.short_address(&inputs, false), Some(OPERATOR_SHORT));
        assert_eq!(tap.short_address(&inputs, true), Some(WIRE_SHORT));
    }

    #[test]
    fn a_poll_behind_the_captures_still_resends_and_times_the_unit() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut tap = Tap::hydrated(&inputs);
        assert_eq!(tap.forward(dtr0(WIRE_SHORT)), None);
        assert_eq!(tap.forward(set_short_address()), None);
        let evidence = tap.forward(set_short_address());
        assert!(evidence.is_some());
        let due = tap.at_ms + BUS_UNIT_RESEND_MS;
        assert_eq!(tap.tap.on_poll(Some(&inputs), due, false).0, evidence);
    }

    #[test]
    fn a_second_attach_keeps_the_unit_the_first_one_built() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut tap = Tap::hydrated(&inputs);
        assert_eq!(tap.forward(dtr0(WIRE_SHORT)), None);
        assert_eq!(tap.forward(set_short_address()), None);
        tap.tap.attach(Some(&IDENTITY), entropy);
        let evidence = tap.forward(set_short_address());
        assert_eq!(evidence.map(|evidence| evidence.observed.short_address), Some(Some(WIRE_SHORT)));
    }

    #[test]
    fn a_tap_before_its_identity_or_without_answer_inputs_reports_nothing() {
        let inputs = inputs_at(operator_nvm(), ADOPTED_REVISION, 0);
        let mut tap = UnitTap::default();
        let capture = Capture { after_gap: false, epoch: 1, forward_length: true, frame: Some(dtr0(WIRE_SHORT)), at_ms: 0 };
        assert_eq!(tap.on_capture(capture), None);
        assert_eq!(tap.on_poll(Some(&inputs), 0, true), (None, None));
        tap.attach(Some(&IDENTITY), entropy);
        assert_eq!(tap.on_poll(None, 0, true), (None, None));
    }

    #[test]
    fn the_tick_clock_counts_whole_milliseconds_and_never_runs_back() {
        let mut clock = TickClock::default();
        let start = u32::MAX - 4;
        assert_eq!(clock.at(start), 0);
        assert_eq!(clock.at(start.wrapping_add(10)), 1);
        assert_eq!(clock.at(start.wrapping_add(20)), 2);
        assert_eq!(clock.at(start.wrapping_add(15)), 2);
        assert_eq!(clock.at(start.wrapping_add(30)), 3);
        assert_eq!(clock.at(start.wrapping_add(30 + 1_000 * 10)), 1_043);
    }

    #[test]
    fn the_tick_clock_carries_the_microseconds_a_short_step_leaves() {
        let mut clock = TickClock::default();
        assert_eq!(clock.at(0), 0);
        assert_eq!(clock.at(5), 0);
        assert_eq!(clock.at(10), 1, "two 520 µs steps make a millisecond");
    }

    #[test]
    fn only_a_24_bit_forward_frame_reaches_the_unit_as_itself() {
        let bytes = [0xC1, 0x30, 0x05];
        assert_eq!(unit_frame(ObservedRawFrameKind::Forward24, bytes), UnitFrame::Forward24(bytes));
        assert_eq!(unit_frame(ObservedRawFrameKind::Forward16, bytes), UnitFrame::Other);
        assert_eq!(unit_frame(ObservedRawFrameKind::Backward8, bytes), UnitFrame::Other);
        assert_eq!(sniffed_unit_frame(SniffedFrame::Forward24(bytes)), UnitFrame::Forward24(bytes));
        for other in [
            SniffedFrame::Forward16(0xFF00),
            SniffedFrame::Backward8(0xFF),
            SniffedFrame::UnsupportedLength(20),
            SniffedFrame::DecodeFailed,
        ] {
            assert_eq!(sniffed_unit_frame(other), UnitFrame::Other, "{other:?}");
        }
    }

    #[test]
    fn the_controller_identity_carries_the_release_and_the_hardware_address() {
        let mac = [0x30, 0xED, 0xA0, 0x12, 0x34, 0x56];
        let identity = controller_identity("1.4.1234+abcdef0", Some(mac));
        assert_eq!(identity.firmware_version, [1, 4]);
        assert_eq!(identity.hardware_version, HARDWARE_VERSION);
        assert_eq!(identity.identification_number, [0, 0, 0x30, 0xED, 0xA0, 0x12, 0x34, 0x56]);
        let unknown = controller_identity("dev", None);
        assert_eq!(unknown.firmware_version, [0, 0]);
        assert_eq!(unknown.identification_number, [0; 8]);
    }

    fn injected() -> (InjectedWire, std::sync::mpsc::Receiver<ObservedItem>) {
        let (tx, rx) = sync_channel(8);
        let mut wire = InjectedWire::default();
        wire.set_sender(tx);
        wire.set_answer_inputs(Arc::new(inputs_at(operator_nvm(), ADOPTED_REVISION, 0)));
        wire.set_identity(&IDENTITY);
        (wire, rx)
    }

    fn inject(wire: &mut InjectedWire, frame: UnitFrame) {
        let UnitFrame::Forward24(bytes) = frame else { unreachable!() };
        assert!(wire.inject(bytes, ObservedRawFrameKind::Forward24));
    }

    #[test]
    fn an_injected_frame_reaches_the_channel_before_the_report_it_caused() {
        let (mut wire, rx) = injected();
        let frames = [dtr0(WIRE_SHORT), set_short_address(), set_short_address()];
        for frame in frames {
            inject(&mut wire, frame);
        }
        let items: Vec<_> = rx.try_iter().collect();
        assert_eq!(items.len(), frames.len() + 1, "{items:?}");
        assert!(items[..frames.len()].iter().all(|item| matches!(item, ObservedItem::Frame(_))));
        assert!(matches!(items[frames.len()], ObservedItem::BusUnit(_)), "{items:?}");
    }

    #[test]
    fn a_frame_the_host_transport_sent_splits_an_injected_pair() {
        let (mut wire, rx) = injected();
        let disable = broadcast(Device103Command::DisableApplicationController);
        inject(&mut wire, disable);
        wire.note_own_forward();
        inject(&mut wire, disable);
        assert!(rx.try_iter().all(|item| matches!(item, ObservedItem::Frame(_))));
    }

    type Send<T> = fn(&mut T);

    fn pair_reported<T: DaliTransport>(
        mut transport: T,
        inject: fn(&mut T, [u8; 3]) -> bool,
        between: Option<Send<T>>,
    ) -> bool {
        let (tx, rx) = sync_channel(8);
        transport.set_observed_frame_sender(tx);
        transport.set_answer_inputs(Arc::new(inputs_at(operator_nvm(), ADOPTED_REVISION, 0)));
        transport.set_bus_unit_identity(IDENTITY);
        let UnitFrame::Forward24(disable) = broadcast(Device103Command::DisableApplicationController) else {
            unreachable!()
        };
        assert!(inject(&mut transport, disable));
        if let Some(send) = between {
            send(&mut transport);
        }
        assert!(inject(&mut transport, disable));
        rx.try_iter().any(|item| matches!(item, ObservedItem::BusUnit(_)))
    }

    fn sends<T: DaliTransport>() -> [Send<T>; 2] {
        [
            |transport| drop(transport.exchange_frame(0xFF00, false)),
            |transport| drop(transport.exchange_frame24_with_settle([0xFF, 0xFE, 0x00], false, 0)),
        ]
    }

    #[test]
    fn a_frame_a_host_transport_sends_between_two_halves_splits_the_pair() {
        let mock: fn(&mut MockDaliTransport, [u8; 3]) -> bool =
            |mock, bytes| mock.inject_observed_frame(bytes, ObservedRawFrameKind::Forward24);
        let sim: fn(&mut SimDaliTransport, [u8; 3]) -> bool =
            |sim, bytes| sim.inject_observed_frame(bytes, ObservedRawFrameKind::Forward24);
        assert!(pair_reported(MockDaliTransport::new(), mock, None));
        assert!(pair_reported(SimDaliTransport::demo_bus(), sim, None));
        for send in sends() {
            assert!(!pair_reported(MockDaliTransport::new(), mock, Some(send)));
        }
        for send in sends() {
            assert!(!pair_reported(SimDaliTransport::demo_bus(), sim, Some(send)));
        }
    }
}
