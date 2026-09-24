use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

pub const SIZE_CLASS_LIMITS: [usize; 5] = [64, 256, 1024, 4096, 16384];
pub const SIZE_CLASSES: usize = SIZE_CLASS_LIMITS.len() + 1;
pub const PSRAM_FIRST_FROM_BYTES: usize = 256;

static PSRAM_FIRST: AtomicBool = AtomicBool::new(true);

pub fn keep_rust_heap_internal() {
    PSRAM_FIRST.store(false, Relaxed);
}

pub fn psram_first_active() -> bool {
    cfg!(esp_idf_spiram_xip_from_psram) && PSRAM_FIRST.load(Relaxed)
}

pub fn prefers_psram(size: usize, active: bool) -> bool {
    active && size >= PSRAM_FIRST_FROM_BYTES
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RustHeapFigures {
    pub internal_live_bytes: u32,
    pub internal_peak_bytes: u32,
    pub psram_live_bytes: u32,
    pub psram_peak_bytes: u32,
    pub internal_live_by_class: [u32; SIZE_CLASSES],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Internal,
    Psram,
}

pub fn placement_of(addr: usize, psram_low: usize, psram_high: usize) -> Placement {
    if (psram_low..psram_high).contains(&addr) {
        Placement::Psram
    } else {
        Placement::Internal
    }
}

pub fn size_class(size: usize) -> usize {
    SIZE_CLASS_LIMITS
        .iter()
        .position(|&limit| size < limit)
        .unwrap_or(SIZE_CLASS_LIMITS.len())
}

fn bytes(size: usize) -> u32 {
    u32::try_from(size).unwrap_or(u32::MAX)
}

pub struct HeapLedger {
    internal_live: AtomicU32,
    internal_peak: AtomicU32,
    psram_live: AtomicU32,
    psram_peak: AtomicU32,
    internal_by_class: [AtomicU32; SIZE_CLASSES],
}

impl HeapLedger {
    pub const fn new() -> Self {
        Self {
            internal_live: AtomicU32::new(0),
            internal_peak: AtomicU32::new(0),
            psram_live: AtomicU32::new(0),
            psram_peak: AtomicU32::new(0),
            internal_by_class: [const { AtomicU32::new(0) }; SIZE_CLASSES],
        }
    }

    pub fn note_alloc(&self, placement: Placement, size: usize) {
        let size = bytes(size);
        let (live, peak) = self.pair(placement);
        let now = live.fetch_add(size, Relaxed).wrapping_add(size);
        peak.fetch_max(now, Relaxed);
        if placement == Placement::Internal {
            self.internal_by_class[size_class(size as usize)].fetch_add(size, Relaxed);
        }
    }

    pub fn note_free(&self, placement: Placement, size: usize) {
        let size = bytes(size);
        let (live, _) = self.pair(placement);
        live.fetch_sub(size, Relaxed);
        if placement == Placement::Internal {
            self.internal_by_class[size_class(size as usize)].fetch_sub(size, Relaxed);
        }
    }

    pub fn figures(&self) -> RustHeapFigures {
        let mut by_class = [0; SIZE_CLASSES];
        for (out, cell) in by_class.iter_mut().zip(&self.internal_by_class) {
            *out = cell.load(Relaxed);
        }
        RustHeapFigures {
            internal_live_bytes: self.internal_live.load(Relaxed),
            internal_peak_bytes: self.internal_peak.load(Relaxed),
            psram_live_bytes: self.psram_live.load(Relaxed),
            psram_peak_bytes: self.psram_peak.load(Relaxed),
            internal_live_by_class: by_class,
        }
    }

    fn pair(&self, placement: Placement) -> (&AtomicU32, &AtomicU32) {
        match placement {
            Placement::Internal => (&self.internal_live, &self.internal_peak),
            Placement::Psram => (&self.psram_live, &self.psram_peak),
        }
    }
}

impl Default for HeapLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "espidf")]
pub use esp::{figures, CountingAllocator};

#[cfg(not(target_os = "espidf"))]
pub fn figures() -> RustHeapFigures {
    RustHeapFigures::default()
}

#[cfg(target_os = "espidf")]
mod esp {
    use std::alloc::{GlobalAlloc, Layout, System};

    use esp_idf_svc::sys::{
        heap_caps_aligned_alloc, heap_caps_aligned_calloc, heap_caps_calloc, heap_caps_malloc,
        heap_caps_realloc, MALLOC_CAP_8BIT, MALLOC_CAP_SPIRAM, SOC_EXTRAM_HIGH, SOC_EXTRAM_LOW,
    };

    use super::{placement_of, prefers_psram, psram_first_active, HeapLedger, Placement, RustHeapFigures};

    const PSRAM_CAPS: u32 = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT;
    const HEAP_ALIGN: usize = 4;

    static LEDGER: HeapLedger = HeapLedger::new();

    pub fn figures() -> RustHeapFigures {
        LEDGER.figures()
    }

    fn placement(ptr: *mut u8) -> Placement {
        placement_of(ptr as usize, SOC_EXTRAM_LOW as usize, SOC_EXTRAM_HIGH as usize)
    }

    fn psram_alloc(layout: Layout, zeroed: bool) -> *mut u8 {
        let (size, align) = (layout.size(), layout.align());
        let padded = size.next_multiple_of(align.max(HEAP_ALIGN));
        // SAFETY: capability-tagged allocations of a valid size and a power-of-two alignment; `free` releases each.
        let ptr = unsafe {
            match (align <= HEAP_ALIGN, zeroed) {
                (true, false) => heap_caps_malloc(size, PSRAM_CAPS),
                (true, true) => heap_caps_calloc(1, size, PSRAM_CAPS),
                (false, false) => heap_caps_aligned_alloc(align, padded, PSRAM_CAPS),
                (false, true) => heap_caps_aligned_calloc(align, 1, padded, PSRAM_CAPS),
            }
        };
        ptr.cast()
    }

