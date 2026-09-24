use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

static GATE: Mutex<()> = Mutex::new(());

static FIRMWARE_WRITE_OPEN: AtomicBool = AtomicBool::new(false);

pub fn hold() -> MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn try_hold() -> Option<MutexGuard<'static, ()>> {
    match GATE.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

pub fn set_firmware_write_open(open: bool) {
    FIRMWARE_WRITE_OPEN.store(open, Ordering::Release);
}

pub fn firmware_write_open() -> bool {
    FIRMWARE_WRITE_OPEN.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_hold_refuses_while_held_and_admits_after() {
        let guard = hold();
        let refused = std::thread::spawn(|| try_hold().is_none()).join().unwrap();
        assert!(refused, "a held gate must refuse try_hold from another thread");
        drop(guard);
        let admitted = std::thread::spawn(|| try_hold().is_some()).join().unwrap();
        assert!(admitted);
    }

    #[test]
    fn the_firmware_write_flag_round_trips() {
        set_firmware_write_open(true);
        assert!(firmware_write_open());
        set_firmware_write_open(false);
        assert!(!firmware_write_open());
    }
}
