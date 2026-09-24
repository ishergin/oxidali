use core::sync::atomic::{AtomicU32, Ordering};

use esp_idf_svc::sys::{
    esp_timer_get_time, heap_caps_free, heap_caps_malloc, MALLOC_CAP_8BIT, MALLOC_CAP_SPIRAM,
    SOC_EXTRAM_HIGH, SOC_EXTRAM_LOW,
};

use crate::rust_heap::{placement_of, Placement};

const ROUNDS_PER_CORE: u32 = 100_000;
const CELLS: usize = 2;
const OTHER_CORE: u8 = 1;
const HELPER_STACK_BYTES: usize = 3 * 1024;

type Cells = [AtomicU32; CELLS];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomicSelftest {
    pub expected: u32,
    pub fetch_add_total: u32,
    pub compare_exchange_total: u32,
    pub cell_in_psram: bool,
    pub elapsed_us: i64,
}

impl AtomicSelftest {
    pub fn passed(&self) -> bool {
        self.cell_in_psram
            && self.fetch_add_total == self.expected
            && self.compare_exchange_total == self.expected
    }
}

struct PsramCells(*mut Cells);

impl PsramCells {
    fn allocate() -> Option<Self> {
        // SAFETY: plain capability-tagged allocation, released in `Drop`.
        let raw = unsafe { heap_caps_malloc(core::mem::size_of::<Cells>(), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT) };
        let cells = raw.cast::<Cells>();
        if cells.is_null() {
            return None;
        }
        // SAFETY: a fresh allocation sized and aligned for the cells; zero is a valid `AtomicU32`.
        unsafe { cells.write([const { AtomicU32::new(0) }; CELLS]) };
        Some(Self(cells))
    }

    fn get(&self) -> &Cells {
        // SAFETY: allocated and initialised in `allocate`, freed only in `Drop`.
        unsafe { &*self.0 }
    }

    fn in_psram(&self) -> bool {
        placement_of(self.0 as usize, SOC_EXTRAM_LOW as usize, SOC_EXTRAM_HIGH as usize) == Placement::Psram
    }
}

impl Drop for PsramCells {
    fn drop(&mut self) {
        // SAFETY: allocated by `heap_caps_malloc` in `allocate`; every user has been joined.
        unsafe { heap_caps_free(self.0.cast()) };
    }
}

fn hammer(cells: &Cells) {
    for _ in 0..ROUNDS_PER_CORE {
        cells[0].fetch_add(1, Ordering::AcqRel);
        let mut seen = cells[1].load(Ordering::Acquire);
        while let Err(now) = cells[1].compare_exchange_weak(
            seen,
            seen.wrapping_add(1),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            seen = now;
        }
    }
}

fn race_both_cores(cells: &PsramCells) -> bool {
    let address = cells.0 as usize;
    let helper = crate::esp_thread::try_spawn_named_stack_on_core(
        c"psram-selftest",
        HELPER_STACK_BYTES,
        OTHER_CORE,
        move || {
            // SAFETY: the cells outlive the helper, which is joined before they are freed.
            hammer(unsafe { &*(address as *const Cells) });
        },
    );
    hammer(cells.get());
    helper.map(|h| h.join().is_ok()).unwrap_or(false)
}

pub fn run() -> Option<AtomicSelftest> {
    let cells = PsramCells::allocate()?;
    // SAFETY: monotonic µs clock, callable from any task.
    let started = unsafe { esp_timer_get_time() };
    let both_cores = race_both_cores(&cells);
    // SAFETY: monotonic µs clock, callable from any task.
    let elapsed_us = unsafe { esp_timer_get_time() } - started;
    Some(AtomicSelftest {
        expected: ROUNDS_PER_CORE * if both_cores { 2 } else { 1 },
        fetch_add_total: cells.get()[0].load(Ordering::Acquire),
        compare_exchange_total: cells.get()[1].load(Ordering::Acquire),
        cell_in_psram: cells.in_psram(),
        elapsed_us,
    })
}
