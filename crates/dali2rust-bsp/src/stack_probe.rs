pub struct StackLowWater {
    #[cfg(target_os = "espidf")]
    min_free: core::sync::atomic::AtomicU32,
    #[cfg(target_os = "espidf")]
    task: &'static str,
    #[cfg(target_os = "espidf")]
    stack_bytes: usize,
}

impl StackLowWater {
    pub const fn new(task: &'static str, stack_bytes: usize) -> Self {
        #[cfg(target_os = "espidf")]
        {
            Self {
                min_free: core::sync::atomic::AtomicU32::new(u32::MAX),
                task,
                stack_bytes,
            }
        }
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = (task, stack_bytes);
            Self {}
        }
    }

    #[cfg(target_os = "espidf")]
    pub fn note(&self, tag: &str) {
        use core::sync::atomic::Ordering;

        // SAFETY: pure FFI query about the current task; no aliasing, no state.
        let free = unsafe { esp_idf_svc::sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) };
        if free < self.min_free.fetch_min(free, Ordering::Relaxed) {
            log::warn!(
                "{} stack low-water: {free} B free of {} after {tag}",
                self.task,
                self.stack_bytes
            );
        }
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn note(&self, tag: &str) {
        let _ = tag;
    }
}
