use crate::fsm::{PHY_TICK_US, TX_ARM_LEAD_TICKS};


// IEC 62386-101 Table 20
pub const BACKWARD_MIN_US: u32 = 5_500;
pub const BACKWARD_MAX_US: u32 = 10_500;
pub const BACKWARD_TARGET_US: u32 = 6_500;

#[allow(clippy::arithmetic_side_effects, reason = "const-evaluated division by a non-zero constant; nothing reaches the interrupt")]
const fn us_to_ticks(us: u32) -> u32 {
    us / PHY_TICK_US
}

pub const BACKWARD_DATA_BITS: u8 = 8;

pub const ANSWER_ARM_LEAD_TICKS: u16 = TX_ARM_LEAD_TICKS as u16;

pub const ANSWER_ARM_TARGET_IDLE_TICKS: u16 =
    (us_to_ticks(BACKWARD_TARGET_US) as u16).saturating_sub(ANSWER_ARM_LEAD_TICKS);
pub const ANSWER_ARM_MAX_IDLE_TICKS: u16 =
    (us_to_ticks(BACKWARD_MAX_US) as u16).saturating_sub(ANSWER_ARM_LEAD_TICKS);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerGate {
    Hold,
    Arm { late: bool },
    Expired,
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
pub const fn answer_gate(idle_ticks: u16, target: u16, max: u16) -> AnswerGate {
    if idle_ticks < target {
        AnswerGate::Hold
    } else if idle_ticks <= max {
        AnswerGate::Arm { late: idle_ticks > target }
    } else {
        AnswerGate::Expired
    }
}

const ANSWER_TARGET_EDGE_US: u32 = (ANSWER_ARM_TARGET_IDLE_TICKS as u32)
    .saturating_add(ANSWER_ARM_LEAD_TICKS as u32)
    .saturating_mul(PHY_TICK_US);
const _: () = assert!(ANSWER_TARGET_EDGE_US >= BACKWARD_MIN_US);
const _: () = assert!(ANSWER_TARGET_EDGE_US <= BACKWARD_MAX_US);
const ANSWER_MAX_EDGE_US: u32 = (ANSWER_ARM_MAX_IDLE_TICKS as u32)
    .saturating_add(ANSWER_ARM_LEAD_TICKS as u32)
    .saturating_mul(PHY_TICK_US);
const _: () = assert!(ANSWER_MAX_EDGE_US <= BACKWARD_MAX_US);
const _: () = assert!(ANSWER_ARM_TARGET_IDLE_TICKS < ANSWER_ARM_MAX_IDLE_TICKS);

const PHY_QUIET_AFTER_ANSWER_TICKS: u32 = (ANSWER_ARM_MAX_IDLE_TICKS as u32)
    .saturating_add(ANSWER_ARM_LEAD_TICKS as u32)
    .saturating_add(1);

const _: () = assert!(
    PHY_QUIET_AFTER_ANSWER_TICKS == dali2rust_platform::dali::PERSIST_QUIET_GAP_TICKS
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_arm_lands_inside_table_20() {
        let first_edge_us =
            (u32::from(ANSWER_ARM_LEAD_TICKS) + u32::from(ANSWER_ARM_TARGET_IDLE_TICKS)) * PHY_TICK_US;
        assert!(
            (BACKWARD_MIN_US..=BACKWARD_MAX_US).contains(&first_edge_us),
            "first edge at {first_edge_us} µs is outside {BACKWARD_MIN_US}..={BACKWARD_MAX_US}"
        );
    }

    #[test]
    fn answer_gate_holds_below_target_arms_inside_and_expires_past_max() {
        let (t, m) = (ANSWER_ARM_TARGET_IDLE_TICKS, ANSWER_ARM_MAX_IDLE_TICKS);
        assert_eq!(t, 59, "target moved — update the literals below with the reason");
        assert_eq!(m, 97, "max moved — update the literals below with the reason");
        assert_eq!(answer_gate(58, t, m), AnswerGate::Hold);
        assert_eq!(answer_gate(59, t, m), AnswerGate::Arm { late: false });
        assert_eq!(answer_gate(60, t, m), AnswerGate::Arm { late: true });
        assert_eq!(answer_gate(97, t, m), AnswerGate::Arm { late: true });
        assert_eq!(answer_gate(98, t, m), AnswerGate::Expired);
    }
}
