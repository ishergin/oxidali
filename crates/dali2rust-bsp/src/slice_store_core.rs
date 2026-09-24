use core::sync::atomic::{AtomicU8, Ordering};

use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

use crate::slice_layout::{
    choose_bank, slot_geometry, slot_index, BankChoice, SlotGeometry, BANK_HEADER_BYTES as HEADER_BYTES,
    TOTAL_SLOT_COUNT,
};

const BANK_MAGIC: u32 = 0x4453_4C31;

const ERASED_BYTE: u8 = 0xFF;

const ERASE_SCAN_CHUNK_BYTES: usize = 256;

const IO_CHUNK_BYTES: usize = 1024;

pub trait RawFlash: Send + Sync {
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<(), StoreError>;
    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), StoreError>;
    fn erase(&self, offset: u32, len: u32) -> Result<(), StoreError>;
    fn crc32(&self, seed: u32, bytes: &[u8]) -> u32;
    fn note_commit(&self) {}
}

pub(crate) struct BankHeader {
    pub seq: u32,
    pub len: u32,
    pub crc: u32,
}

impl BankHeader {
    fn decode(raw: &[u8; HEADER_BYTES as usize]) -> Option<Self> {
        let word = |i: usize| u32::from_le_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
        (word(0) == BANK_MAGIC).then(|| Self {
            seq: word(4),
            len: word(8),
            crc: word(12),
        })
    }

    fn encode(&self) -> [u8; HEADER_BYTES as usize] {
        let mut raw = [0u8; HEADER_BYTES as usize];
        raw[0..4].copy_from_slice(&BANK_MAGIC.to_le_bytes());
        raw[4..8].copy_from_slice(&self.seq.to_le_bytes());
        raw[8..12].copy_from_slice(&self.len.to_le_bytes());
        raw[12..16].copy_from_slice(&self.crc.to_le_bytes());
        raw
    }
}

#[derive(Default)]
struct SlotMemory {
    served: AtomicU8,
    erased: AtomicU8,
}

impl SlotMemory {
    fn get(cell: &AtomicU8) -> Option<u8> {
        match cell.load(Ordering::Relaxed) {
            0 => None,
            packed => Some(packed - 1),
        }
    }

    fn set(cell: &AtomicU8, bank: u8) {
        cell.store(bank + 1, Ordering::Relaxed);
    }

    fn clear(cell: &AtomicU8) {
        cell.store(0, Ordering::Relaxed);
    }
}

pub struct SliceStoreCore<F: RawFlash> {
    flash: F,
    slots: Vec<SlotMemory>,
}

impl<F: RawFlash> SliceStoreCore<F> {
    pub fn new(flash: F) -> Self {
        Self {
            flash,
            slots: (0..TOTAL_SLOT_COUNT).map(|_| SlotMemory::default()).collect(),
        }
    }

    fn memory(&self, key: SliceKey) -> Option<&SlotMemory> {
        slot_index(key).and_then(|index| self.slots.get(index as usize))
    }

    fn erase_bank(&self, slot: &SlotGeometry, bank: u8) -> Result<(), StoreError> {
        self.flash.erase(slot.bank_offset(bank), slot.bank_bytes)
    }

    fn header(&self, slot: &SlotGeometry, bank: u8) -> Result<Option<BankHeader>, StoreError> {
        let mut raw = [0u8; HEADER_BYTES as usize];
        self.flash.read(slot.bank_offset(bank), &mut raw)?;
        Ok(BankHeader::decode(&raw))
    }

    fn candidate_banks(&self, slot: &SlotGeometry) -> Result<Vec<(u8, BankHeader)>, StoreError> {
        let mut found: Vec<(u8, BankHeader)> = Vec::new();
        for bank in 0..2u8 {
            let Some(header) = self.header(slot, bank)? else {
                continue;
            };
            if header.len > slot.bank_bytes - HEADER_BYTES {
                continue;
            }
            found.push((bank, header));
        }
        found.sort_unstable_by(|(_, a), (_, b)| b.seq.cmp(&a.seq));
        Ok(found)
    }

    fn current_bank(&self, slot: &SlotGeometry) -> Result<Option<(u8, BankHeader)>, StoreError> {
        Ok(self.candidate_banks(slot)?.into_iter().next())
    }

    fn is_erased(&self, slot: &SlotGeometry, bank: u8) -> Result<bool, StoreError> {
        let mut buf = [0u8; ERASE_SCAN_CHUNK_BYTES];
        let base = slot.bank_offset(bank);
        let mut offset = 0u32;
        while offset < slot.bank_bytes {
            let take = ERASE_SCAN_CHUNK_BYTES.min((slot.bank_bytes - offset) as usize);
            let chunk = &mut buf[..take];
            self.flash.read(base + offset, chunk)?;
            if chunk.iter().any(|b| *b != ERASED_BYTE) {
                return Ok(false);
            }
            offset += take as u32;
        }
        Ok(true)
    }
}

