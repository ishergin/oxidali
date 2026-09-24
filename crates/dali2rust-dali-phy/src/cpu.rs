use core::sync::atomic::{AtomicU32, Ordering};

pub static ISR_CORE_ID: AtomicU32 = AtomicU32::new(u32::MAX);

pub fn isr_core_id() -> Option<u32> {
    match ISR_CORE_ID.load(Ordering::Relaxed) {
        u32::MAX => None,
        id => Some(id),
    }
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
pub(crate) fn raw_core_id() -> u32 {
    #[cfg(target_arch = "riscv32")]
    {
        let id: u32;
        // SAFETY: `mhartid` is a read-only machine CSR with no side effects, readable at the ISR's privilege level.
        unsafe {
            core::arch::asm!("csrr {0}, mhartid", out(reg) id, options(nomem, nostack, preserves_flags));
        }
        id
    }
    #[cfg(not(target_arch = "riscv32"))]
    {
        u32::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_reads_as_unknown_and_a_stored_id_reads_back() {
        assert_eq!(ISR_CORE_ID.load(Ordering::Relaxed), u32::MAX);
        assert_eq!(isr_core_id(), None);

        ISR_CORE_ID.store(1, Ordering::Relaxed);
        assert_eq!(isr_core_id(), Some(1));
        ISR_CORE_ID.store(u32::MAX, Ordering::Relaxed);
    }
}
