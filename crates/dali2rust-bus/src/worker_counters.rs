use core::sync::atomic::{AtomicU32, Ordering};

pub fn bump(counter: &AtomicU32) {
    bump_by(counter, 1);
}

pub fn bump_by(counter: &AtomicU32, by: u32) {
    counter.fetch_add(by, Ordering::Relaxed);
}

pub fn load(counter: &AtomicU32) -> u32 {
    counter.load(Ordering::Relaxed)
}

pub fn set_flag(counter: &AtomicU32, up: bool) {
    counter.store(u32::from(up), Ordering::Relaxed);
}

pub fn mirror(counter: &AtomicU32, total: u32) {
    counter.store(total, Ordering::Relaxed);
}
