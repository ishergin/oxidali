use core::sync::atomic::{AtomicBool, Ordering};

use dali2rust_platform::hal::BitbangHal;

use crate::halfbits::HalfBitBuffer;

pub const DALI_RX_SAMPLE_BUF_LEN: usize = 64;
const MAX_RX_IDX: usize = DALI_RX_SAMPLE_BUF_LEN - 1;
pub const PHY_TICK_US: u32 = 104;

pub const RX_IDLE_LINE_HIGH_TICKS: u8 = 16;

pub const RX_START_DEBOUNCE_TICKS: u8 = 2;
pub const TX_HALF_BIT_TICKS: u32 = 4;

// IEC 62386-101 §4.11, Table 4
pub const BUS_POWER_DOWN_TICKS: u16 = 433;
pub const LINE_HELD_TICKS: u16 = 24;
pub const SYSTEM_FAILURE_TICKS: u16 = 5_288;

pub const TX_ARM_LEAD_TICKS: u8 = 3;

// IEC 62386-101 Table 25
const COLLISION_BREAK_MIN_US: u32 = 1_200;

const fn ceil_ticks_for_window(min_us: u32, tick_us: u32) -> u8 {
    min_us.div_ceil(tick_us) as u8
}

pub const COLLISION_BREAK_TICKS: u8 = ceil_ticks_for_window(COLLISION_BREAK_MIN_US, PHY_TICK_US);
const COLLISION_CHECK_TICK: u8 = COLLISION_BREAK_TICKS + 2;

