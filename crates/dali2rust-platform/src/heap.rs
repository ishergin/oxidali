#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeapStats {
    pub free_bytes: u32,
    pub largest_free_block_bytes: u32,
    pub min_free_ever_bytes: u32,
    pub internal_free_bytes: u32,
    pub internal_largest_block_bytes: u32,
    pub internal_min_free_bytes: u32,
    pub internal_total_bytes: u32,
    pub internal_allocated_blocks: u32,
    pub rust_internal_live_bytes: u32,
    pub rust_internal_peak_bytes: u32,
    pub rust_psram_live_bytes: u32,
    pub rust_psram_peak_bytes: u32,
}

pub trait HeapStatsPort: Send + Sync {
    fn snapshot(&self) -> HeapStats;
}
