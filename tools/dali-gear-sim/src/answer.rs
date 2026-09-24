use std::sync::{Arc, Mutex};

use dali2rust_dali_codec::codec::merge_dominant;
use dali2rust_dali_codec::rx_decode::{RxDecode, SniffedFrame};
use dali2rust_dali_phy::{
    HalfBitBuffer, PHY_TICK_US, TX_ARM_LEAD_TICKS,
};
use dali2rust_gear_model::GearFleet;
use dali2rust_platform::dali::TransferOutcome;

use crate::logsink::{self, GearDigest, Level};
use crate::phy::GearPhy;
use crate::{logerr, note};

// IEC 62386-101 Table 20
const BACKWARD_MIN_US: u32 = 5_500;
const BACKWARD_MAX_US: u32 = 10_500;
const BACKWARD_TARGET_US: u32 = 6_500;

const SUBMIT_TO_EDGE_TICKS: u32 = 1 + TX_ARM_LEAD_TICKS as u32;
const FIXED_TICKS: u32 = SUBMIT_TO_EDGE_TICKS;

const fn us_to_ticks(us: u32) -> u32 {
    us / PHY_TICK_US
}

pub const TARGET_SUBMIT_IDLE_TICKS: u32 = us_to_ticks(BACKWARD_TARGET_US) - FIXED_TICKS;
pub const MIN_SUBMIT_IDLE_TICKS: u32 = us_to_ticks(BACKWARD_MIN_US) - FIXED_TICKS;
pub const MAX_SUBMIT_IDLE_TICKS: u32 = us_to_ticks(BACKWARD_MAX_US) - FIXED_TICKS;

const _: () = assert!(TARGET_SUBMIT_IDLE_TICKS > MIN_SUBMIT_IDLE_TICKS);
const _: () = assert!(TARGET_SUBMIT_IDLE_TICKS < MAX_SUBMIT_IDLE_TICKS);
const FIRST_EDGE_US: u32 = (FIXED_TICKS + TARGET_SUBMIT_IDLE_TICKS) * PHY_TICK_US;
const _: () = assert!(FIRST_EDGE_US >= BACKWARD_MIN_US);
const _: () = assert!(FIRST_EDGE_US <= BACKWARD_MAX_US);

const BACKWARD_DATA_BITS: u8 = 8;

const MIN_SAMPLES_FOR_DECODE: u16 = 8;

const COARSE_WAIT_TICK_MARGIN: u32 = 20;

#[derive(Debug, Default, Clone, Copy)]
pub struct LoopStats {
    pub frames_heard: u64,
    pub forward16: u64,
    pub answered: u64,
    pub collisions: u64,
    pub aborted: u64,
    pub rejected: u64,
    pub tx_collided: u64,
    pub decode_failed: u64,
    pub other_width: u64,
    pub ring_dropped: u64,
    pub submit_ticks: [u32; SUBMIT_HISTOGRAM_BUCKETS],
    pub submit_out_of_window: u64,
    pub settle_bands: [u32; SETTLE_BANDS],
    pub below_p1_floor: u64,
}

pub const SETTLE_BANDS: usize = 5;

// IEC 62386-101 Table 22
const SETTLE_BAND_MIN_TICKS: [u32; 5] = [
    13_500u32.div_ceil(PHY_TICK_US),
    14_900u32.div_ceil(PHY_TICK_US),
    16_300u32.div_ceil(PHY_TICK_US),
    17_900u32.div_ceil(PHY_TICK_US),
    19_500u32.div_ceil(PHY_TICK_US),
];

fn settle_band(pre_idle_ticks: u8) -> Option<usize> {
    let ticks = u32::from(pre_idle_ticks);
    if ticks < SETTLE_BAND_MIN_TICKS[0] {
        return None;
    }
    Some(
        SETTLE_BAND_MIN_TICKS
            .iter()
            .rposition(|min| ticks >= *min)
            .unwrap_or(0),
    )
}

pub const SUBMIT_HISTOGRAM_BUCKETS: usize = 16;
const SUBMIT_HISTOGRAM_TICKS_PER_BUCKET: u32 = 8;

