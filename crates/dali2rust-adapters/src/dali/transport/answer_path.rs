use core::sync::atomic::Ordering::Relaxed;

use dali2rust_dali_phy::{
    AnswerCounts, AnswerTableCounts, PhyIsrCore, RxCompletedEvent, FORWARD24_BITS, PHY_TICK_US,
    TX_HALF_BIT_TICKS,
};
use dali2rust_domain::dali::dev103::unit::answer_table;
use dali2rust_platform::arbitration::{AnswerInputs, ArbitrationAnswerCounters};
use dali2rust_platform::dali::PhySnifferCounters;

const US_PER_MS: u32 = 1_000;
// IEC 62386-101 Table 22
const BACKWARD_TO_FORWARD_SETTLE_MIN_US: u32 = 2_400;
const FORWARD24_HALF_BITS: u32 = (1 + FORWARD24_BITS as u32) * 2;

pub const PUBLISH_BUDGET_TICKS: u32 =
    BACKWARD_TO_FORWARD_SETTLE_MIN_US / PHY_TICK_US + FORWARD24_HALF_BITS * TX_HALF_BIT_TICKS;

#[must_use]
pub fn lease_until_tick(now_tick: u32, remaining_ms: u32) -> u32 {
    now_tick.wrapping_add(remaining_ms.saturating_mul(US_PER_MS) / PHY_TICK_US)
}

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

pub fn mirror_table_counts(sink: &PhySnifferCounters, table: &AnswerTableCounts) {
    sink.answer_staged.store(table.answered, Relaxed);
    sink.answer_table_behind.store(table.behind, Relaxed);
    sink.answer_table_torn.store(table.torn, Relaxed);
    sink.answer_decode_overrun.store(table.decode_overrun, Relaxed);
}

#[derive(Debug, Default)]
pub struct AnswerPublisher {
    accounts_for: u8,
    oldest_unpublished_tick: Option<u32>,
}

