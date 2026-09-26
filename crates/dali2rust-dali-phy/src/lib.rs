#![cfg_attr(not(test), no_std)]
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::float_arithmetic,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
    )
)]

pub mod answer;
pub mod backward_window;
pub mod command;
pub mod cpu;
pub mod frame_length;
pub mod fsm;
pub mod gpio;
pub mod halfbits;
pub mod isr;
pub mod ring;
pub mod timer_alarm;

pub use answer::AtomicAnswerCell;
pub use backward_window::{
    answer_gate, AnswerGate, ANSWER_ARM_LEAD_TICKS, ANSWER_ARM_MAX_IDLE_TICKS,
    ANSWER_ARM_TARGET_IDLE_TICKS, BACKWARD_DATA_BITS, BACKWARD_MAX_US, BACKWARD_MIN_US,
    BACKWARD_TARGET_US,
};
pub use command::{AtomicCommandCell, ExchangeId, TxCommand, TxGates};
pub use cpu::{isr_core_id, ISR_CORE_ID};
pub use frame_length::{
    frame_length_class, FrameLengthClass, BACKWARD8_SAMPLE_COUNT, FORWARD16_SAMPLE_COUNT,
    FORWARD24_SAMPLE_COUNT, FORWARD_LENGTH_MIN_SAMPLES, MIN_SAMPLES_FOR_DECODE,
};
pub use fsm::{
    restart_gate_for, settle_ticks_on_wire, BusState, CollisionPolicy, DaliBitbangPhy,
    RxCompletedEvent, RxState, TxPollResult, BUS_POWER_DOWN_TICKS, DALI_RX_SAMPLE_BUF_LEN, LINE_HELD_TICKS, PHY_TICK_US,
    RX_IDLE_LINE_HIGH_TICKS, RX_START_DEBOUNCE_TICKS, TX_ARM_LEAD_TICKS, TX_HALF_BIT_TICKS,
};
pub use gpio::RegisterGpio;
pub use halfbits::HalfBitBuffer;
pub use isr::{
    dali_phy_alarm_isr, AnswerCounts, LateTick, PhyIsrCore, SessionEvent, LATE_TICK_US,
    RX_RING_CAP, SESSION_RING_CAP,
};
pub use ring::SpscRing;
pub use timer_alarm::{dali_phy_raw_isr, PhyAlarmIsr, TimerAlarmRegs};

#[cfg(all(target_os = "espidf", not(test)))]
const _: () = assert!(
    !cfg!(debug_assertions),
    "dali2rust-dali-phy must be built with debug-assertions off on ESP targets — see ADR-010"
);

#[cfg(test)]
mod profile_tests {
    #[test]
    fn host_tests_keep_the_checks_the_firmware_gives_up() {
        const {
            assert!(
                cfg!(debug_assertions),
                "[profile.test.package.dali2rust-dali-phy] override is missing"
            )
        };
    }
}