    fn place(layout: Layout, zeroed: bool) -> *mut u8 {
        if prefers_psram(layout.size(), psram_first_active()) {
            let ptr = psram_alloc(layout, zeroed);
            if !ptr.is_null() {
                return ptr;
            }
        }
        // SAFETY: the caller's layout contract is forwarded unchanged.
        unsafe {
            if zeroed {
                System.alloc_zeroed(layout)
            } else {
                System.alloc(layout)
            }
        }
    }

    pub struct CountingAllocator;

    impl CountingAllocator {
        unsafe fn move_to_psram(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            if placement(ptr) == Placement::Psram && layout.align() <= HEAP_ALIGN {
                // SAFETY: `ptr` is a live PSRAM block from this allocator; realloc keeps or copies it.
                let moved = unsafe { heap_caps_realloc(ptr.cast(), new_size, PSRAM_CAPS) }.cast::<u8>();
                if !moved.is_null() {
                    LEDGER.note_free(Placement::Psram, layout.size());
                    LEDGER.note_alloc(placement(moved), new_size);
                    return moved;
                }
            }
            // SAFETY: `layout.align()` is already a valid alignment and `new_size` comes from the caller's contract.
            let grown = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
            // SAFETY: a fresh allocation for `grown`; the old block is live for `layout.size()` bytes.
            unsafe {
                let fresh = self.alloc(grown);
                if !fresh.is_null() {
                    core::ptr::copy_nonoverlapping(ptr, fresh, layout.size().min(new_size));
                    self.dealloc(ptr, layout);
                }
                fresh
            }
        }
    }

    // SAFETY: blocks come from `System` or tagged ESP-IDF allocations, all released by `free`.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = place(layout, false);
            if !ptr.is_null() {
                LEDGER.note_alloc(placement(ptr), layout.size());
            }
            ptr
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let ptr = place(layout, true);
            if !ptr.is_null() {
                LEDGER.note_alloc(placement(ptr), layout.size());
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            LEDGER.note_free(placement(ptr), layout.size());
            // SAFETY: `free` releases a block from `System` or from any ESP-IDF capability allocation alike.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            if prefers_psram(new_size, psram_first_active()) {
                // SAFETY: the caller's `GlobalAlloc::realloc` contract is forwarded unchanged.
                return unsafe { self.move_to_psram(ptr, layout, new_size) };
            }
            let before = placement(ptr);
            // SAFETY: the caller's `GlobalAlloc::realloc` contract is forwarded unchanged.
            let moved = unsafe { System.realloc(ptr, layout, new_size) };
            if !moved.is_null() {
                LEDGER.note_free(before, layout.size());
                LEDGER.note_alloc(placement(moved), new_size);
            }
            moved
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOW: usize = 0x4800_0000;
    const HIGH: usize = 0x4C00_0000;

    #[test]
    fn an_address_in_the_external_window_is_psram_and_the_edges_are_exact() {
        assert_eq!(placement_of(LOW, LOW, HIGH), Placement::Psram);
        assert_eq!(placement_of(HIGH - 1, LOW, HIGH), Placement::Psram);
        assert_eq!(placement_of(HIGH, LOW, HIGH), Placement::Internal);
        assert_eq!(placement_of(0x4FF2_0000, LOW, HIGH), Placement::Internal);
    }

    #[test]
    fn a_size_class_is_the_first_limit_the_size_stays_under() {
        assert_eq!(size_class(0), 0);
        assert_eq!(size_class(63), 0);
        assert_eq!(size_class(64), 1);
        assert_eq!(size_class(16383), 4);
        assert_eq!(size_class(16384), 5);
        assert_eq!(size_class(usize::MAX), 5);
    }

    #[test]
    fn only_objects_from_the_threshold_prefer_psram_and_only_while_active() {
        assert!(!prefers_psram(PSRAM_FIRST_FROM_BYTES - 1, true));
        assert!(prefers_psram(PSRAM_FIRST_FROM_BYTES, true));
        assert!(!prefers_psram(1 << 20, false));
        assert!(!psram_first_active());
    }

    #[test]
    fn live_bytes_return_to_zero_and_the_peak_stays() {
        let ledger = HeapLedger::new();
        ledger.note_alloc(Placement::Internal, 100);
        ledger.note_alloc(Placement::Internal, 5000);
        ledger.note_alloc(Placement::Psram, 20000);
        ledger.note_free(Placement::Internal, 100);
        let mid = ledger.figures();
        assert_eq!(mid.internal_live_bytes, 5000);
        assert_eq!(mid.internal_peak_bytes, 5100);
        assert_eq!(mid.psram_live_bytes, 20000);
        assert_eq!(mid.internal_live_by_class, [0, 0, 0, 0, 5000, 0]);
        ledger.note_free(Placement::Internal, 5000);
        ledger.note_free(Placement::Psram, 20000);
        let end = ledger.figures();
        assert_eq!(end.internal_live_bytes, 0);
        assert_eq!(end.internal_peak_bytes, 5100);
        assert_eq!(end.psram_live_bytes, 0);
        assert_eq!(end.psram_peak_bytes, 20000);
        assert_eq!(end.internal_live_by_class, [0; SIZE_CLASSES]);
    }
}
