use core::ffi::c_void;
use core::mem::MaybeUninit;
use std::sync::{Mutex, OnceLock};

use esp_idf_svc::sys::{
    esp_timer_get_time, heap_caps_get_info, heap_caps_walk, multi_heap_info_t,
    walker_block_info_t, walker_heap_into_t, MALLOC_CAP_INTERNAL,
};

use crate::heap_census::{
    line_of, BootCensusLog, CensusText, HeapCensus, Span, StageRecord, HOLD_BUDGET_US,
    STAGE_SLOTS,
};
use crate::psram::PsramBox;

static STORE: OnceLock<Option<Mutex<PsramBox<BootCensusLog>>>> = OnceLock::new();

fn store() -> Option<&'static Mutex<PsramBox<BootCensusLog>>> {
    STORE.get_or_init(|| new_store().map(Mutex::new)).as_ref()
}

fn new_store() -> Option<PsramBox<BootCensusLog>> {
    // SAFETY: all-u32 fields, so a zeroed place is the empty log; `write_bytes` writes all of it once.
    unsafe {
        PsramBox::try_new_with(|place: &mut MaybeUninit<BootCensusLog>| {
            place.as_mut_ptr().write_bytes(0, 1);
        })
    }
}

pub fn record_boot_census(stage: &str) {
    let Some(store) = store() else {
        log::warn!("boot heap census [{stage}]: no PSRAM for the census table");
        return;
    };
    let Ok(mut log) = store.lock() else {
        return;
    };
    let Some(record) = log.open_stage(stage) else {
        log::warn!("boot heap census [{stage}]: more stages than the {STAGE_SLOTS} records hold");
        return;
    };
    fill(record);
}

pub fn flush_boot_census(max_lines: usize) {
    let Some(store) = store() else {
        return;
    };
    for _ in 0..max_lines {
        let Ok(mut log) = store.lock() else {
            return;
        };
        let Some((record, at)) = log.take_line() else {
            return;
        };
        let text = line_of(record, at);
        if !matches!(text, CensusText::Nothing) {
            log::warn!("{text}");
        }
    }
}

fn fill(record: &mut StageRecord) {
    record.census.hold_budget_us = hold_budget_us();
    let opened = now_us();
    record.blocks_before = internal_blocks();
    let armed = now_us();
    let walked = walk_into(&mut record.census);
    record.blocks_after = internal_blocks();
    record.walk_us = elapsed_us(armed, walked);
    record.info_us = elapsed_us(opened, armed);
    record.census.rank_top();
    note_rust_share(record);
}

fn walk_into(census: &mut HeapCensus) -> i64 {
    // SAFETY: read-only traversal; `tally` only bumps the census it is handed, which nothing else holds.
    unsafe {
        heap_caps_walk(MALLOC_CAP_INTERNAL, Some(tally), (&raw mut *census).cast());
    }
    let walked = now_us();
    census.close_span_holds(walked as u32);
    walked
}

fn hold_budget_us() -> u32 {
    option_env!("DALI2RUST_HEAP_CENSUS_HOLD_US")
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(HOLD_BUDGET_US)
}

fn note_rust_share(record: &mut StageRecord) {
    let figures = crate::rust_heap::figures();
    record.rust_internal_bytes = figures.internal_live_bytes;
    record.rust_psram_bytes = figures.psram_live_bytes;
    for (into, from) in record
        .rust_by_class
        .iter_mut()
        .zip(&figures.internal_live_by_class)
    {
        *into = *from;
    }
}

unsafe extern "C" fn tally(
    heap: walker_heap_into_t,
    block: walker_block_info_t,
    user: *mut c_void,
) -> bool {
    // SAFETY: `user` is the census pointer `walk_into` handed over, and the walk is synchronous.
    let census = unsafe { &mut *user.cast::<HeapCensus>() };
    let span = Span {
        start: heap.start as u32,
        end: heap.end as u32,
    };
    let opened = census.spans_seen;
    census.record(span, block.size as u32, block.used);
    if census.spans_seen != opened {
        census.stamp_span_open(now_us() as u32);
        return true;
    }
    if census.hold_is_spent(now_us() as u32) {
        census.cut_this_heap();
        return false;
    }
    true
}

fn internal_blocks() -> u32 {
    let mut info = multi_heap_info_t::default();
    // SAFETY: read-only heap statistics FFI; `info` is a live, exclusively borrowed struct.
    unsafe { heap_caps_get_info(&raw mut info, MALLOC_CAP_INTERNAL) };
    info.allocated_blocks as u32
}

fn now_us() -> i64 {
    // SAFETY: a read of the monotonic `esp_timer`, callable from any context.
    unsafe { esp_timer_get_time() }
}

fn elapsed_us(from: i64, to: i64) -> u32 {
    to.saturating_sub(from).max(0) as u32
}
