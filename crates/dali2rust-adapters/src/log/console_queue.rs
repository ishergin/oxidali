#![cfg_attr(not(target_os = "espidf"), allow(dead_code, reason = "only the ESP build calls this"))]

use core::sync::atomic::{AtomicU32, Ordering};

use dali2rust_bsp::psram::PsramBox;
use dali2rust_platform::logs::{ConsoleLogCounters, LogLevel};

pub const SLOT_BYTES: usize = 768;
const SLOT_COUNT: usize = 24;
const RESERVED_SLOTS: usize = 4;
const RESERVE_ATTEMPTS: usize = 4;

pub const fn slot_count() -> usize {
    SLOT_COUNT
}

const SLOT_EMPTY: u32 = 0;
const SLOT_WRITING: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopOutcome {
    Line(usize),
    Empty,
    Busy,
}

pub struct FillResult {
    pub len: usize,
    pub truncated: bool,
}

impl FillResult {
    pub const fn complete(len: usize) -> Self {
        Self {
            len,
            truncated: false,
        }
    }
}

struct QueueData {
    bytes: [[u8; SLOT_BYTES]; SLOT_COUNT],
}

pub struct ConsoleQueue {
    tail: AtomicU32,
    head: AtomicU32,
    slots: [AtomicU32; SLOT_COUNT],
    slot0: *mut [u8; SLOT_BYTES],
    _arena: PsramBox<QueueData>,
    counters: &'static ConsoleLogCounters,
}

// SAFETY: a slot is addressed only by its reserving producer or, once published, by the one consumer.
unsafe impl Sync for ConsoleQueue {}
// SAFETY: as above; the owned arena moves with the value.
unsafe impl Send for ConsoleQueue {}

impl ConsoleQueue {
    pub fn try_new(counters: &'static ConsoleLogCounters) -> Option<Self> {
        // SAFETY: all-zero is a valid empty `QueueData`, initialised in place in PSRAM.
        let mut arena: PsramBox<QueueData> = unsafe {
            PsramBox::try_new_with(|place| {
                core::ptr::write_bytes(place.as_mut_ptr(), 0, 1);
            })?
        };
        let slot0 = arena.bytes.as_mut_ptr();
        Some(Self {
            tail: AtomicU32::new(0),
            head: AtomicU32::new(0),
            slots: [const { AtomicU32::new(SLOT_EMPTY) }; SLOT_COUNT],
            slot0,
            _arena: arena,
            counters,
        })
    }

    fn depth_limit(level: LogLevel) -> u32 {
        let slots = SLOT_COUNT as u32;
        if level <= LogLevel::Warn {
            slots
        } else {
            slots - RESERVED_SLOTS as u32
        }
    }

    fn reserve(&self, level: LogLevel) -> Option<u32> {
        let limit = Self::depth_limit(level);
        for _ in 0..RESERVE_ATTEMPTS {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            if tail.wrapping_sub(head) >= limit {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            if self
                .tail
                .compare_exchange(tail, tail.wrapping_add(1), Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Some(tail);
            }
        }
        self.counters.busy.fetch_add(1, Ordering::Relaxed);
        self.counters.dropped.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub fn try_push(&self, level: LogLevel, fill: impl FnOnce(&mut [u8]) -> FillResult) -> bool {
        let Some(ticket) = self.reserve(level) else {
            return false;
        };
        let index = (ticket % SLOT_COUNT as u32) as usize;
        self.slots[index].store(SLOT_WRITING, Ordering::Relaxed);
        // SAFETY: this producer owns `index` (`WRITING`) until it publishes; `index < SLOT_COUNT` by the modulo.
        let slot = unsafe { &mut *self.slot0.add(index) };
        let filled = fill(slot);
        let len = filled.len.min(SLOT_BYTES);
        if filled.truncated || filled.len > SLOT_BYTES {
            self.counters.truncated.fetch_add(1, Ordering::Relaxed);
        }
        self.slots[index].store(len as u32 + 1, Ordering::Release);
        true
    }

    pub fn try_pop(&self, output: &mut [u8; SLOT_BYTES]) -> PopOutcome {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return PopOutcome::Empty;
        }
        let index = (head % SLOT_COUNT as u32) as usize;
        let state = self.slots[index].load(Ordering::Acquire);
        if state == SLOT_WRITING || state == SLOT_EMPTY {
            return PopOutcome::Busy;
        }
        let len = (state - 1) as usize;
        // SAFETY: the slot is published, so its producer is done with it, and this is the only consumer.
        let slot = unsafe { &*self.slot0.add(index) };
        output[..len].copy_from_slice(&slot[..len]);
        self.slots[index].store(SLOT_EMPTY, Ordering::Release);
        self.head.store(head.wrapping_add(1), Ordering::Release);
        PopOutcome::Line(len)
    }

    pub fn pop_in_place(&self, mut write: impl FnMut(&[u8])) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return false;
        }
        let index = (head % SLOT_COUNT as u32) as usize;
        let state = self.slots[index].load(Ordering::Acquire);
        if state == SLOT_WRITING || state == SLOT_EMPTY {
            return false;
        }
        let len = (state - 1) as usize;
        // SAFETY: as `try_pop`: published slot, single consumer; the borrow ends before the slot is released.
        let slot = unsafe { &*self.slot0.add(index) };
        write(&slot[..len]);
        self.slots[index].store(SLOT_EMPTY, Ordering::Release);
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    #[test]
    fn warn_uses_the_reserved_tail_after_info_is_refused() {
        static COUNTERS: ConsoleLogCounters = ConsoleLogCounters::new();
        let queue = ConsoleQueue::try_new(&COUNTERS).unwrap();
        for _ in 0..(SLOT_COUNT - RESERVED_SLOTS) {
            assert!(queue.try_push(LogLevel::Info, |slot| {
                slot[0] = b'i';
                FillResult::complete(1)
            }));
        }
        assert!(!queue.try_push(LogLevel::Info, |_| FillResult::complete(1)));
        for _ in 0..RESERVED_SLOTS {
            assert!(queue.try_push(LogLevel::Warn, |slot| {
                slot[0] = b'w';
                FillResult::complete(1)
            }));
        }
        assert!(!queue.try_push(LogLevel::Error, |_| FillResult::complete(1)));
    }

