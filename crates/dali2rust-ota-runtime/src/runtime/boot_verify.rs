use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use dali2rust_platform::firmware::FirmwareUpdatePort;

const PROBE_INTERVAL: Duration = Duration::from_secs(5);

const REQUIRED_HEALTHY_PROBES: u32 = 6;

const VERIFY_DEADLINE: Duration = Duration::from_secs(180);

pub fn spawn_boot_verifier(
    port: Arc<dyn FirmwareUpdatePort>,
    healthy: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Option<JoinHandle<()>> {
    if !port.slot().pending_verify {
        return None;
    }
    log::warn!("firmware image is pending verification — proving it before the next reset");
    Some(dali2rust_bsp::esp_thread::spawn_named_stack(
        c"ota-verify",
        dali2rust_bsp::std_thread_stack::EVENT_WORKER_STACK,
        move || verify_loop(port.as_ref(), healthy.as_ref()),
    ))
}

fn verify_loop(port: &dyn FirmwareUpdatePort, healthy: &(dyn Fn() -> bool + Send + Sync)) {
    let deadline = std::time::Instant::now() + VERIFY_DEADLINE;
    let mut streak = 0;
    while std::time::Instant::now() < deadline {
        // sleep-ok: documented product timing, not a poll for a condition.
        std::thread::sleep(PROBE_INTERVAL);
        streak = if healthy() { streak + 1 } else { 0 };
        if streak < REQUIRED_HEALTHY_PROBES {
            continue;
        }
        match port.mark_valid() {
            Ok(()) => log::info!("firmware image verified and kept"),
            Err(error) => log::error!("could not mark the image valid: {}", error.as_str()),
        }
        return;
    }
    log::error!(
        "firmware image did not prove itself within {} s — the next reset rolls back",
        VERIFY_DEADLINE.as_secs()
    );
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use dali2rust_platform::firmware::{FirmwareError, FirmwareSlot};

    use super::*;

    struct FakePort {
        pending: bool,
        marked: AtomicU32,
    }

    impl FirmwareUpdatePort for FakePort {
        fn slot(&self) -> FirmwareSlot {
            FirmwareSlot::new("ota_1", self.pending, true)
        }
        fn begin(&self, _total: Option<u32>) -> Result<(), FirmwareError> {
            Ok(())
        }
        fn write(&self, _chunk: &[u8]) -> Result<(), FirmwareError> {
            Ok(())
        }
        fn finish(&self) -> Result<(), FirmwareError> {
            Ok(())
        }
        fn abort(&self) {}
        fn mark_valid(&self) -> Result<(), FirmwareError> {
            self.marked.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn reboot(&self) {}
    }

    #[test]
    fn an_image_with_nothing_to_prove_starts_no_verifier() {
        let port = Arc::new(FakePort {
            pending: false,
            marked: AtomicU32::new(0),
        });
        let handle = spawn_boot_verifier(port.clone(), Arc::new(|| true));
        assert!(handle.is_none());
        assert_eq!(port.marked.load(Ordering::Relaxed), 0);
    }
}
