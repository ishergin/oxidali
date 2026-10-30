use core::sync::atomic::Ordering::Relaxed;

use dali2rust_dali_phy::{
    AnswerCounts, AnswerTableCounts, PhyIsrCore, RxCompletedEvent, FORWARD24_BITS, PHY_TICK_US,
};
use dali2rust_platform::answer_table::AnswerTableImage;
use dali2rust_platform::arbitration::{AnswerInputs, ArbitrationAnswerCounters};
use dali2rust_platform::dali::PhySnifferCounters;

// IEC 62386-101 Table 22
const BACKWARD_TO_FORWARD_SETTLE_MIN_US: u32 = 2_400;
// IEC 62386-101 Table 21
const MULTI_MASTER_HALF_BIT_MIN_US: u32 = 400;
const FORWARD24_HALF_BITS: u32 = (1 + FORWARD24_BITS as u32) * 2;

pub const PUBLISH_BUDGET_TICKS: u32 = (BACKWARD_TO_FORWARD_SETTLE_MIN_US
    + FORWARD24_HALF_BITS * MULTI_MASTER_HALF_BIT_MIN_US)
    / PHY_TICK_US;

#[must_use]
pub const fn arbitration_answers(
    sent: &AnswerCounts,
    table: &AnswerTableCounts,
) -> ArbitrationAnswerCounters {
    ArbitrationAnswerCounters {
        answered: sent.sent,
        suppressed_lease: table.lease_lapsed,
        cell_busy: table.cell_busy,
        aborted: sent.stale,
        window_closed: sent.expired,
        out_of_window: sent.late,
    }
}

pub fn mirror_answer_counts(sink: &PhySnifferCounters, sent: &AnswerCounts, table: &AnswerTableCounts) {
    sink.answer_staged.store(table.answered, Relaxed);
    sink.answer_over_backward.store(sent.over_backward, Relaxed);
    sink.answer_table_behind.store(table.behind, Relaxed);
    sink.answer_table_torn.store(table.torn, Relaxed);
    sink.answer_decode_overrun.store(table.decode_overrun, Relaxed);
}

#[derive(Debug, Default)]
pub struct AnswerPublisher {
    accounts_for: u32,
    oldest_unpublished_tick: Option<u32>,
}

impl AnswerPublisher {
    pub fn drain(
        &mut self,
        mut pop: impl FnMut() -> Option<RxCompletedEvent>,
        mut each: impl FnMut(&RxCompletedEvent, bool),
    ) {
        while let Some(ev) = pop() {
            let after_gap = ev.sniff_seq != self.accounts_for.wrapping_add(1);
            self.accounts_for = ev.sniff_seq;
            self.oldest_unpublished_tick.get_or_insert(ev.rx_tick);
            each(&ev, after_gap);
        }
    }

    #[must_use]
    pub fn caught_up(&self, isr: &PhyIsrCore) -> bool {
        isr.latest_capture() == self.accounts_for
    }

