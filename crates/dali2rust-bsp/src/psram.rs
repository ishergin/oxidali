use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};

pub struct PsramBox<T>(Repr<T>);

impl<T> PsramBox<T> {
    pub fn new(value: T) -> Self {
        Self(Repr::new(value))
    }

    /// # Safety
    /// `init` must write every field of the place exactly once.
    pub unsafe fn new_with(init: impl FnOnce(&mut MaybeUninit<T>)) -> Self {
        // SAFETY: forwarded verbatim to this function's own contract.
        Self(unsafe { place_with(init) })
    }

    /// # Safety
    /// `init` must write every field of the place exactly once; it does not run when PSRAM refuses.
    pub unsafe fn try_new_with(init: impl FnOnce(&mut MaybeUninit<T>)) -> Option<Self> {
        // SAFETY: forwarded verbatim to this function's own contract.
        unsafe { try_place_with(init).map(Self) }
    }
}

impl<T> Deref for PsramBox<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for PsramBox<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> AsRef<T> for PsramBox<T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> AsMut<T> for PsramBox<T> {
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for PsramBox<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: Clone> Clone for PsramBox<T> {
    fn clone(&self) -> Self {
        Self::new((**self).clone())
    }
}

impl<T: Default> Default for PsramBox<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

#[cfg(not(target_os = "espidf"))]
type Repr<T> = Box<T>;

#[cfg(not(target_os = "espidf"))]
unsafe fn place_with<T>(init: impl FnOnce(&mut MaybeUninit<T>)) -> Repr<T> {
    let mut place = Box::<T>::new_uninit();
    init(&mut place);
    // SAFETY: the caller's contract says `init` initialised every field.
    unsafe { place.assume_init() }
}

#[cfg(not(target_os = "espidf"))]
unsafe fn try_place_with<T>(init: impl FnOnce(&mut MaybeUninit<T>)) -> Option<Repr<T>> {
    // SAFETY: the caller's `init` fills the whole place, as `PsramBox::new_with` requires.
    Some(unsafe { place_with(init) })
}

#[cfg(target_os = "espidf")]
type Repr<T> = esp::PsramPtr<T>;

#[cfg(target_os = "espidf")]
unsafe fn place_with<T>(init: impl FnOnce(&mut MaybeUninit<T>)) -> Repr<T> {
    // SAFETY: the caller's `init` fills the whole place, as `PsramBox::new_with` requires.
    unsafe { esp::PsramPtr::new_with(init) }
}

#[cfg(target_os = "espidf")]
unsafe fn try_place_with<T>(init: impl FnOnce(&mut MaybeUninit<T>)) -> Option<Repr<T>> {
    // SAFETY: the caller's `init` fills the whole place, as `PsramBox::new_with` requires.
    unsafe { esp::PsramPtr::try_new_with(init) }
}

#[cfg(target_os = "espidf")]
mod esp {
    use core::mem::MaybeUninit;
    use core::ptr::NonNull;
    use esp_idf_svc::sys::{
        heap_caps_aligned_alloc, heap_caps_free, MALLOC_CAP_8BIT, MALLOC_CAP_SPIRAM,
    };

    const MIN_ALIGN: usize = 4;

    const PSRAM_CAPS: u32 = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT;

    pub struct PsramPtr<T>(NonNull<T>);

    // SAFETY: an owned, never-aliased heap allocation, as Send and Sync as the `T` it owns, exactly like `Box`.
    unsafe impl<T: Send> Send for PsramPtr<T> {}
    unsafe impl<T: Sync> Sync for PsramPtr<T> {}

    impl<T> PsramPtr<T> {
        pub fn new(value: T) -> Self {
            let ptr = Self::alloc();
            // SAFETY: `alloc` returns a live, uninitialised allocation sized and aligned for `T` that nothing references.
            unsafe { ptr.as_ptr().write(value) };
            Self(ptr)
        }

        /// # Safety
        /// `init` must write every field of the place exactly once.
        pub unsafe fn new_with(init: impl FnOnce(&mut MaybeUninit<T>)) -> Self {
            struct FreeOnUnwind<T>(NonNull<T>);
            impl<T> Drop for FreeOnUnwind<T> {
                fn drop(&mut self) {
                    // SAFETY: the pointer came from `alloc` and is still uninitialised, so there is nothing to drop in place.
                    unsafe { heap_caps_free(self.0.as_ptr().cast()) };
                }
            }

            let guard = FreeOnUnwind(Self::alloc());
            // SAFETY: a fresh, unaliased allocation sized and aligned for `T`; `MaybeUninit<T>` permits it uninitialised.
            let place = unsafe { &mut *guard.0.as_ptr().cast::<MaybeUninit<T>>() };
            init(place);
            let ptr = guard.0;
            core::mem::forget(guard);
            Self(ptr)
        }