    #[test]
    fn an_empty_queue_is_not_a_reserved_one() {
        static POPS: ConsoleLogCounters = ConsoleLogCounters::new();
        let queue = ConsoleQueue::try_new(&POPS).unwrap();
        let mut line = [0u8; SLOT_BYTES];
        assert_eq!(queue.try_pop(&mut line), PopOutcome::Empty);
        let ticket = queue.reserve(LogLevel::Info).unwrap();
        queue.slots[ticket as usize].store(SLOT_WRITING, Ordering::Relaxed);
        assert_eq!(queue.try_pop(&mut line), PopOutcome::Busy);
        queue.slots[ticket as usize].store(2, Ordering::Release);
        assert_eq!(queue.try_pop(&mut line), PopOutcome::Line(1));
        assert_eq!(queue.try_pop(&mut line), PopOutcome::Empty);
        assert_eq!(POPS.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn producer_finishes_while_consumer_is_stopped() {
        static STOPPED: ConsoleLogCounters = ConsoleLogCounters::new();
        let queue = ConsoleQueue::try_new(&STOPPED).unwrap();
        for _ in 0..1000 {
            let _ = queue.try_push(LogLevel::Info, |slot| {
                slot[0] = b'x';
                FillResult::complete(1)
            });
        }
        assert!(STOPPED.dropped.load(Ordering::Relaxed) > 0);
        assert_eq!(STOPPED.busy.load(Ordering::Relaxed), 0);
    }

    #[cfg(test)]
    const RACE_LINE_BYTES: usize = 16;

    #[cfg(test)]
    fn race_producer(queue: &ConsoleQueue, byte: u8, rounds: usize) -> usize {
        let mut accepted = 0;
        for _ in 0..rounds {
            if queue.try_push(LogLevel::Info, |slot| {
                for cell in slot[..RACE_LINE_BYTES].iter_mut() {
                    *cell = byte;
                }
                FillResult::complete(RACE_LINE_BYTES)
            }) {
                accepted += 1;
            }
        }
        accepted
    }

    #[cfg(test)]
    fn race_consumer(queue: &ConsoleQueue, producers_done: &AtomicUsize, producers: usize) -> usize {
        let mut drained = 0;
        let mut line = [0u8; SLOT_BYTES];
        loop {
            match queue.try_pop(&mut line) {
                PopOutcome::Line(len) => {
                    let first = line[0];
                    assert_eq!(len, RACE_LINE_BYTES, "a line came out with the wrong length");
                    assert!(
                        line[..len].iter().all(|b| *b == first),
                        "a line was assembled from two producers"
                    );
                    drained += 1;
                }
                PopOutcome::Busy => std::thread::yield_now(),
                PopOutcome::Empty => {
                    if producers_done.load(Ordering::Acquire) == producers {
                        return drained;
                    }
                    std::thread::yield_now();
                }
            }
        }
    }

    #[test]
    fn concurrent_producers_never_interleave_a_line() {
        static RACE: ConsoleLogCounters = ConsoleLogCounters::new();
        const PRODUCERS: usize = 4;
        const PER_PRODUCER: usize = 500;

        let queue = Arc::new(ConsoleQueue::try_new(&RACE).unwrap());
        let done = Arc::new(AtomicUsize::new(0));
        let reader = {
            let (queue, done) = (queue.clone(), done.clone());
            std::thread::spawn(move || race_consumer(&queue, &done, PRODUCERS))
        };
        let writers: Vec<_> = (0..PRODUCERS)
            .map(|id| {
                let (queue, done) = (queue.clone(), done.clone());
                std::thread::spawn(move || {
                    let taken = race_producer(&queue, b'a' + id as u8, PER_PRODUCER);
                    done.fetch_add(1, Ordering::Release);
                    taken
                })
            })
            .collect();

        let accepted: usize = writers.into_iter().map(|w| w.join().unwrap()).sum();
        let drained = reader.join().unwrap();
        assert_eq!(
            accepted, drained,
            "every accepted line must reach the consumer exactly once"
        );
        assert!(accepted > 0);
    }
}