    pub fn publish(
        &mut self,
        isr: &PhyIsrCore,
        inputs: Option<&AnswerInputs>,
        table: Option<&AnswerTableImage>,
        now_ms: u32,
    ) -> Option<u32> {
        let now_tick = isr.wire_tick_counts().0;
        let remaining_ms = inputs.map_or(0, |inputs| inputs.lease_remaining_ms(now_ms));
        isr.set_answer_lease_for_ms(now_tick, remaining_ms);
        let oldest = self.oldest_unpublished_tick.take();
        isr.publish_answer_table(table?, self.accounts_for);
        oldest.map(|captured| now_tick.wrapping_sub(captured))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dali::transport::bus_unit_wire::BusUnitWire;
    use dali2rust_dali_phy::{RegisterGpio, TableLookup};
    use dali2rust_domain::dali::dev103::bus_unit::FACTORY_NVM;
    use dali2rust_platform::bus_unit::{BusUnitIdentity, BusUnitSnapshot};

    const QACE_BROADCAST: [u8; 3] = [0xFF, 0xFE, 0x3D];
    const NOW_MS: u32 = 1_000;
    const LEASE_MS: u32 = 3_000;
    const TOO_SHORT_TO_DECODE: u16 = 2;

    struct Pins {
        set: u32,
        clr: u32,
        input: u32,
    }

    impl Pins {
        const IDLE: Self = Self { set: 0, clr: 0, input: 2 };
    }

    fn core(pins: &mut Pins) -> PhyIsrCore {
        // SAFETY: the three "registers" outlive the core within each test.
        let gpio = unsafe { RegisterGpio::new(&mut pins.set, &mut pins.clr, &pins.input, 1, 2) };
        PhyIsrCore::new(gpio)
    }

    const IDENTITY: BusUnitIdentity = BusUnitIdentity {
        firmware_version: [0, 1],
        hardware_version: [1, 0],
        identification_number: [0; 8],
    };

    struct Named {
        inputs: AnswerInputs,
        wire: BusUnitWire,
    }

    impl Named {
        fn new(application_active: bool) -> Self {
            let inputs = AnswerInputs::new();
            inputs.extend_lease(NOW_MS, LEASE_MS);
            let mut named = Self { inputs, wire: BusUnitWire::new(&IDENTITY, || 0) };
            named.rename(application_active, 1);
            named
        }

        fn rename(&mut self, application_active: bool, revision: u32) {
            let nvm = dali2rust_platform::bus_unit::BusUnitNvm {
                short_address: Some(7),
                application_active,
                ..FACTORY_NVM
            };
            let snapshot = BusUnitSnapshot { nvm, revision, acknowledged_wire_sequence: 0 };
            self.inputs.publish_bus_unit(snapshot);
            let _ = self.wire.poll(Some(&self.inputs), NOW_MS);
        }

        fn publish(&self, publisher: &mut AnswerPublisher, isr: &PhyIsrCore) -> Option<u32> {
            publisher.publish(isr, Some(&self.inputs), self.wire.answer_table().as_ref(), NOW_MS)
        }
    }

    fn capture(sniff_seq: u32, rx_tick: u32, sample_count: u16) -> RxCompletedEvent {
        RxCompletedEvent {
            sniff_seq,
            rx_tick,
            sample_count,
            ..RxCompletedEvent::new()
        }
    }

    fn drain(publisher: &mut AnswerPublisher, captures: &[RxCompletedEvent]) -> Vec<(u32, bool)> {
        let mut queue = captures.iter().cloned();
        let mut handed = Vec::new();
        publisher.drain(|| queue.next(), |ev, after_gap| handed.push((ev.sniff_seq, after_gap)));
        handed
    }

    const SENT: AnswerCounts = AnswerCounts {
        sent: 1,
        stale: 2,
        expired: 3,
        late: 4,
        rejected: 5,
        collided: 6,
        over_backward: 7,
    };
    const TABLE: AnswerTableCounts = AnswerTableCounts {
        answered: 11,
        lease_lapsed: 12,
        behind: 13,
        torn: 14,
        cell_busy: 15,
        decode_overrun: 16,
    };

    #[test]
    fn every_arbitration_count_comes_from_its_own_interrupt_counter() {
        assert_eq!(
            arbitration_answers(&SENT, &TABLE),
            ArbitrationAnswerCounters {
                answered: 1,
                suppressed_lease: 12,
                cell_busy: 15,
                aborted: 2,
                window_closed: 3,
                out_of_window: 4,
            }
        );
    }

    #[test]
    fn every_table_count_reaches_its_own_sniffer_counter() {
        let sink = PhySnifferCounters::default();
        mirror_answer_counts(&sink, &SENT, &TABLE);
        let got = [
            sink.answer_staged.load(Relaxed),
            sink.answer_table_behind.load(Relaxed),
            sink.answer_table_torn.load(Relaxed),
            sink.answer_decode_overrun.load(Relaxed),
            sink.answer_over_backward.load(Relaxed),
        ];
        assert_eq!(got, [11, 13, 14, 16, 7]);
    }

    #[test]
    fn the_budget_is_the_shortest_wire_time_from_one_capture_to_the_end_of_a_24_bit_query() {
        assert_eq!(PUBLISH_BUDGET_TICKS, 215);
    }

    #[test]
    fn nothing_is_published_before_the_unit_is_hydrated() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::default();
        let inputs = AnswerInputs::new();
        inputs.extend_lease(NOW_MS, LEASE_MS);
        let mut wire = BusUnitWire::new(&IDENTITY, || 0);
        assert_eq!(wire.poll(Some(&inputs), NOW_MS), None);
        drain(&mut publisher, &[capture(1, 0, 27)]);
        assert_eq!(publisher.publish(&isr, Some(&inputs), wire.answer_table().as_ref(), NOW_MS), None);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 1), TableLookup::NotPublished);
    }

    #[test]
    fn every_drained_capture_is_handed_on_and_accounted_for() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::default();
        let handed = drain(
            &mut publisher,
            &[capture(1, 0, 27), capture(2, 0, TOO_SHORT_TO_DECODE)],
        );
        assert_eq!(handed, [(1, false), (2, false)]);
        let handed = drain(&mut publisher, &[capture(4, 0, 27), capture(5, 0, 27)]);
        assert_eq!(handed, [(4, true), (5, false)], "the capture after a lost one says so");
        Named::new(true).publish(&mut publisher, &isr);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 5), TableLookup::Answer(0xFF));
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 4), TableLookup::Behind);
    }

    #[test]
    fn the_drain_is_caught_up_only_with_the_last_capture_the_interrupt_numbered() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::default();
        assert!(publisher.caught_up(&isr));
        drain(&mut publisher, &[capture(1, 0, 27)]);
        assert!(!publisher.caught_up(&isr), "the drain names a capture the interrupt never numbered");
    }

    #[test]
    fn a_changed_unit_reaches_the_table_at_the_next_poll() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::default();
        let mut named = Named::new(true);
        named.publish(&mut publisher, &isr);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 0), TableLookup::Answer(0xFF));
        named.rename(false, 2);
        named.publish(&mut publisher, &isr);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 0), TableLookup::Silent);
    }

    #[test]
    fn a_revoked_lease_reaches_the_interrupt_at_the_next_poll() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::default();
        let named = Named::new(true);
        named.publish(&mut publisher, &isr);
        named.inputs.revoke_lease(NOW_MS);
        named.publish(&mut publisher, &isr);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 0), TableLookup::LeaseLapsed);
    }

    #[test]
    fn the_delay_runs_from_the_oldest_capture_the_publish_accounts_for() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::default();
        let named = Named::new(true);
        drain(
            &mut publisher,
            &[capture(1, u32::MAX - 4, 27), capture(2, u32::MAX - 1, 27)],
        );
        assert_eq!(named.publish(&mut publisher, &isr), Some(5));
        assert_eq!(named.publish(&mut publisher, &isr), None);
    }

    #[test]
    fn captures_before_the_unit_is_named_carry_no_delay_forward() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::default();
        drain(&mut publisher, &[capture(1, u32::MAX - 4, 27)]);
        assert_eq!(publisher.publish(&isr, Some(&AnswerInputs::new()), None, NOW_MS), None);
        assert_eq!(Named::new(true).publish(&mut publisher, &isr), None);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 1), TableLookup::Answer(0xFF));
    }
}