        /// # Safety
        /// `init` must write every field of the place exactly once and must not panic, or the block leaks.
        pub unsafe fn try_new_with(init: impl FnOnce(&mut MaybeUninit<T>)) -> Option<Self> {
            let ptr = Self::try_alloc_psram()?;
            // SAFETY: this allocation is uniquely owned and correctly laid out.
            let place = unsafe { &mut *ptr.as_ptr().cast::<MaybeUninit<T>>() };
            init(place);
            Some(Self(ptr))
        }

        fn alloc() -> NonNull<T> {
            let (size, align) = Self::layout();
            // SAFETY: plain capability-tagged allocation, released in `Drop`.
            let psram = unsafe { heap_caps_aligned_alloc(align, size, PSRAM_CAPS) };
            if let Some(ptr) = NonNull::new(psram.cast::<T>()) {
                return ptr;
            }
            log::warn!("PSRAM allocation of {size} B failed; falling back to internal heap");
            // SAFETY: same contract, internal heap.
            let internal = unsafe { heap_caps_aligned_alloc(align, size, MALLOC_CAP_8BIT) };
            NonNull::new(internal.cast::<T>()).expect("out of memory placing a record")
        }

        fn try_alloc_psram() -> Option<NonNull<T>> {
            let (size, align) = Self::layout();
            // SAFETY: plain capability-tagged allocation, released in `Drop`.
            let ptr = unsafe { heap_caps_aligned_alloc(align, size, PSRAM_CAPS) };
            NonNull::new(ptr.cast::<T>())
        }

        fn layout() -> (usize, usize) {
            let align = core::mem::align_of::<T>().max(MIN_ALIGN);
            (core::mem::size_of::<T>().next_multiple_of(align), align)
        }
    }

    impl<T> core::ops::Deref for PsramPtr<T> {
        type Target = T;

        fn deref(&self) -> &T {
            // SAFETY: initialised in `new` and owned until `Drop`.
            unsafe { self.0.as_ref() }
        }
    }

    impl<T> core::ops::DerefMut for PsramPtr<T> {
        fn deref_mut(&mut self) -> &mut T {
            // SAFETY: initialised in `new`, owned until `Drop`, and `&mut self` proves this is the only live reference.
            unsafe { self.0.as_mut() }
        }
    }

    impl<T> Drop for PsramPtr<T> {
        fn drop(&mut self) {
            // SAFETY: the pointer came from `alloc`, was initialised in `new`, and this is its only owner.
            unsafe {
                core::ptr::drop_in_place(self.0.as_ptr());
                heap_caps_free(self.0.as_ptr().cast());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MaybeUninit, PsramBox};

    #[test]
    fn stores_the_value_behind_a_pointer_sized_handle() {
        let boxed = PsramBox::new([7u8; 3032]);
        assert_eq!(boxed[0], 7);
        assert_eq!(
            core::mem::size_of::<PsramBox<[u8; 3032]>>(),
            core::mem::size_of::<usize>()
        );
    }

    #[test]
    fn drops_the_placed_value() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static DROPS: AtomicU32 = AtomicU32::new(0);

        struct Noisy;
        impl Drop for Noisy {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }

        drop(PsramBox::new(Noisy));
        assert_eq!(DROPS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn new_with_initialises_the_place_it_is_handed() {
        const N: usize = 3032;
        let fill = |place: &mut MaybeUninit<[u8; N]>| {
            let first = place.as_mut_ptr().cast::<u8>();
            for i in 0..N {
                // SAFETY: `i < N`, so this stays inside the place.
                unsafe { first.add(i).write((i % 251) as u8) };
            }
        };
        // SAFETY: `fill` writes every one of the N bytes exactly once.
        let boxed = unsafe { PsramBox::<[u8; N]>::new_with(fill) };
        assert_eq!(boxed[0], 0);
        assert_eq!(boxed[N - 1], ((N - 1) % 251) as u8);
    }

    #[test]
    fn a_panicking_init_drops_nothing_it_did_not_build() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static DROPS: AtomicU32 = AtomicU32::new(0);

        struct NeverBuilt;
        impl Drop for NeverBuilt {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }

        let unwound = std::panic::catch_unwind(|| {
            // SAFETY: the closure never returns, so the contract is vacuous, which is the case under test.
            let _ = unsafe {
                PsramBox::<NeverBuilt>::new_with(|_place| panic!("init failed"))
            };
        });

        assert!(
            unwound.is_err(),
            "the panic must propagate, not be swallowed"
        );
        assert_eq!(
            DROPS.load(Ordering::Relaxed),
            0,
            "an uninitialised place must never be dropped as a T"
        );
    }

    #[test]
    fn new_with_drops_the_placed_value() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static DROPS: AtomicU32 = AtomicU32::new(0);

        struct Placed;
        impl Drop for Placed {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }

        // SAFETY: the closure writes the whole value exactly once.
        drop(unsafe {
            PsramBox::new_with(|place| {
                place.write(Placed);
            })
        });
        assert_eq!(DROPS.load(Ordering::Relaxed), 1);
    }
}