impl<F: RawFlash> SliceStore for SliceStoreCore<F> {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        let slot = slot_geometry(key).ok_or(StoreError::Missing)?;
        let mut last_err = StoreError::Missing;
        for (bank, header) in self.candidate_banks(&slot)? {
            let mut payload = vec![0u8; header.len as usize];
            let base = slot.bank_offset(bank) + HEADER_BYTES;
            let mut read_failed = false;
            for (index, chunk) in payload.chunks_mut(IO_CHUNK_BYTES).enumerate() {
                if let Err(e) = self.flash.read(base + (index * IO_CHUNK_BYTES) as u32, chunk) {
                    last_err = e;
                    read_failed = true;
                    break;
                }
            }
            if read_failed {
                continue;
            }
            if self.flash.crc32(0, &payload) != header.crc {
                log::warn!(
                    "slice store: {} bank {bank} (seq {}) failed CRC; trying the older bank",
                    key.label(),
                    header.seq,
                );
                continue;
            }
            if let Some(memory) = self.memory(key) {
                SlotMemory::set(&memory.served, bank);
            }
            return Ok(payload);
        }
        Err(last_err)
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        let slot = slot_geometry(key)
            .ok_or_else(|| StoreError::Backend(format!("no slot reserved for {}", key.label())))?;
        let current = self.current_bank(&slot)?;
        let memory = self.memory(key);
        let seq = current.as_ref().map_or(1, |(_, header)| header.seq + 1);
        let BankChoice { keep, target } = choose_bank(
            memory.and_then(|m| SlotMemory::get(&m.served)),
            current.as_ref().map(|(bank, _)| *bank),
        );

        let known_erased = memory
            .and_then(|m| SlotMemory::get(&m.erased))
            .is_some_and(|bank| bank == target);
        if !known_erased && !self.is_erased(&slot, target)? {
            self.erase_bank(&slot, target)?;
        }
        if let Some(memory) = memory {
            SlotMemory::clear(&memory.erased);
        }
        Ok(Box::new(CoreWriteSession {
            store: self,
            slot,
            key,
            target,
            stale: keep,
            seq,
            written: 0,
            crc: 0,
            failed: false,
        }))
    }
}

struct CoreWriteSession<'a, F: RawFlash> {
    store: &'a SliceStoreCore<F>,
    slot: SlotGeometry,
    key: SliceKey,
    target: u8,
    stale: Option<u8>,
    seq: u32,
    written: u32,
    crc: u32,
    failed: bool,
}

impl<F: RawFlash> CoreWriteSession<'_, F> {
    fn capacity(&self) -> u32 {
        self.slot.bank_bytes - HEADER_BYTES
    }
}

impl<F: RawFlash> SliceWriteSession for CoreWriteSession<'_, F> {
    fn append(&mut self, chunk: &[u8]) -> Result<(), StoreError> {
        if self.failed {
            return Err(StoreError::Backend("session already failed".to_string()));
        }
        let end = self.written + chunk.len() as u32;
        if end > self.capacity() {
            self.failed = true;
            return Err(StoreError::TooLarge {
                len: end as usize,
                capacity: self.capacity() as usize,
            });
        }
        let offset = self.slot.bank_offset(self.target) + HEADER_BYTES + self.written;
        if let Err(e) = self.store.flash.write(offset, chunk) {
            self.failed = true;
            return Err(e);
        }
        self.crc = self.store.flash.crc32(self.crc, chunk);
        self.written = end;
        Ok(())
    }

    fn commit(self: Box<Self>) -> Result<(), StoreError> {
        if self.failed {
            return Err(StoreError::Backend("commit after failed append".to_string()));
        }
        let header = BankHeader {
            seq: self.seq,
            len: self.written,
            crc: self.crc,
        };
        self.store.flash.note_commit();
        self.store
            .flash
            .write(self.slot.bank_offset(self.target), &header.encode())?;

        if let Some(memory) = self.store.memory(self.key) {
            SlotMemory::set(&memory.served, self.target);
        }
        if let Some(stale) = self.stale {
            match self.store.erase_bank(&self.slot, stale) {
                Ok(()) => {
                    if let Some(memory) = self.store.memory(self.key) {
                        SlotMemory::set(&memory.erased, stale);
                    }
                }
                Err(e) => log::warn!("slice store: stale bank erase deferred: {e}"),
            }
        }
        Ok(())
    }

    fn abort(self: Box<Self>) {
    }
}

#[cfg(test)]
mod tests;