impl AnswerPublisher {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            accounts_for: 0,
            oldest_unpublished_tick: None,
        }
    }

    pub fn drain(
        &mut self,
        mut pop: impl FnMut() -> Option<RxCompletedEvent>,
        mut each: impl FnMut(&RxCompletedEvent),
    ) {
        while let Some(ev) = pop() {
            self.accounts_for = ev.sniff_seq;
            self.oldest_unpublished_tick.get_or_insert(ev.rx_tick);
            each(&ev);
        }
    }

    pub fn publish(
        &mut self,
        isr: &PhyIsrCore,
        inputs: Option<&AnswerInputs>,
        now_ms: u32,
    ) -> Option<u32> {
        let now_tick = isr.wire_tick_counts().0;
        let remaining_ms = inputs.map_or(0, |inputs| inputs.lease_remaining_ms(now_ms));
        isr.set_answer_lease_until_tick(lease_until_tick(now_tick, remaining_ms));
        let oldest = self.oldest_unpublished_tick.take();
        let unit = inputs.and_then(AnswerInputs::unit)?;
        isr.publish_answer_table(
            &answer_table(unit.application_active, unit.device_short_address),
            self.accounts_for,
        );
        oldest.map(|captured| now_tick.wrapping_sub(captured))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_dali_phy::{RegisterGpio, TableLookup};
    use dali2rust_platform::arbitration::UnitInputs;

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

    fn named(application_active: bool) -> AnswerInputs {
        let inputs = AnswerInputs::new();
        inputs.publish_unit(UnitInputs {
            application_active,
            device_short_address: Some(7),
        });
        inputs.extend_lease(NOW_MS, LEASE_MS);
        inputs
    }

    fn capture(sniff_seq: u8, rx_tick: u32, sample_count: u16) -> RxCompletedEvent {
        RxCompletedEvent {
            sniff_seq,
            rx_tick,
            sample_count,
            ..RxCompletedEvent::new()
        }
    }

    fn drain(publisher: &mut AnswerPublisher, captures: &[RxCompletedEvent]) -> Vec<u8> {
        let mut queue = captures.iter().cloned();
        let mut handed = Vec::new();
        publisher.drain(|| queue.next(), |ev| handed.push(ev.sniff_seq));
        handed
    }

    const SENT: AnswerCounts = AnswerCounts {
        sent: 1,
        stale: 2,
        expired: 3,
        late: 4,
        rejected: 5,
        collided: 6,
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
        mirror_table_counts(&sink, &TABLE);
        let got = [
            sink.answer_staged.load(Relaxed),
            sink.answer_table_behind.load(Relaxed),
            sink.answer_table_torn.load(Relaxed),
            sink.answer_decode_overrun.load(Relaxed),
        ];
        assert_eq!(got, [11, 13, 14, 16]);
    }

    #[test]
    fn three_seconds_of_lease_is_three_seconds_of_ticks() {
        assert_eq!(lease_until_tick(0, 3_000), 3_000_000 / PHY_TICK_US);
        assert_eq!(lease_until_tick(u32::MAX, 0), u32::MAX);
        assert_eq!(lease_until_tick(10, u32::MAX), 10u32.wrapping_add(u32::MAX / PHY_TICK_US));
    }

    #[test]
    fn the_budget_is_the_shortest_wire_time_from_one_capture_to_the_end_of_a_24_bit_query() {
        assert_eq!(PUBLISH_BUDGET_TICKS, 2_400 / 104 + 50 * 4);
    }

    #[test]
    fn nothing_is_published_before_the_supervisor_names_the_unit() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::new();
        let inputs = AnswerInputs::new();
        inputs.extend_lease(NOW_MS, LEASE_MS);
        drain(&mut publisher, &[capture(1, 0, 27)]);
        assert_eq!(publisher.publish(&isr, Some(&inputs), NOW_MS), None);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 1), TableLookup::NotPublished);
    }

    #[test]
    fn every_drained_capture_is_handed_on_and_accounted_for() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::new();
        let handed = drain(
            &mut publisher,
            &[capture(4, 0, 27), capture(5, 0, TOO_SHORT_TO_DECODE)],
        );
        assert_eq!(handed, [4, 5]);
        publisher.publish(&isr, Some(&named(true)), NOW_MS);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 5), TableLookup::Answer(0xFF));
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 4), TableLookup::Behind);
    }

    #[test]
    fn a_changed_unit_reaches_the_table_at_the_next_poll() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::new();
        publisher.publish(&isr, Some(&named(true)), NOW_MS);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 0), TableLookup::Answer(0xFF));
        publisher.publish(&isr, Some(&named(false)), NOW_MS);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 0), TableLookup::Silent);
    }

    #[test]
    fn a_revoked_lease_reaches_the_interrupt_at_the_next_poll() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::new();
        let inputs = named(true);
        publisher.publish(&isr, Some(&inputs), NOW_MS);
        inputs.revoke_lease();
        publisher.publish(&isr, Some(&inputs), NOW_MS);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 0), TableLookup::LeaseLapsed);
    }

    #[test]
    fn the_delay_runs_from_the_oldest_capture_the_publish_accounts_for() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::new();
        let inputs = named(true);
        drain(
            &mut publisher,
            &[capture(1, u32::MAX - 4, 27), capture(2, u32::MAX - 1, 27)],
        );
        assert_eq!(publisher.publish(&isr, Some(&inputs), NOW_MS), Some(5));
        assert_eq!(publisher.publish(&isr, Some(&inputs), NOW_MS), None);
    }

    #[test]
    fn captures_before_the_unit_is_named_carry_no_delay_forward() {
        let mut pins = Pins::IDLE;
        let isr = core(&mut pins);
        let mut publisher = AnswerPublisher::new();
        drain(&mut publisher, &[capture(1, u32::MAX - 4, 27)]);
        assert_eq!(publisher.publish(&isr, Some(&AnswerInputs::new()), NOW_MS), None);
        assert_eq!(publisher.publish(&isr, Some(&named(true)), NOW_MS), None);
        assert_eq!(isr.lookup_answer(QACE_BROADCAST, 1), TableLookup::Answer(0xFF));
    }
}
