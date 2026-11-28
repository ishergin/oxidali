use super::announce::{reached, PAGE_GAP_MS};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PageSlot {
    next_ms: Option<u32>,
}

impl PageSlot {
    pub(super) fn held(self, now_ms: u32) -> bool {
        self.next_ms.is_some_and(|next_ms| {
            let ahead_ms = next_ms.wrapping_sub(now_ms);
            ahead_ms != 0 && ahead_ms <= PAGE_GAP_MS
        })
    }

    pub(super) fn take(&mut self, now_ms: u32) {
        self.next_ms = Some(now_ms.wrapping_add(PAGE_GAP_MS));
    }

    pub(super) fn clamp(self, now_ms: u32, at_ms: u32) -> u32 {
        match self.next_ms {
            Some(next_ms) if self.held(now_ms) && reached(next_ms, at_ms) => next_ms,
            _ => at_ms,
        }
    }
}
