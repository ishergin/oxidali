use core::sync::atomic::{AtomicU32, Ordering};

use dali2rust_platform::heap::{HeapStats, HeapStatsPort};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EspHeapStats;

static LARGEST_8BIT: AtomicU32 = AtomicU32::new(0);
static LARGEST_INTERNAL: AtomicU32 = AtomicU32::new(0);
static TOTAL_INTERNAL: AtomicU32 = AtomicU32::new(0);
static BLOCKS_INTERNAL: AtomicU32 = AtomicU32::new(0);

impl EspHeapStats {
    pub fn boot_snapshot(&self) -> HeapStats {
        use esp_idf_svc::sys::{
            heap_caps_get_info, heap_caps_get_largest_free_block, multi_heap_info_t,
            MALLOC_CAP_8BIT, MALLOC_CAP_INTERNAL,
        };

        let mut info = multi_heap_info_t::default();
        // SAFETY: read-only heap statistics FFI callable from any task; `info` is a live, exclusively borrowed struct.
        let largest = unsafe {
            heap_caps_get_info(&raw mut info, MALLOC_CAP_INTERNAL);
            heap_caps_get_largest_free_block(MALLOC_CAP_8BIT) as u32
        };
        LARGEST_8BIT.store(largest, Ordering::Relaxed);
        LARGEST_INTERNAL.store(info.largest_free_block as u32, Ordering::Relaxed);
        TOTAL_INTERNAL.store(
            (info.total_free_bytes as u32).saturating_add(info.total_allocated_bytes as u32),
            Ordering::Relaxed,
        );
        BLOCKS_INTERNAL.store(info.allocated_blocks as u32, Ordering::Relaxed);
        self.snapshot()
    }
}

impl HeapStatsPort for EspHeapStats {
    fn snapshot(&self) -> HeapStats {
        use esp_idf_svc::sys::{
            esp_get_minimum_free_heap_size, heap_caps_get_free_size,
            heap_caps_get_minimum_free_size, MALLOC_CAP_8BIT, MALLOC_CAP_INTERNAL,
        };

        let rust = crate::rust_heap::figures();
        // SAFETY: O(1) reads of each heap's own free and minimum-free counters, callable from any task.
        unsafe {
            HeapStats {
                free_bytes: heap_caps_get_free_size(MALLOC_CAP_8BIT) as u32,
                largest_free_block_bytes: LARGEST_8BIT.load(Ordering::Relaxed),
                min_free_ever_bytes: esp_get_minimum_free_heap_size(),
                internal_free_bytes: heap_caps_get_free_size(MALLOC_CAP_INTERNAL) as u32,
                internal_largest_block_bytes: LARGEST_INTERNAL.load(Ordering::Relaxed),
                internal_min_free_bytes: heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL)
                    as u32,
                internal_total_bytes: TOTAL_INTERNAL.load(Ordering::Relaxed),
                internal_allocated_blocks: BLOCKS_INTERNAL.load(Ordering::Relaxed),
                rust_internal_live_bytes: rust.internal_live_bytes,
                rust_internal_peak_bytes: rust.internal_peak_bytes,
                rust_psram_live_bytes: rust.psram_live_bytes,
                rust_psram_peak_bytes: rust.psram_peak_bytes,
            }
        }
    }
}
