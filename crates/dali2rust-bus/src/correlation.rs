use std::sync::atomic::{AtomicU32, Ordering};

pub struct CorrelationIdAllocator {
    next: AtomicU32,
}

impl CorrelationIdAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicU32::new(1),
        }
    }

    pub fn next_id(&self) -> u64 {
        loop {
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return u64::from(id);
            }
        }
    }
}

impl Default for CorrelationIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}