pub struct AnswerLoop {
    phy: &'static GearPhy,
    fleet: Arc<Mutex<GearFleet>>,
    stats: Arc<Mutex<LoopStats>>,
    digests: Vec<GearDigest>,
    enabled_device_type: Option<u8>,
}

impl AnswerLoop {
    pub fn new(
        phy: &'static GearPhy,
        fleet: Arc<Mutex<GearFleet>>,
        stats: Arc<Mutex<LoopStats>>,
    ) -> Self {
        Self {
            phy,
            fleet,
            stats,
            digests: Vec::new(),
            enabled_device_type: None,
        }
    }

    pub fn run(mut self) -> ! {
        note!(
            "answer window: submit at idle_ticks={} (window {}..{}), \
             {} µs/tick, first edge {} ticks after submit",
            TARGET_SUBMIT_IDLE_TICKS,
            MIN_SUBMIT_IDLE_TICKS,
            MAX_SUBMIT_IDLE_TICKS,
            PHY_TICK_US,
            SUBMIT_TO_EDGE_TICKS
        );
        loop {
            match self.phy.pop_rx() {
                Some(event) => self.on_capture(&event),
                None => {
                    self.drain_ring_drops();
                    self.drain_session_events();
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    fn drain_ring_drops(&self) {
        let dropped = self.phy.take_sniff_dropped();
        if dropped > 0 {
            self.bump(|s| s.ring_dropped += u64::from(dropped));
            logerr!("rx_ring_full dropped={dropped}");
        }
    }

    fn bump(&self, f: impl FnOnce(&mut LoopStats)) {
        if let Ok(mut stats) = self.stats.lock() {
            f(&mut stats);
        }
    }

    fn on_capture(&mut self, event: &dali2rust_dali_phy::RxCompletedEvent) {
        self.bump(|s| s.frames_heard += 1);
        if event.sample_count < MIN_SAMPLES_FOR_DECODE {
            self.bump(|s| s.decode_failed += 1);
            return;
        }
        let frame = match event.decode_sniffed() {
            SniffedFrame::Forward16(frame) => {
                self.note_settle_band(event.pre_idle_ticks);
                frame
            }
            SniffedFrame::Forward24(_) => {
                self.note_settle_band(event.pre_idle_ticks);
                self.bump(|s| s.other_width += 1);
                return;
            }
            SniffedFrame::Backward8(_) => {
                self.bump(|s| s.other_width += 1);
                return;
            }
            SniffedFrame::UnsupportedLength(bits) => {
                self.bump(|s| s.other_width += 1);
                if logsink::enabled(Level::Trace) {
                    logerr!("decode_unsupported bits={bits}");
                }
                return;
            }
            SniffedFrame::DecodeFailed => {
                self.bump(|s| s.decode_failed += 1);
                if logsink::enabled(Level::Trace) {
                    logerr!("decode_failed samples={}", event.sample_count);
                }
                return;
            }
        };
        self.bump(|s| s.forward16 += 1);
        logsink::forward(frame, self.enabled_device_type);
        let held = self.act_on(frame);
        self.remember_device_type_latch(frame, held);
    }

    fn note_settle_band(&mut self, pre_idle_ticks: u8) {
        match settle_band(pre_idle_ticks) {
            Some(band) => self.bump(|s| {
                if let Some(slot) = s.settle_bands.get_mut(band) {
                    *slot += 1;
                }
            }),
            None => self.bump(|s| s.below_p1_floor += 1),
        }
    }

    // IEC 62386-102 §11.7.14
    fn remember_device_type_latch(&mut self, frame: u16, held: bool) {
        const ENABLE_DEVICE_TYPE_ADDRESS: u8 = 0xC1;
        if (frame >> 8) as u8 == ENABLE_DEVICE_TYPE_ADDRESS {
            self.enabled_device_type = Some(frame as u8);
        } else if !held {
            self.enabled_device_type = None;
        }
    }

    fn act_on(&mut self, frame: u16) -> bool {
        let (answers, held) = {
            let Ok(mut fleet) = self.fleet.lock() else {
                logerr!("fleet_lock_poisoned");
                return false;
            };
            logsink::snapshot(&fleet, &mut self.digests);
            let expects = fleet.expects_backward(frame);
            let outcome = fleet.exchange(frame, expects);
            logsink::log_changes(&self.digests, &fleet);
            let held = fleet.any_gear_holds(frame);
            match outcome {
                TransferOutcome::NoAnswer => return held,
                _ => (fleet.last_answers().to_vec(), held),
            }
        };
        self.answer(&answers);
        held
    }

    fn answer(&mut self, answers: &[u8]) {
        let Some(buffer) = merge_dominant(answers, BACKWARD_DATA_BITS) else {
            return;
        };
        let collided = answers.iter().any(|b| *b != answers[0]);
        match self.wait_for_window() {
            Some(at) => self.transmit(&buffer, answers, at, collided),
            None => {
                self.bump(|s| s.aborted += 1);
                if logsink::enabled(Level::Frame) {
                    logerr!("window_lost bus went active before the answer was due");
                }
            }
        }
    }

    fn transmit(&mut self, buffer: &HalfBitBuffer, answers: &[u8], at: u32, collided: bool) {
        if !self.phy.submit_backward(&buffer.data, buffer.length) {
            self.bump(|s| s.rejected += 1);
            logerr!("tx_rejected the interrupt still held the previous frame");
            return;
        }
        let in_window = (MIN_SUBMIT_IDLE_TICKS..=MAX_SUBMIT_IDLE_TICKS).contains(&at);
        self.bump(|s| {
            s.answered += 1;
            if collided {
                s.collisions += 1;
            }
            if !in_window {
                s.submit_out_of_window += 1;
            }
            let bucket = (at / SUBMIT_HISTOGRAM_TICKS_PER_BUCKET) as usize;
            if let Some(slot) = s.submit_ticks.get_mut(bucket) {
                *slot += 1;
            }
        });
        if collided {
            logsink::collision(answers.len());
        } else {
            logsink::backward(answers[0], answers.len(), at);
        }
        if !in_window {
            logerr!(
                "window_missed idle_ticks={at} not in {MIN_SUBMIT_IDLE_TICKS}..={MAX_SUBMIT_IDLE_TICKS}"
            );
        }
    }

    fn wait_for_window(&self) -> Option<u32> {
        let deadline = logsink::now_us() + WAIT_GIVE_UP_US;
        let mut previous = self.phy.idle_ticks();
        loop {
            let now = self.phy.idle_ticks();
            if now < previous {
                return None;
            }
            if now >= TARGET_SUBMIT_IDLE_TICKS && self.bus_still_idle(now) {
                return Some(now);
            }
            if logsink::now_us() > deadline {
                return None;
            }
            previous = now;
            if TARGET_SUBMIT_IDLE_TICKS.saturating_sub(now) > COARSE_WAIT_TICK_MARGIN {
                std::thread::sleep(std::time::Duration::from_millis(1));
            } else {
                // SAFETY: ROM busy-wait, no state, callable from any task.
                unsafe { esp_idf_svc::sys::esp_rom_delay_us(FINE_WAIT_US) };
            }
        }
    }

    fn bus_still_idle(&self, reading: u32) -> bool {
        // SAFETY: ROM busy-wait, no state, callable from any task.
        unsafe { esp_idf_svc::sys::esp_rom_delay_us(CONFIRM_IDLE_US) };
        self.phy.idle_ticks() > reading
    }

    fn drain_session_events(&self) {
        while let Some(event) = self.phy.pop_session_event() {
            match event {
                dali2rust_dali_phy::SessionEvent::TxCollision => {
                    self.bump(|s| s.tx_collided += 1);
                    logerr!("tx_collision our answer overlapped another transmitter");
                }
                dali2rust_dali_phy::SessionEvent::TxRejected => {
                    self.bump(|s| s.rejected += 1);
                    logerr!("tx_rejected the phy refused the frame (bus not idle)");
                }
                _ => {}
            }
        }
    }
}

const FINE_WAIT_US: u32 = 20;
const CONFIRM_IDLE_US: u32 = 2 * PHY_TICK_US;
const WAIT_GIVE_UP_US: i64 = 40_000;
