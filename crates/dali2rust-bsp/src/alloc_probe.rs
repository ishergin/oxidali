#[cfg(target_os = "espidf")]
mod imp {
    use core::ffi::c_char;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNT: AtomicU32 = AtomicU32::new(0);
    static LAST_SIZE: AtomicU32 = AtomicU32::new(0);
    static LAST_CAPS: AtomicU32 = AtomicU32::new(0);
    static MAX_SIZE: AtomicU32 = AtomicU32::new(0);

    unsafe extern "C" fn on_failed_alloc(size: usize, caps: u32, _function_name: *const c_char) {
        let size = size as u32;
        COUNT.fetch_add(1, Ordering::Relaxed);
        LAST_SIZE.store(size, Ordering::Relaxed);
        LAST_CAPS.store(caps, Ordering::Relaxed);
        if size > MAX_SIZE.load(Ordering::Relaxed) {
            MAX_SIZE.store(size, Ordering::Relaxed);
        }
    }

    static ARM_RC: AtomicU32 = AtomicU32::new(u32::MAX);

    pub fn arm() {
        // SAFETY: registers a `'static` extern "C" hook; ESP-IDF stores the pointer and calls it on allocator failure.
        let rc = unsafe {
            esp_idf_svc::sys::heap_caps_register_failed_alloc_callback(Some(on_failed_alloc))
        };
        ARM_RC.store(rc as u32, Ordering::Relaxed);
    }

    pub fn log_armed() {
        let rc = ARM_RC.load(Ordering::Relaxed);
        if rc == esp_idf_svc::sys::ESP_OK as u32 {
            log::warn!("alloc-failure probe armed (ISSUE-11)");
        } else {
            log::error!("alloc-failure probe NOT armed: rc={rc}");
        }
    }

    pub fn snapshot() -> (u32, u32, u32, u32) {
        (
            COUNT.load(Ordering::Relaxed),
            LAST_SIZE.load(Ordering::Relaxed),
            LAST_CAPS.load(Ordering::Relaxed),
            MAX_SIZE.load(Ordering::Relaxed),
        )
    }
}

#[cfg(not(target_os = "espidf"))]
mod imp {
    pub fn arm() {}

    pub fn log_armed() {}

    pub fn snapshot() -> (u32, u32, u32, u32) {
        (0, 0, 0, 0)
    }
}

pub use imp::{arm, log_armed};

use std::sync::atomic::{AtomicU32, Ordering};

static REPORTED: AtomicU32 = AtomicU32::new(0);

pub fn poll(force: bool) {
    let (count, last_size, last_caps, max_size) = imp::snapshot();
    if count == 0 || (!force && count == REPORTED.load(Ordering::Relaxed)) {
        return;
    }
    REPORTED.store(count, Ordering::Relaxed);
    log::warn!(
        "alloc failures: count={count} last_size={last_size} last_caps=0x{last_caps:08x} \
         max_size={max_size}"
    );
}
