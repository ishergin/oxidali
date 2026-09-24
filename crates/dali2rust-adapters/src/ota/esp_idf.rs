use std::sync::Mutex;

use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;
use esp_idf_svc::sys::{
    esp, esp_crt_bundle_attach, esp_ota_abort, esp_ota_begin, esp_ota_end, esp_ota_get_boot_partition,
    esp_ota_get_next_update_partition, esp_ota_get_running_partition, esp_ota_get_state_partition,
    esp_ota_handle_t, esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY,
    esp_ota_mark_app_valid_cancel_rollback, esp_ota_set_boot_partition, esp_ota_write,
    esp_partition_t,
    esp_restart, OTA_WITH_SEQUENTIAL_WRITES,
};

use dali2rust_platform::firmware::{
    FirmwareError, FirmwareImageSource, FirmwareSink, FirmwareSlot, FirmwareUpdatePort,
};
use dali2rust_platform::flash_gate;

static LAST_SLOT: Mutex<Option<FirmwareSlot>> = Mutex::new(None);

const READ_CHUNK_BYTES: usize = 8 * 1024;

const HTTP_BUFFER_BYTES: usize = 4096;

const HTTP_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(60);

struct OpenSlot {
    handle: esp_ota_handle_t,
    partition: *const esp_partition_t,
}

// SAFETY: `partition` points into the partition table, mapped for life; mutation only through the mutex.
unsafe impl Send for OpenSlot {}

#[derive(Default)]
pub struct EspFirmwarePort {
    open: Mutex<Option<OpenSlot>>,
}

impl EspFirmwarePort {
    fn take_open(&self) -> Option<OpenSlot> {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

fn label_of(partition: *const esp_partition_t) -> &'static str {
    if partition.is_null() {
        return "";
    }
    // SAFETY: non-null; the label is a NUL-terminated array in the partition table, mapped for life.
    unsafe { core::ffi::CStr::from_ptr((*partition).label.as_ptr()) }
        .to_str()
        .unwrap_or("")
}

fn running_slot() -> FirmwareSlot {
    if let Some(slot) = *LAST_SLOT.lock().unwrap_or_else(|e| e.into_inner()) {
        return slot;
    }
    if flash_mapping_refused("the running-slot read").is_err() {
        return FirmwareSlot::new("", false, false);
    }
    let _gate = flash_gate::hold();
    let slot = read_running_slot();
    *LAST_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(slot);
    slot
}

fn flash_mapping_refused(call: &str) -> Result<(), FirmwareError> {
    if dali2rust_bsp::esp_thread::current_stack_is_external() {
        log::error!("ota: {call} maps flash and stops the cache; refused on a task whose stack is in PSRAM");
        return Err(FirmwareError::Write);
    }
    Ok(())
}

fn read_running_slot() -> FirmwareSlot {
    // SAFETY: both calls read the partition table and return a valid pointer or null; nothing is written.
    let (running, update) = unsafe {
        (
            esp_ota_get_running_partition(),
            esp_ota_get_next_update_partition(core::ptr::null()),
        )
    };
    if running.is_null() {
        return FirmwareSlot::new("", false, false);
    }
    FirmwareSlot::new(
        label_of(running),
        pending_verify(running),
        !update.is_null(),
    )
}

fn pending_verify(partition: *const esp_partition_t) -> bool {
    let mut state = 0;
    // SAFETY: `partition` is non-null (checked by the caller) and `state` is a live out-parameter.
    let ok = unsafe { esp_ota_get_state_partition(partition, &mut state) } == 0;
    ok && state == esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY
}

pub fn log_slots() {
    let _gate = flash_gate::hold();
    *LAST_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(read_running_slot());
    // SAFETY: all three read the partition table and return a pointer or null.
    let (running, boot, next) = unsafe {
        (
            esp_ota_get_running_partition(),
            esp_ota_get_boot_partition(),
            esp_ota_get_next_update_partition(core::ptr::null()),
        )
    };
    log::warn!(
        "ota slots: running={} boot={} next={}",
        label_of(running),
        label_of(boot),
        label_of(next)
    );
}

impl FirmwareUpdatePort for EspFirmwarePort {
    fn slot(&self) -> FirmwareSlot {
        running_slot()
    }

    fn begin(&self, total_bytes: Option<u32>) -> Result<(), FirmwareError> {
        flash_mapping_refused("esp_ota_begin")?;
        // SAFETY: both read the partition table and return a pointer or null.
        let (partition, running) = unsafe {
            (
                esp_ota_get_next_update_partition(core::ptr::null()),
                esp_ota_get_running_partition(),
            )
        };
        if partition.is_null() {
            return Err(FirmwareError::NoSlot);
        }
        if !running.is_null() && core::ptr::eq(partition, running) {
            log::error!(
                "ota: refusing to write the running slot {} — no inactive slot to use",
                label_of(running)
            );
            return Err(FirmwareError::NoSlot);
        }
        let handle = open_slot(partition)?;
        log::warn!(
            "ota: writing {} bytes into slot {}",
            total_bytes.map_or(-1i64, i64::from),
            label_of(partition)
        );
        *self.open.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(OpenSlot { handle, partition });
        Ok(())
    }

