use std::sync::{Mutex, PoisonError, TryLockError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusUnitIdentity {
    pub firmware_version: [u8; 2],
    pub hardware_version: [u8; 2],
    pub identification_number: [u8; 8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusUnitNvm {
    pub short_address: Option<u8>,
    pub random_address: u32,
    pub device_groups: u32,
    pub operating_mode: u8,
    pub power_cycle_notification: bool,
    pub application_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusUnitSnapshot {
    pub nvm: BusUnitNvm,
    pub revision: u32,
    pub acknowledged_wire_sequence: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusUnitEvidence {
    pub adopted: BusUnitNvm,
    pub adopted_revision: u32,
    pub observed: BusUnitNvm,
    pub wire_sequence: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

#[derive(Debug)]
pub struct Latest<T> {
    slot: Mutex<Option<T>>,
}

impl<T: Copy> Default for Latest<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy> Latest<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self { slot: Mutex::new(None) }
    }

    pub fn publish(&self, value: T) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
    }

    pub fn latest(&self) -> Option<T> {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn try_latest(&self) -> Result<Option<T>, Busy> {
        match self.slot.try_lock() {
            Ok(slot) => Ok(*slot),
            Err(TryLockError::Poisoned(poisoned)) => Ok(*poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => Err(Busy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_reads_back_before_the_first_publish() {
        let latest = Latest::<u32>::new();
        assert_eq!(latest.latest(), None);
        assert_eq!(latest.try_latest(), Ok(None));
    }

    #[test]
    fn the_last_value_published_is_the_one_read() {
        let latest = Latest::new();
        latest.publish(1u32);
        latest.publish(2);
        assert_eq!(latest.latest(), Some(2));
        assert_eq!(latest.try_latest(), Ok(Some(2)));
    }

    #[test]
    fn a_reader_that_must_not_wait_is_told_the_slot_is_busy() {
        let latest = Latest::new();
        latest.publish(1u32);
        let held = latest.slot.lock().expect("unpoisoned");
        assert_eq!(latest.try_latest(), Err(Busy));
        drop(held);
        assert_eq!(latest.try_latest(), Ok(Some(1)));
    }
}
