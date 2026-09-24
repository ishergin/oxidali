use dali2rust_platform::slice_store::StoreError;
use esp_idf_svc::sys::{
    esp_partition_erase_range, esp_partition_find_first, esp_partition_read,
    esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_ANY, esp_partition_t,
    esp_partition_type_t_ESP_PARTITION_TYPE_DATA, esp_partition_write, esp_rom_crc32_le,
    heap_caps_free, heap_caps_malloc, ESP_OK, MALLOC_CAP_8BIT, MALLOC_CAP_INTERNAL, SOC_DRAM_HIGH,
    SOC_DRAM_LOW,
};
#[cfg(not(esp_idf_spiram_xip_from_psram))]
use esp_idf_svc::sys::{
    esp_partition_mmap, esp_partition_mmap_handle_t,
    esp_partition_mmap_memory_t_ESP_PARTITION_MMAP_DATA,
};

use crate::slice_layout::{LAYOUT_BYTES, SECTOR_BYTES};
use crate::slice_store_core::{RawFlash, SliceStoreCore};

const PARTITION_LABEL: &[u8] = b"storage\0";

const WRITE_BOUNCE_BYTES: usize = 1024;

struct EspPartitionFlash {
    partition: *const esp_partition_t,
    mapped: *const u8,
    mapped_len: usize,
}

// SAFETY: a static partition descriptor and a read-only mapping never unmapped; partition APIs lock internally.
unsafe impl Send for EspPartitionFlash {}
unsafe impl Sync for EspPartitionFlash {}

#[cfg(esp_idf_spiram_xip_from_psram)]
fn map_layout(_partition: *const esp_partition_t) -> *const u8 {
    log::info!("storage: reads take the flash driver (XIP from PSRAM: the cache never turns off)");
    core::ptr::null()
}

#[cfg(not(esp_idf_spiram_xip_from_psram))]
fn map_layout(partition: *const esp_partition_t) -> *const u8 {
    let mut ptr: *const core::ffi::c_void = core::ptr::null();
    let mut handle: esp_partition_mmap_handle_t = 0;
    // SAFETY: a valid descriptor, a range checked by `mount`, local out-pointers; the mapping lives for the process.
    let err = unsafe {
        esp_partition_mmap(
            partition,
            0,
            LAYOUT_BYTES as usize,
            esp_partition_mmap_memory_t_ESP_PARTITION_MMAP_DATA,
            &mut ptr,
            &mut handle,
        )
    };
    if err == ESP_OK {
        ptr.cast()
    } else {
        log::warn!("storage: layout not mapped ({err}); reads take the flash driver path");
        core::ptr::null()
    }
}

struct DramBounce {
    ptr: *mut u8,
    len: usize,
}

impl DramBounce {
    fn new(len: usize) -> Result<Self, StoreError> {
        // SAFETY: an ordinary caps-heap allocation; checked for null below.
        let ptr = unsafe { heap_caps_malloc(len, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) };
        if ptr.is_null() {
            return Err(StoreError::Backend(format!("no {len} B of DRAM for a bounce")));
        }
        Ok(Self { ptr: ptr.cast(), len })
    }

    fn fill(&mut self, chunk: &[u8]) -> *const u8 {
        // SAFETY: `chunk.len() <= self.len` by the caller's chunking; the buffer holds `len` bytes and cannot overlap.
        unsafe { core::ptr::copy_nonoverlapping(chunk.as_ptr(), self.ptr, chunk.len()) };
        self.ptr
    }
}

impl Drop for DramBounce {
    fn drop(&mut self) {
        // SAFETY: allocated by `heap_caps_malloc` in `new`, freed once.
        unsafe { heap_caps_free(self.ptr.cast()) };
    }
}

fn in_dram(buf: &[u8]) -> bool {
    let start = buf.as_ptr() as usize;
    start >= SOC_DRAM_LOW as usize && start.saturating_add(buf.len()) <= SOC_DRAM_HIGH as usize
}

fn check(err: i32, op: &str) -> Result<(), StoreError> {
    if err == ESP_OK {
        Ok(())
    } else {
        Err(StoreError::Backend(format!("partition {op} failed: {err}")))
    }
}

#[cfg(esp_idf_spiram_xip_from_psram)]
fn hold_for_wire_gap() {
}

#[cfg(not(esp_idf_spiram_xip_from_psram))]
fn hold_for_wire_gap() {
    use dali2rust_platform::dali::{
        await_wire_gap, note_persist_gate, wire_busy_for_persist, PERSIST_GATE_BUDGET_MS,
    };
    const STEP_MS: u32 = 1;
    let outcome = await_wire_gap(
        wire_busy_for_persist,
        // sleep-ok: one FreeRTOS tick at 1 kHz, only while the wire is busy
        |ms| std::thread::sleep(std::time::Duration::from_millis(u64::from(ms))),
        STEP_MS,
        PERSIST_GATE_BUDGET_MS,
    );
    note_persist_gate(outcome);
}