const BACKWARD_FRAME_HALF_BITS: u8 = 22;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum BusState {
    Idle = 0,
    Rx = 1,
    Tx = 3,
    CollisionTx = 4,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RxState {
    Empty,
    Receiving,
    Completed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TxPollResult {
    Ok = 0,
    Collision = 3,
    Transmitting = 4,
    Yielded = 5,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CollisionPolicy {
    Auto = 0,
    Off = 1,
    On = 2,
}

pub const fn settle_ticks_on_wire(pre_idle_ticks: u16, lead: u8) -> u16 {
    pre_idle_ticks.saturating_add(lead as u16)
}

// IEC 62386-101 §9.1.4
pub const fn restart_gate_for(release_to_edge_ticks: u8) -> u8 {
    release_to_edge_ticks.saturating_sub(TX_ARM_LEAD_TICKS + 1)
}

#[derive(Debug)]
pub struct RxCompletedEvent {
    pub samples: [u8; DALI_RX_SAMPLE_BUF_LEN],
    pub sample_count: u16,
    pub pre_idle_ticks: u8,
    pub rx_epoch: u8,
    pub rx_tick: u32,
    pub max_sample_gap_us: u32,
    pub timing_degraded: bool,
}

impl Clone for RxCompletedEvent {
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn clone(&self) -> Self {
        Self {
            samples: self.samples,
            sample_count: self.sample_count,
            pre_idle_ticks: self.pre_idle_ticks,
            rx_epoch: self.rx_epoch,
            rx_tick: self.rx_tick,
            max_sample_gap_us: self.max_sample_gap_us,
            timing_degraded: self.timing_degraded,
        }
    }
}

impl RxCompletedEvent {
    pub const MAX_SAMPLES: usize = DALI_RX_SAMPLE_BUF_LEN;
    pub const fn new() -> Self {
        Self {
            samples: [0; DALI_RX_SAMPLE_BUF_LEN],
            sample_count: 0,
            pre_idle_ticks: 0,
            rx_epoch: 0,
            rx_tick: 0,
            max_sample_gap_us: 0,
            timing_degraded: false,
        }
    }
}

impl Default for RxCompletedEvent {
    fn default() -> Self {
        Self::new()
    }
}

pub struct DaliBitbangPhy<H: BitbangHal> {
    hal: H,
    pub tx_busy: AtomicBool,
    bus_state: BusState,
    idlecnt: u16,
    high_run_ticks: u16,
    rx_state: RxState,
    rx_data: [u8; DALI_RX_SAMPLE_BUF_LEN],
    rx_pos: u16,
    rx_byte: u8,
    rx_bit_cnt: u8,
    rx_idle: u8,
    rx_debounce: u8,
    low_run_ticks: u16,
    tx_hb_data: [u8; HalfBitBuffer::DATA_LEN],
    tx_hb_len: u8,
    tx_hb_cnt: u8,
    tx_sp_cnt: u8,
    tx_high: u8,
    tx_collision: u8,
    tx_idle_after_break: bool,
    tx_armed: u8,
    tx_yielded: bool,
    rx_pre_idle: u8,
    tx_pre_idle: u16,
    collision_policy: CollisionPolicy,
    last_rx: RxCompletedEvent,
}

impl<H: BitbangHal> DaliBitbangPhy<H> {
    pub const RX_BUF_SIZE: usize = DALI_RX_SAMPLE_BUF_LEN;

    pub fn new(hal: H) -> Self {
        let mut s = Self {
            hal,
            tx_busy: AtomicBool::new(false),
            bus_state: BusState::Idle,
            idlecnt: 0,
            high_run_ticks: 0,
            rx_state: RxState::Empty,
            rx_data: [0; DALI_RX_SAMPLE_BUF_LEN],
            rx_pos: 0,
            rx_byte: 0,
            rx_bit_cnt: 0,
            rx_idle: 0,
            rx_debounce: 0,
            low_run_ticks: 0,
            tx_hb_data: [0; HalfBitBuffer::DATA_LEN],
            tx_hb_len: 0,
            tx_hb_cnt: 0,
            tx_sp_cnt: 0,
            tx_high: 0,
            tx_collision: 0,
            tx_idle_after_break: false,
            tx_armed: 0,
            tx_yielded: false,
            rx_pre_idle: 0,
            tx_pre_idle: 0,
            collision_policy: CollisionPolicy::Auto,
            last_rx: RxCompletedEvent::new(),
        };
        s.set_bus_idle();
        s.rx_state = RxState::Empty;
        s.tx_collision = 0;
        s
    }

    pub fn set_collision_policy(&mut self, p: CollisionPolicy) {
        self.collision_policy = p;
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn set_bus_idle(&mut self) {
        self.hal.bus_set_high();
        // IEC 62386-101 §7.2.2
        self.idlecnt = self.high_run_ticks;
        self.rx_debounce = 0;
        self.bus_state = BusState::Idle;
        self.tx_busy.store(false, Ordering::Release);
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn enter_rx(&mut self) {
        self.rx_pre_idle = if self.idlecnt > 0xFF { 0xFF } else { self.idlecnt as u8 };
        self.rx_pos = 0;
        self.rx_bit_cnt = 0;
        self.rx_idle = 0;
        self.rx_byte = 0;
        self.rx_state = RxState::Receiving;
        self.bus_state = BusState::Rx;
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn finish_rx_and_notify(&mut self) {
        self.last_rx.sample_count = self.rx_pos;
        self.last_rx.samples = self.rx_data;
        self.last_rx.pre_idle_ticks = self.rx_pre_idle;
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn tick(&mut self) {
        let bus_is_high = if self.hal.bus_is_high() { 1u8 } else { 0u8 };
        self.low_run_ticks = if bus_is_high != 0 {
            0
        } else {
            self.low_run_ticks.saturating_add(1)
        };
        self.high_run_ticks = if bus_is_high != 0 {
            self.high_run_ticks.saturating_add(1)
        } else {
            0
        };
        match self.bus_state {
            BusState::Idle => self.tick_idle(bus_is_high),
            BusState::Rx => self.tick_rx(bus_is_high),
            BusState::Tx => self.tick_tx(bus_is_high),
            BusState::CollisionTx => self.tick_collision_tx(bus_is_high),
        }
    }

    #[inline]
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn tick_idle(&mut self, bus_is_high: u8) {
        if bus_is_high != 0 {
            self.rx_debounce = 0;
            if self.idlecnt != 0xFFFF {
                self.idlecnt = self.idlecnt.wrapping_add(1);
            }
            return;
        }
        self.rx_debounce = self.rx_debounce.saturating_add(1);
        if self.rx_debounce >= RX_START_DEBOUNCE_TICKS {
            self.enter_rx();
            for _ in 0..self.rx_debounce {
                self.tick_rx(0);
            }
        }
    }

    #[inline]
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn tick_tx_armed(&mut self, bus_is_high: u8) {
        if bus_is_high == 0 {
            self.tx_yielded = true;
            self.tx_armed = 0;
            self.set_bus_idle();
            self.tick_idle(bus_is_high);
            return;
        }
        self.tx_armed = self.tx_armed.saturating_sub(1);
    }

    #[inline]
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn tx_collision_detected(&self, bus_is_high: u8) -> bool {
        let policy_ok = (self.collision_policy == CollisionPolicy::On)
            || (self.collision_policy == CollisionPolicy::Auto
                && self.tx_hb_len != BACKWARD_FRAME_HALF_BITS);
        policy_ok
            && self.tx_high != 0
            && bus_is_high == 0
            && (self.tx_sp_cnt == 1 || self.tx_sp_cnt == 2)
    }

    #[inline]
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn enter_collision_tx(&mut self) {
        if self.tx_collision != 0xFF {
            self.tx_collision = self.tx_collision.wrapping_add(1);
        }
        self.tx_sp_cnt = 0;
        self.tx_idle_after_break = false;
        self.bus_state = BusState::CollisionTx;
    }

    #[inline]
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn tick_tx(&mut self, bus_is_high: u8) {
        if self.tx_armed != 0 {
            self.tick_tx_armed(bus_is_high);
            return;
        }
        if self.tx_hb_cnt >= self.tx_hb_len {
            self.set_bus_idle();
            return;
        }
        if self.tx_collision_detected(bus_is_high) {
            self.enter_collision_tx();
            return;
        }

        if self.tx_sp_cnt == 0 {
            let pos = usize::from(self.tx_hb_cnt >> 3);
            let Some(&byte) = self.tx_hb_data.get(pos) else {
                self.set_bus_idle();
                return;
            };
            let bitmask = 0x80u8 >> (self.tx_hb_cnt & 7);
            if (byte & bitmask) != 0 {
                self.hal.bus_set_low();
                self.tx_high = 0;
            } else {
                self.hal.bus_set_high();
                self.tx_high = 1;
            }
            self.tx_hb_cnt = self.tx_hb_cnt.wrapping_add(1);
            self.tx_sp_cnt = 4;
        }
        self.tx_sp_cnt = self.tx_sp_cnt.saturating_sub(1);
    }

    #[inline]
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn tick_collision_tx(&mut self, bus_is_high: u8) {
        self.tx_sp_cnt = self.tx_sp_cnt.wrapping_add(1);
        if self.tx_sp_cnt <= COLLISION_BREAK_TICKS {
            self.hal.bus_set_low();
            return;
        }
        self.hal.bus_set_high();
        if self.tx_sp_cnt < COLLISION_CHECK_TICK {
            return;
        }
        // IEC 62386-101 §9.1.4
        self.tx_idle_after_break = bus_is_high != 0;
        self.set_bus_idle();
        if bus_is_high == 0 {
            self.tick_idle(bus_is_high);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn tick_rx(&mut self, bus_is_high: u8) {
        self.rx_byte = (self.rx_byte << 1) | bus_is_high;
        self.rx_bit_cnt = self.rx_bit_cnt.wrapping_add(1);
        if self.rx_bit_cnt == 8 {
            let idx = core::cmp::min(usize::from(self.rx_pos), MAX_RX_IDX);
            if let Some(slot) = self.rx_data.get_mut(idx) {
                *slot = self.rx_byte;
            }
            self.rx_pos = self.rx_pos.saturating_add(1);
            if usize::from(self.rx_pos) > MAX_RX_IDX {
                self.rx_pos = MAX_RX_IDX as u16;
            }
            self.rx_bit_cnt = 0;
        }
        if bus_is_high != 0 {
            self.rx_idle = self.rx_idle.saturating_add(1);
            if self.rx_idle >= RX_IDLE_LINE_HIGH_TICKS {
                if let Some(slot) = self.rx_data.get_mut(usize::from(self.rx_pos)) {
                    *slot = 0xFF;
                }
                self.rx_pos = self.rx_pos.saturating_add(1);
                self.rx_state = RxState::Completed;
                self.set_bus_idle();
                self.finish_rx_and_notify();
            }
        } else {
            self.rx_idle = 0;
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn start_tx(&mut self, halfbits: &HalfBitBuffer) -> bool {
        if halfbits.length == 0 || halfbits.length > HalfBitBuffer::MAX_DATA_HALF_BITS {
            return false;
        }
        if self.bus_state != BusState::Idle {
            return false;
        }
        self.tx_pre_idle = self.idlecnt;
        self.tx_hb_data = halfbits.data;
        self.tx_hb_len = halfbits.length;
        self.tx_hb_cnt = 0;
        self.tx_sp_cnt = 0;
        self.tx_collision = 0;
        self.tx_idle_after_break = false;
        self.tx_yielded = false;
        self.tx_armed = TX_ARM_LEAD_TICKS;
        self.rx_state = RxState::Empty;
        self.bus_state = BusState::Tx;
        self.tx_busy.store(true, Ordering::Release);
        true
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn bus_state(&self) -> BusState {
        self.bus_state
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn line_debouncing(&self) -> bool {
        self.rx_debounce != 0
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn sample_bus_high(&mut self) -> bool {
        self.hal.bus_is_high()
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn poll_tx_state(&mut self) -> TxPollResult {
        if self.tx_yielded {
            self.tx_yielded = false;
            return TxPollResult::Yielded;
        }
        if self.tx_collision != 0 {
            self.tx_collision = 0;
            return TxPollResult::Collision;
        }
        if self.bus_state == BusState::Tx {
            return TxPollResult::Transmitting;
        }
        TxPollResult::Ok
    }

    // IEC 62386-101 §9.1.4
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn take_idle_after_break(&mut self) -> bool {
        let idle = self.tx_idle_after_break;
        self.tx_idle_after_break = false;
        idle
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn rx_state(&self) -> RxState {
        self.rx_state
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn idle_tick_count(&self) -> u16 {
        self.idlecnt
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn bus_low_run_ticks(&self) -> u16 {
        self.low_run_ticks
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn tx_pre_idle_ticks(&self) -> u16 {
        self.tx_pre_idle
    }

    pub fn rx_pre_idle_ticks(&self) -> u8 {
        self.rx_pre_idle
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn last_rx_event(&self) -> &RxCompletedEvent {
        &self.last_rx
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn acknowledge_rx_frame(&mut self) {
        if self.rx_state == RxState::Completed {
            self.rx_state = RxState::Empty;
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn take_completed_rx_event(&mut self) -> Option<RxCompletedEvent> {
        if self.rx_state != RxState::Completed {
            return None;
        }
        let ev = self.last_rx.clone();
        self.acknowledge_rx_frame();
        Some(ev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHal {
        bus_high: bool,
    }

    impl BitbangHal for TestHal {
        fn bus_is_high(&mut self) -> bool {
            self.bus_high
        }
        fn bus_set_low(&mut self) {}
        fn bus_set_high(&mut self) {}
    }

    struct RecordingHal {
        bus_high: bool,
        tick: u32,
        writes: Vec<(u32, bool)>,
    }

    impl BitbangHal for RecordingHal {
        fn bus_is_high(&mut self) -> bool {
            self.bus_high
        }
        fn bus_set_low(&mut self) {
            self.writes.push((self.tick, false));
        }
        fn bus_set_high(&mut self) {
            self.writes.push((self.tick, true));
        }
    }

    #[test]
    fn a_dominant_line_during_the_arming_lead_yields_without_writing_an_edge() {
        let mut phy = DaliBitbangPhy::new(RecordingHal {
            bus_high: true,
            tick: 0,
            writes: Vec::new(),
        });
        let halfbits = HalfBitBuffer {
            data: [0b0101_0101; 9],
            length: 16,
        };
        assert!(phy.start_tx(&halfbits));
        phy.hal.writes.clear();
        phy.hal.tick = 1;
        phy.tick();
        assert_eq!(phy.poll_tx_state(), TxPollResult::Transmitting);
        phy.hal.bus_high = false;
        phy.hal.tick = 2;
        phy.tick();
        assert_eq!(phy.poll_tx_state(), TxPollResult::Yielded);
        assert_ne!(phy.bus_state(), BusState::Tx, "the frame must have left Tx");
        assert!(
            phy.hal.writes.iter().all(|&(_, level)| level),
            "a yielded frame writes no dominant level: {:?}",
            phy.hal.writes
        );
        assert!(
            !phy.tx_busy.load(Ordering::Acquire),
            "tx_busy must clear on a yield"
        );
        assert_eq!(phy.poll_tx_state(), TxPollResult::Ok);
    }

    #[test]
    fn a_recessive_lead_still_transmits_the_frame() {
        let mut phy = DaliBitbangPhy::new(RecordingHal {
            bus_high: true,
            tick: 0,
            writes: Vec::new(),
        });
        let halfbits = HalfBitBuffer {
            data: [0b0101_0101; 9],
            length: 16,
        };
        assert!(phy.start_tx(&halfbits));
        phy.hal.writes.clear();
        for t in 1..=u32::from(TX_ARM_LEAD_TICKS) {
            phy.hal.tick = t;
            phy.tick();
            assert_eq!(phy.poll_tx_state(), TxPollResult::Transmitting);
        }
        assert!(phy.hal.writes.is_empty(), "the lead writes nothing");
        phy.hal.tick = u32::from(TX_ARM_LEAD_TICKS) + 1;
        phy.tick();
        assert_eq!(
            phy.hal.writes.len(),
            1,
            "the tick after the lead writes the opening edge"
        );
        assert_eq!(phy.poll_tx_state(), TxPollResult::Transmitting);
    }

    #[test]
    fn the_frames_opening_half_bit_lasts_as_long_as_the_others() {
        let mut phy = DaliBitbangPhy::new(RecordingHal {
            bus_high: true,
            tick: 0,
            writes: Vec::new(),
        });
        let halfbits = HalfBitBuffer {
            data: [0b0101_0101; 9],
            length: 16,
        };
        assert!(phy.start_tx(&halfbits));
        phy.hal.writes.clear();

        let hb_count = 16;
        for t in 0..(u32::from(TX_ARM_LEAD_TICKS) + hb_count * TX_HALF_BIT_TICKS) {
            phy.hal.tick = t;
            phy.tick();
        }

        let writes: Vec<u32> = phy.hal.writes.iter().map(|&(tick, _)| tick).collect();
        assert!(
            writes.len() > hb_count as usize,
            "expected {hb_count} half-bit writes plus the idle write, got {writes:?}"
        );
        let writes: Vec<u32> = writes[..(hb_count as usize)].to_vec();

        let spans: Vec<u32> = writes.windows(2).map(|w| w[1] - w[0]).collect();
        for (i, span) in spans.iter().enumerate() {
            assert_eq!(
                *span, TX_HALF_BIT_TICKS,
                "half-bit {i} spans {span} ticks, not {TX_HALF_BIT_TICKS} (spans: {spans:?})"
            );
        }
        assert_eq!(
            writes[0],
            u32::from(TX_ARM_LEAD_TICKS),
            "the opening level must be written once the arming tick's overrun has passed"
        );
    }

    #[test]
    fn a_full_length_frame_transmits_every_half_bit() {
        let mut phy = DaliBitbangPhy::new(RecordingHal {
            bus_high: true,
            tick: 0,
            writes: Vec::new(),
        });
        let halfbits = HalfBitBuffer {
            data: [0b0101_0101; 9],
            length: HalfBitBuffer::MAX_DATA_HALF_BITS,
        };
        assert!(phy.start_tx(&halfbits));
        phy.hal.writes.clear();

        let hb_count = u32::from(HalfBitBuffer::MAX_DATA_HALF_BITS);
        for t in 0..(u32::from(TX_ARM_LEAD_TICKS) + hb_count * TX_HALF_BIT_TICKS) {
            phy.hal.tick = t;
            phy.tick();
        }

        assert_eq!(
            phy.hal.writes.len(),
            hb_count as usize + 1,
            "every half-bit must be written exactly once, plus the idle write"
        );
        assert_eq!(phy.bus_state(), BusState::Idle);
    }

    #[test]
    fn init_idle_high() {
        let phy = DaliBitbangPhy::new(TestHal { bus_high: true });
        assert_eq!(phy.bus_state(), BusState::Idle);
        assert!(!phy.tx_busy.load(Ordering::Acquire));
    }

    #[test]
    fn single_tick_glitch_ignored() {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });

        phy.hal.bus_high = false;
        phy.tick();

        assert_eq!(phy.bus_state(), BusState::Idle);
        assert_eq!(phy.rx_state(), RxState::Empty);

        phy.hal.bus_high = true;
        phy.tick();
        assert_eq!(phy.bus_state(), BusState::Idle);
    }

    #[test]
    fn two_consecutive_low_enters_rx() {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });

        phy.hal.bus_high = false;
        phy.tick();
        assert_eq!(phy.bus_state(), BusState::Idle);

        phy.tick();
        assert_eq!(phy.bus_state(), BusState::Rx);
        assert_eq!(phy.rx_state(), RxState::Receiving);
    }

    #[test]
    fn auto_collision_detects_dominant_low_during_forward_high_half_bit() {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });
        let halfbits = HalfBitBuffer {
            data: [0; 9],
            length: 8,
        };

        assert!(phy.start_tx(&halfbits));

        for _ in 0..TX_ARM_LEAD_TICKS {
            phy.tick();
        }
        phy.tick();
        phy.hal.bus_high = false;
        phy.tick();
        phy.tick();

        assert_eq!(phy.bus_state(), BusState::CollisionTx);
        assert_eq!(phy.poll_tx_state(), TxPollResult::Collision);
    }

    #[test]
    fn the_collision_break_stays_within_table_25() {
        const TABLE_25_BREAK_US: (u32, u32) = (1_200, 1_400);
        let break_us = u32::from(COLLISION_BREAK_TICKS) * PHY_TICK_US;

        assert!(break_us >= TABLE_25_BREAK_US.0);
        assert!(break_us <= TABLE_25_BREAK_US.1);
    }

    #[test]
    fn tx_half_bit_timing_stays_within_multi_master_window() {
        let half_bit_us = PHY_TICK_US * TX_HALF_BIT_TICKS;
        let double_half_bit_us = half_bit_us * 2;

        assert!((400..=433).contains(&half_bit_us));
        assert!((800..=866).contains(&double_half_bit_us));
    }

    #[test]
    fn rx_completes_after_idle_high() {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });

        phy.hal.bus_high = false;
        phy.tick();
        phy.tick();

        assert_eq!(phy.bus_state(), BusState::Rx);

        phy.hal.bus_high = true;
        for _ in 0..20 {
            phy.tick();
        }

        assert_eq!(phy.rx_state(), RxState::Completed);
        assert_eq!(phy.bus_state(), BusState::Idle);
    }

    #[test]
    fn settling_after_a_reception_counts_from_the_last_edge() {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });
        phy.hal.bus_high = false;
        phy.tick();
        phy.tick();
        assert_eq!(phy.bus_state(), BusState::Rx);

        phy.hal.bus_high = true;
        for _ in 0..usize::from(RX_IDLE_LINE_HIGH_TICKS) {
            phy.tick();
        }
        assert_eq!(phy.bus_state(), BusState::Idle);

        assert!(
            phy.idle_tick_count() >= u16::from(RX_IDLE_LINE_HIGH_TICKS),
            "idle count restarted at {} after a reception — settling must be \
             counted from the last rising edge (§7.2.2), and the latch that \
             ends the frame is {} ticks after it",
            phy.idle_tick_count(),
            RX_IDLE_LINE_HIGH_TICKS
        );
    }

    #[test]
    fn settling_after_a_transmission_counts_from_the_last_edge() {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });
        for _ in 0..200 {
            phy.tick();
        }
        let hb = HalfBitBuffer {
            data: [0b1010_0000, 0, 0, 0, 0, 0, 0, 0, 0],
            length: 8,
        };
        assert!(phy.start_tx(&hb));
        for _ in 0..64 {
            phy.tick();
            if phy.bus_state() == BusState::Idle {
                break;
            }
        }
        assert_eq!(phy.bus_state(), BusState::Idle, "transmission never ended");
        assert!(
            phy.idle_tick_count() > 0,
            "idle count restarted at 0 after a transmission — the stop bits are \
             settling time that has already elapsed"
        );
    }

    #[test]
    fn a_received_frame_carries_the_idle_gap_that_preceded_it() {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });

        let idle_ticks_before_frame = 130u8;
        for _ in 0..idle_ticks_before_frame {
            phy.tick();
        }

        phy.hal.bus_high = false;
        phy.tick();
        phy.tick();
        assert_eq!(phy.bus_state(), BusState::Rx);

        phy.hal.bus_high = true;
        for _ in 0..20 {
            phy.tick();
        }
        assert_eq!(phy.rx_state(), RxState::Completed);

        assert_eq!(
            phy.last_rx_event().pre_idle_ticks,
            idle_ticks_before_frame,
            "the capture must carry the gap measured before the start bit"
        );
    }

    #[test]
    fn wire_settling_adds_the_arming_lead_for_our_own_frames() {
        assert_eq!(settle_ticks_on_wire(130, TX_ARM_LEAD_TICKS), 133);
        assert_eq!(settle_ticks_on_wire(130, 0), 130);
        assert_eq!(settle_ticks_on_wire(903, TX_ARM_LEAD_TICKS), 906);
        assert_eq!(settle_ticks_on_wire(u16::MAX, TX_ARM_LEAD_TICKS), u16::MAX);
    }

    fn frame_levels(bits: &[bool]) -> Vec<bool> {
        let mut levels = Vec::new();
        for &bit in core::iter::once(&true).chain(bits) {
            let (first, second) = if bit { (false, true) } else { (true, false) };
            levels.extend(core::iter::repeat_n(first, TX_HALF_BIT_TICKS as usize));
            levels.extend(core::iter::repeat_n(second, TX_HALF_BIT_TICKS as usize));
        }
        levels
    }

    fn bits(n: usize, last: bool) -> Vec<bool> {
        (0..n)
            .map(|i| if i + 1 == n { last } else { i % 2 == 0 })
            .collect()
    }

    fn overlapped(a: &[bool], b: &[bool], offset_ticks: usize) -> Vec<bool> {
        let len = a.len().max(offset_ticks + b.len());
        (0..len)
            .map(|t| {
                let la = a.get(t).copied().unwrap_or(true);
                let lb = t
                    .checked_sub(offset_ticks)
                    .and_then(|i| b.get(i).copied())
                    .unwrap_or(true);
                la && lb
            })
            .collect()
    }

    fn capture_of(levels: &[bool]) -> RxCompletedEvent {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });
        for _ in 0..40 {
            phy.tick();
        }
        for &level in levels {
            phy.hal.bus_high = level;
            phy.tick();
        }
        phy.hal.bus_high = true;
        for _ in 0..40 {
            phy.tick();
            if phy.rx_state() == RxState::Completed {
                break;
            }
        }
        phy.take_completed_rx_event().expect("capture completed")
    }

    #[test]
    fn captures_of_each_width_classify_by_length() {
        use crate::frame_length::{frame_length_class, FrameLengthClass};

        let answer = |last| frame_levels(&bits(8, last));
        let cases = [
            ("backward, last bit 1", answer(true), 11, FrameLengthClass::Backward),
            ("backward, last bit 0", answer(false), 12, FrameLengthClass::Backward),
            ("forward16, last bit 1", frame_levels(&bits(16, true)), 19, FrameLengthClass::Forward),
            ("forward16, last bit 0", frame_levels(&bits(16, false)), 20, FrameLengthClass::Forward),
            ("forward24, last bit 1", frame_levels(&bits(24, true)), 27, FrameLengthClass::Forward),
            ("forward24, last bit 0", frame_levels(&bits(24, false)), 28, FrameLengthClass::Forward),
            ("two answers 5,5 and 10,5 ms apart, last bit 1", overlapped(&answer(true), &answer(true), 48), 17, FrameLengthClass::Backward),
            ("two answers 5,5 and 10,5 ms apart, last bit 0", overlapped(&answer(false), &answer(false), 48), 18, FrameLengthClass::Backward),
        ];
        for (name, levels, expected_samples, expected_class) in cases {
            let ev = capture_of(&levels);
            assert_eq!(ev.sample_count, expected_samples, "{name}: sample count");
            assert_eq!(
                frame_length_class(ev.sample_count),
                expected_class,
                "{name}: class"
            );
        }
    }

    #[test]
    fn a_completed_capture_carries_the_samples_up_to_its_count() {
        let mut phy = DaliBitbangPhy::new(TestHal { bus_high: true });

        phy.hal.bus_high = false;
        phy.tick();
        phy.tick();
        phy.hal.bus_high = true;
        for _ in 0..20 {
            phy.tick();
        }

        let ev = phy.take_completed_rx_event().expect("capture completed");
        let n = usize::from(ev.sample_count).min(RxCompletedEvent::MAX_SAMPLES);
        assert!(n > 0, "a completed capture has at least one sample");
        assert_eq!(
            ev.samples[..n],
            phy.rx_data[..n],
            "the published prefix is the captured prefix"
        );
    }
}
