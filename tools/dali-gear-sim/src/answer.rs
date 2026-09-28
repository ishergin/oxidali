use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_dali_codec::codec::merge_dominant;
use dali2rust_dali_codec::rx_decode::{RxDecode, SniffedFrame};
use dali2rust_dali_phy::{RxCompletedEvent, BACKWARD_DATA_BITS, MIN_SAMPLES_FOR_DECODE, PHY_TICK_US};
use dali2rust_gear_model::GearFleet;
use dali2rust_platform::dali::TransferOutcome;

use crate::logerr;
use crate::logsink::{self, GearDigest, Level};
use crate::phy::GearPhy;

const RING_POLL: Duration = Duration::from_millis(1);

pub const SETTLE_BANDS: usize = 5;

// IEC 62386-101 Table 22
const SETTLE_BAND_MIN_TICKS: [u32; SETTLE_BANDS] = [
    13_500u32.div_ceil(PHY_TICK_US),
    14_900u32.div_ceil(PHY_TICK_US),
    16_300u32.div_ceil(PHY_TICK_US),
    17_900u32.div_ceil(PHY_TICK_US),
    19_500u32.div_ceil(PHY_TICK_US),
];

#[derive(Debug, Default, Clone, Copy)]
pub struct LoopStats {
    pub frames_heard: u64,
    pub forward16: u64,
    pub answered: u64,
    pub collisions: u64,
    pub cell_busy: u64,
    pub decode_failed: u64,
    pub other_width: u64,
    pub ring_dropped: u64,
    pub settle_bands: [u32; SETTLE_BANDS],
    pub below_p1_floor: u64,
}

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

fn collided(answers: &[u8]) -> bool {
    answers.windows(2).any(|pair| pair[0] != pair[1])
}

fn log_answer(answers: &[u8]) {
    match answers.first() {
        Some(byte) if !collided(answers) => logsink::backward(*byte, answers.len()),
        _ => logsink::collision(answers.len()),
    }
}

pub struct AnswerLoop {
    phy: &'static GearPhy,
    fleet: Arc<Mutex<GearFleet>>,
    stats: Arc<Mutex<LoopStats>>,
    before: Vec<GearDigest>,
    after: Vec<GearDigest>,
    enabled_device_type: Option<u8>,
    born: Instant,
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
            before: Vec::new(),
            after: Vec::new(),
            enabled_device_type: None,
            born: Instant::now(),
        }
    }

    pub fn run(mut self) -> ! {
        let mut captures = Vec::new();
        loop {
            while let Some(event) = self.phy.pop_rx() {
                captures.push(event);
            }
            let last = captures.len().saturating_sub(1);
            for (index, event) in captures.drain(..).enumerate() {
                self.on_capture(&event, index == last);
            }
            self.drain_ring_drops();
            // sleep-ok: the interrupt never wakes a task (ADR-003), so the capture ring is polled once per tick.
            std::thread::sleep(RING_POLL);
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

    fn on_capture(&mut self, event: &RxCompletedEvent, newest: bool) {
        self.bump(|s| s.frames_heard += 1);
        if event.sample_count < MIN_SAMPLES_FOR_DECODE {
            self.bump(|s| s.decode_failed += 1);
            return;
        }
        match event.decode_sniffed() {
            SniffedFrame::Forward16(frame) => {
                self.note_settle_band(event.pre_idle_ticks);
                self.bump(|s| s.forward16 += 1);
                self.act_on(event.rx_epoch, frame, newest);
            }
            SniffedFrame::Forward24(_) => {
                self.note_settle_band(event.pre_idle_ticks);
                self.bump(|s| s.other_width += 1);
            }
            SniffedFrame::Backward8(_) => self.bump(|s| s.other_width += 1),
            SniffedFrame::UnsupportedLength(bits) => {
                self.bump(|s| s.other_width += 1);
                if logsink::enabled(Level::Trace) {
                    logerr!("decode_unsupported bits={bits}");
                }
            }
            SniffedFrame::DecodeFailed => {
                self.bump(|s| s.decode_failed += 1);
                if logsink::enabled(Level::Trace) {
                    logerr!("decode_failed samples={}", event.sample_count);
                }
            }
        }
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

    fn act_on(&mut self, rx_epoch: u8, frame: u16, newest: bool) {
        let Some((answers, held)) = self.exchange(frame) else {
            return;
        };
        let staged = newest && !answers.is_empty() && self.stage(rx_epoch, &answers);
        logsink::forward(frame, self.enabled_device_type);
        if staged {
            log_answer(&answers);
        }
        logsink::log_changes(&self.before, &self.after);
        self.remember_device_type_latch(frame, held);
    }

    fn exchange(&mut self, frame: u16) -> Option<(Vec<u8>, bool)> {
        let Ok(mut fleet) = self.fleet.lock() else {
            logerr!("fleet_lock_poisoned");
            return None;
        };
        let logging = logsink::enabled(Level::Change);
        if logging {
            logsink::snapshot(&fleet, &mut self.before);
        }
        let expects = fleet.expects_backward(frame);
        fleet.advance_to_ms(u64::try_from(self.born.elapsed().as_millis()).unwrap_or(u64::MAX));
        let outcome = fleet.exchange(frame, expects);
        if logging {
            logsink::snapshot(&fleet, &mut self.after);
        }
        let held = fleet.any_gear_holds(frame);
        let answers = match outcome {
            TransferOutcome::NoAnswer => Vec::new(),
            _ => fleet.last_answers().to_vec(),
        };
        Some((answers, held))
    }

    fn stage(&mut self, rx_epoch: u8, answers: &[u8]) -> bool {
        let Some(buffer) = merge_dominant(answers, BACKWARD_DATA_BITS) else {
            return false;
        };
        if !self.phy.submit_answer(rx_epoch, &buffer) {
            self.bump(|s| s.cell_busy += 1);
            logerr!("answer_cell_busy the previous answer is still staged");
            return false;
        }
        let collided = collided(answers);
        self.bump(|s| {
            s.answered += 1;
            if collided {
                s.collisions += 1;
            }
        });
        true
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
}
