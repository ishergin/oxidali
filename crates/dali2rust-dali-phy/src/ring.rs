use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering};

pub struct SpscRing<T, const CAP: usize> {
    slots: UnsafeCell<[MaybeUninit<T>; CAP]>,
    head: AtomicU32,
    tail: AtomicU32,
}

// SAFETY: single producer, single consumer; slot writes are synchronised through the head/tail atomics.
unsafe impl<T: Send, const CAP: usize> Sync for SpscRing<T, CAP> {}

impl<T, const CAP: usize> SpscRing<T, CAP> {
    pub const fn new() -> Self {
        Self {
            slots: UnsafeCell::new([const { MaybeUninit::uninit() }; CAP]),
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    const fn next(i: usize) -> usize {
        let n = i.wrapping_add(1);
        if n >= CAP {
            0
        } else {
            n
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn try_push(&self, item: T) -> Result<(), T> {
        let head = self.head.load(Ordering::Relaxed) as usize;
        let tail = self.tail.load(Ordering::Acquire) as usize;

        let next_head = Self::next(head);
        if next_head == tail {
            return Err(item);
        }

        // SAFETY: only the producer writes `head`; `next_head != tail` proves the slot free; no `&mut` to the array.
        unsafe {
            let slot = self.slots.get().cast::<MaybeUninit<T>>().wrapping_add(head);
            (*slot).write(item);
        }

        self.head.store(next_head as u32, Ordering::Release);
        Ok(())
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn try_pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed) as usize;
        let head = self.head.load(Ordering::Acquire) as usize;

        if tail == head {
            return None;
        }

        // SAFETY: the Acquire load of `head` pairs with the producer's Release, so the slot at `tail` is initialised.
        let item = unsafe {
            let slot = self.slots.get().cast::<MaybeUninit<T>>().wrapping_add(tail);
            (*slot).assume_init_read()
        };

        self.tail.store(Self::next(tail) as u32, Ordering::Release);
        Some(item)
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) == self.tail.load(Ordering::Relaxed)
    }
}

impl<T, const CAP: usize> Default for SpscRing<T, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestEvent {
        A,
        B(u8),
    }

    #[test]
    fn push_pop_roundtrip() {
        let ring: SpscRing<TestEvent, 16> = SpscRing::new();
        assert!(ring.try_pop().is_none());

        ring.try_push(TestEvent::A).unwrap();
        ring.try_push(TestEvent::B(42)).unwrap();

        assert_eq!(ring.try_pop(), Some(TestEvent::A));
        assert_eq!(ring.try_pop(), Some(TestEvent::B(42)));
        assert!(ring.try_pop().is_none());
    }

    #[test]
    fn overflow_drops() {
        let ring: SpscRing<TestEvent, 4> = SpscRing::new();
        for _ in 0..3 {
            ring.try_push(TestEvent::A).unwrap();
        }
        assert!(ring.try_push(TestEvent::B(1)).is_err());
    }

    #[test]
    fn is_empty() {
        let ring: SpscRing<TestEvent, 4> = SpscRing::new();
        assert!(ring.is_empty());
        ring.try_push(TestEvent::A).unwrap();
        assert!(!ring.is_empty());
        ring.try_pop().unwrap();
        assert!(ring.is_empty());
    }

    #[test]
    fn wrap_around() {
        let ring: SpscRing<TestEvent, 4> = SpscRing::new();
        ring.try_push(TestEvent::B(1)).unwrap();
        ring.try_push(TestEvent::B(2)).unwrap();
        ring.try_push(TestEvent::B(3)).unwrap();

        assert_eq!(ring.try_pop(), Some(TestEvent::B(1)));
        assert_eq!(ring.try_pop(), Some(TestEvent::B(2)));

        ring.try_push(TestEvent::B(4)).unwrap();
        ring.try_push(TestEvent::B(5)).unwrap();

        assert_eq!(ring.try_pop(), Some(TestEvent::B(3)));
        assert_eq!(ring.try_pop(), Some(TestEvent::B(4)));
        assert_eq!(ring.try_pop(), Some(TestEvent::B(5)));
        assert!(ring.is_empty());
    }

    #[test]
    fn next_index_agrees_with_modular_increment() {
        for i in 0..8usize {
            assert_eq!(SpscRing::<TestEvent, 8>::next(i), (i + 1) % 8);
        }
        assert_eq!(SpscRing::<TestEvent, 4>::next(3), 0);
    }
}