impl EspPartitionFlash {
    fn read_through_driver(&self, offset: u32, buf: &mut [u8]) -> Result<(), StoreError> {
        if in_dram(buf) {
            return self.read_direct(offset, buf);
        }
        let Ok(bounce) = DramBounce::new(WRITE_BOUNCE_BYTES.min(buf.len().max(1))) else {
            return self.read_direct(offset, buf);
        };
        let mut at = offset as usize;
        for chunk in buf.chunks_mut(bounce.len) {
            // SAFETY: a DRAM destination valid for `chunk.len() <= bounce.len` bytes; the layout bounds-checks the source.
            let err = unsafe {
                esp_partition_read(self.partition, at, bounce.ptr.cast(), chunk.len())
            };
            check(err, "read")?;
            // SAFETY: `chunk.len()` bytes were just read into the bounce, and the two buffers cannot overlap.
            unsafe { core::ptr::copy_nonoverlapping(bounce.ptr, chunk.as_mut_ptr(), chunk.len()) };
            at += chunk.len();
        }
        Ok(())
    }

    fn read_direct(&self, offset: u32, buf: &mut [u8]) -> Result<(), StoreError> {
        // SAFETY: the layout bounds-checks offset and length; `buf` is valid for `buf.len()` writes.
        let err = unsafe {
            esp_partition_read(self.partition, offset as usize, buf.as_mut_ptr().cast(), buf.len())
        };
        check(err, "read")
    }

    fn read_mapped(&self, offset: u32, buf: &mut [u8]) -> Option<()> {
        let start = offset as usize;
        let end = start.checked_add(buf.len())?;
        if self.mapped.is_null() || end > self.mapped_len {
            return None;
        }
        // SAFETY: `[start, end)` lies inside the read-only mapping; `buf` holds `buf.len()` bytes and cannot overlap it.
        unsafe { core::ptr::copy_nonoverlapping(self.mapped.add(start), buf.as_mut_ptr(), buf.len()) };
        Some(())
    }
}

impl RawFlash for EspPartitionFlash {
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<(), StoreError> {
        if self.read_mapped(offset, buf).is_some() {
            return Ok(());
        }
        self.read_through_driver(offset, buf)
    }

    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), StoreError> {
        let _gate = dali2rust_platform::flash_gate::hold();
        hold_for_wire_gap();
        let mut bounce = DramBounce::new(WRITE_BOUNCE_BYTES.min(bytes.len().max(1)))?;
        let mut at = offset as usize;
        for chunk in bytes.chunks(bounce.len) {
            let src = bounce.fill(chunk);
            // SAFETY: a DRAM source the driver programs directly; the layout bounds-checks the target range.
            let err = unsafe { esp_partition_write(self.partition, at, src.cast(), chunk.len()) };
            check(err, "write")?;
            at += chunk.len();
        }
        Ok(())
    }

    fn erase(&self, offset: u32, len: u32) -> Result<(), StoreError> {
        let _gate = dali2rust_platform::flash_gate::hold();
        hold_for_wire_gap();
        // SAFETY: bank offsets and sizes are sector-aligned by construction (asserted in `slice_layout` tests).
        let err =
            unsafe { esp_partition_erase_range(self.partition, offset as usize, len as usize) };
        check(err, "erase")
    }

    fn crc32(&self, seed: u32, bytes: &[u8]) -> u32 {
        // SAFETY: read-only ROM routine over a valid slice.
        unsafe { esp_rom_crc32_le(seed, bytes.as_ptr(), bytes.len() as u32) }
    }

    fn note_commit(&self) {
        dali2rust_platform::dali::note_persist_commit();
    }
}

pub struct RawPartitionSliceStore;

impl RawPartitionSliceStore {
    pub fn mount() -> Result<SliceStoreCore<impl RawFlash>, StoreError> {
        // SAFETY: a NUL-terminated literal label; the call only looks a descriptor up in the partition table.
        let partition = unsafe {
            esp_partition_find_first(
                esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
                esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_ANY,
                PARTITION_LABEL.as_ptr().cast(),
            )
        };
        if partition.is_null() {
            return Err(StoreError::Backend("no `storage` partition".to_string()));
        }
        // SAFETY: non-null descriptor returned by the lookup above.
        let size = unsafe { (*partition).size };
        if size < LAYOUT_BYTES {
            return Err(StoreError::Backend(format!(
                "`storage` is {size} B, layout needs {LAYOUT_BYTES} B"
            )));
        }
        let mapped = map_layout(partition);
        let mapped_len = if mapped.is_null() { 0 } else { LAYOUT_BYTES as usize };
        Ok(SliceStoreCore::new(EspPartitionFlash {
            partition,
            mapped,
            mapped_len,
        }))
    }
}

pub const fn layout_sectors() -> u32 {
    LAYOUT_BYTES / SECTOR_BYTES
}