    fn write(&self, chunk: &[u8]) -> Result<(), FirmwareError> {
        let guard = self.open.lock().unwrap_or_else(|e| e.into_inner());
        let Some(open) = guard.as_ref() else {
            return Err(FirmwareError::Write);
        };
        let _gate = flash_gate::hold();
        // SAFETY: the handle came from a successful `esp_ota_begin` and the slice is live for the call.
        esp!(unsafe { esp_ota_write(open.handle, chunk.as_ptr().cast(), chunk.len()) })
            .map_err(|_| FirmwareError::Write)
    }

    fn finish(&self) -> Result<(), FirmwareError> {
        flash_mapping_refused("esp_ota_end")?;
        let Some(open) = self.take_open() else {
            return Err(FirmwareError::Write);
        };
        let _gate = flash_gate::hold();
        let selected = end_and_select(&open);
        flash_gate::set_firmware_write_open(false);
        selected
    }

    fn abort(&self) {
        if let Some(open) = self.take_open() {
            let _gate = flash_gate::hold();
            // SAFETY: the handle is live and consumed exactly once; a failure here has nowhere left to go.
            let _ = unsafe { esp_ota_abort(open.handle) };
            flash_gate::set_firmware_write_open(false);
        }
    }

    fn mark_valid(&self) -> Result<(), FirmwareError> {
        flash_mapping_refused("esp_ota_mark_app_valid_cancel_rollback")?;
        let _gate = flash_gate::hold();
        // SAFETY: no arguments; cancels the rollback for the running image.
        esp!(unsafe { esp_ota_mark_app_valid_cancel_rollback() })
            .map_err(|_| FirmwareError::Write)?;
        *LAST_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(read_running_slot());
        Ok(())
    }

    fn reboot(&self) {
        log::warn!("firmware update complete — rebooting into the new image");
        // SAFETY: no arguments; does not return.
        unsafe { esp_restart() }
    }
}

fn end_and_select(open: &OpenSlot) -> Result<(), FirmwareError> {
    // SAFETY: the handle is live and consumed exactly once.
    esp!(unsafe { esp_ota_end(open.handle) }).map_err(|_| FirmwareError::InvalidImage)?;
    // SAFETY: the partition pointer is the one the handle was opened on.
    esp!(unsafe { esp_ota_set_boot_partition(open.partition) }).map_err(|error| {
        log::error!("ota: could not select {}: {error:?}", label_of(open.partition));
        FirmwareError::Write
    })?;
    log::warn!("ota: {} selected for the next boot", label_of(open.partition));
    Ok(())
}

fn open_slot(partition: *const esp_partition_t) -> Result<esp_ota_handle_t, FirmwareError> {
    flash_gate::set_firmware_write_open(true);
    let _gate = flash_gate::hold();
    let mut handle: esp_ota_handle_t = 0;
    // SAFETY: `partition` is non-null and `handle` is a live out-parameter.
    let size = OTA_WITH_SEQUENTIAL_WRITES as usize;
    if let Err(e) = esp!(unsafe { esp_ota_begin(partition, size, &mut handle) }) {
        flash_gate::set_firmware_write_open(false);
        log::error!("ota: esp_ota_begin failed: {e:?}");
        return Err(FirmwareError::Write);
    }
    Ok(handle)
}

#[derive(Debug, Default)]
pub struct EspImageSource;

impl EspImageSource {
    fn connect() -> Result<EspHttpConnection, FirmwareError> {
        EspHttpConnection::new(&Configuration {
            buffer_size: Some(HTTP_BUFFER_BYTES),
            timeout: Some(HTTP_TIMEOUT),
            crt_bundle_attach: Some(esp_crt_bundle_attach),
            ..Default::default()
        })
        .map_err(|_| FirmwareError::Fetch)
    }

    fn open(connection: &mut EspHttpConnection, url: &str) -> Result<Option<u32>, FirmwareError> {
        connection
            .initiate_request(Method::Get, url, &[])
            .map_err(|_| FirmwareError::Fetch)?;
        connection
            .initiate_response()
            .map_err(|_| FirmwareError::Fetch)?;
        if !(200..300).contains(&connection.status()) {
            return Err(FirmwareError::Fetch);
        }
        Ok(connection
            .header("Content-Length")
            .and_then(|value| value.parse().ok()))
    }
}

impl FirmwareImageSource for EspImageSource {
    fn fetch(&self, url: &str, sink: &mut dyn FirmwareSink) -> Result<u32, FirmwareError> {
        let mut connection = Self::connect()?;
        let total = Self::open(&mut connection, url)?;
        sink.total(total)?;
        let mut buffer = [0u8; READ_CHUNK_BYTES];
        let mut written = 0u32;
        loop {
            let read = connection
                .read(&mut buffer)
                .map_err(|_| FirmwareError::Fetch)?;
            if read == 0 {
                break;
            }
            sink.chunk(&buffer[..read])?;
            written = written.saturating_add(read as u32);
        }
        if total.is_some_and(|announced| announced != written) {
            return Err(FirmwareError::Fetch);
        }
        Ok(written)
    }
}
