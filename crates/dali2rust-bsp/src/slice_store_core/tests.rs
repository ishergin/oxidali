use super::*;
use std::sync::Mutex;

use dali2rust_platform::slice_store::SliceKey;

struct FakeNor {
    bytes: Mutex<Vec<u8>>,
    write_budget: Mutex<Option<usize>>,
    erase_fails: Mutex<bool>,
}

impl FakeNor {
    fn new(size: usize) -> Self {
        Self {
            bytes: Mutex::new(vec![ERASED_BYTE; size]),
            write_budget: Mutex::new(None),
            erase_fails: Mutex::new(false),
        }
    }

    fn fail_erases(&self, fail: bool) {
        *self.erase_fails.lock().unwrap() = fail;
    }

    fn tear_after(&self, bytes: usize) {
        *self.write_budget.lock().unwrap() = Some(bytes);
    }

    fn heal(&self) {
        *self.write_budget.lock().unwrap() = None;
    }

    fn peek(&self, offset: u32, len: usize) -> Vec<u8> {
        let guard = self.bytes.lock().unwrap();
        guard[offset as usize..offset as usize + len].to_vec()
    }
}

impl RawFlash for FakeNor {
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<(), StoreError> {
        let guard = self.bytes.lock().unwrap();
        let start = offset as usize;
        let end = start + buf.len();
        if end > guard.len() {
            return Err(StoreError::Backend("read past the device".to_string()));
        }
        buf.copy_from_slice(&guard[start..end]);
        Ok(())
    }

    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), StoreError> {
        let mut budget = self.write_budget.lock().unwrap();
        let allowed = match budget.as_mut() {
            None => bytes.len(),
            Some(0) => return Err(StoreError::Backend("power lost".to_string())),
            Some(left) => {
                let take = (*left).min(bytes.len());
                *left -= take;
                take
            }
        };
        drop(budget);
        let mut guard = self.bytes.lock().unwrap();
        let start = offset as usize;
        if start + bytes.len() > guard.len() {
            return Err(StoreError::Backend("write past the device".to_string()));
        }
        for (slot, byte) in guard[start..start + allowed].iter_mut().zip(bytes) {
            *slot &= *byte;
        }
        if allowed < bytes.len() {
            return Err(StoreError::Backend("power lost mid-write".to_string()));
        }
        Ok(())
    }

    fn erase(&self, offset: u32, len: u32) -> Result<(), StoreError> {
        if *self.erase_fails.lock().unwrap() {
            return Err(StoreError::Backend("erase refused".to_string()));
        }
        let mut guard = self.bytes.lock().unwrap();
        let start = offset as usize;
        let end = start + len as usize;
        if end > guard.len() {
            return Err(StoreError::Backend("erase past the device".to_string()));
        }
        guard[start..end].fill(ERASED_BYTE);
        Ok(())
    }

    fn crc32(&self, seed: u32, bytes: &[u8]) -> u32 {
        let mut crc = !seed;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }
}

fn store() -> SliceStoreCore<FakeNor> {
    SliceStoreCore::new(FakeNor::new(crate::slice_layout::LAYOUT_BYTES as usize))
}

const KEY: SliceKey = SliceKey::PollerSettings;

fn write_slice(store: &SliceStoreCore<FakeNor>, payload: &[u8]) -> Result<(), StoreError> {
    let mut session = store.begin_write(KEY)?;
    session.append(payload)?;
    session.commit()
}

fn geometry() -> SlotGeometry {
    slot_geometry(KEY).expect("the poller settings slot is reserved")
}

#[test]
fn a_written_slice_reads_back() {
    let store = store();
    write_slice(&store, b"first revision").expect("write");
    assert_eq!(store.load(KEY).expect("load"), b"first revision");
}

#[test]
fn an_empty_device_reports_the_slice_missing() {
    let store = store();
    assert!(matches!(store.load(KEY), Err(StoreError::Missing)));
}

#[test]
fn consecutive_writes_alternate_banks() {
    let store = store();
    let slot = geometry();
    write_slice(&store, b"one").expect("first");
    let first_bank_magic = store.flash.peek(slot.bank_offset(0), 4);
    assert_eq!(
        first_bank_magic,
        BANK_MAGIC.to_le_bytes(),
        "the first write goes to bank 0"
    );
    write_slice(&store, b"two").expect("second");
    assert_eq!(
        store.flash.peek(slot.bank_offset(1), 4),
        BANK_MAGIC.to_le_bytes(),
        "the second write must go to the OTHER bank, or there is no previous \
         revision to fall back to"
    );
    assert_eq!(store.load(KEY).expect("load"), b"two");
}

#[test]
fn a_torn_append_leaves_the_previous_revision_readable() {
    let store = store();
    write_slice(&store, b"good revision").expect("first");

    store.flash.tear_after(4);
    let torn = write_slice(&store, b"a much longer second revision");
    assert!(torn.is_err(), "the write must fail, not silently truncate");
    store.flash.heal();

    assert_eq!(
        store.load(KEY).expect("the previous revision must survive"),
        b"good revision",
        "one revision old beats gone"
    );
}

#[test]
fn a_write_after_a_torn_one_does_not_erase_the_only_good_copy() {
    let store = store();
    write_slice(&store, b"good revision").expect("first");
    store.flash.tear_after(4);
    let _ = write_slice(&store, b"a much longer second revision");
    store.flash.heal();
    assert_eq!(store.load(KEY).expect("load"), b"good revision");

    write_slice(&store, b"third revision").expect("recovery write");
    assert_eq!(
        store.load(KEY).expect("load"),
        b"third revision",
        "the recovery write must land somewhere readable"
    );
}

#[test]
fn a_dirty_bank_is_erased_before_it_is_reused() {
    let store = store();
    let slot = geometry();

    store
        .flash
        .write(slot.bank_offset(0) + HEADER_BYTES, &[0b1010_1010; 8])
        .expect("stage a dirty bank");
    assert!(
        !store.is_erased(&slot, 0).expect("scan"),
        "precondition: the bank is dirty below its erased header"
    );

    write_slice(&store, &[0b0101_0101; 8]).expect("write");
    assert_eq!(
        store.load(KEY).expect("load"),
        vec![0b0101_0101; 8],
        "the payload must be what was written, not ANDed with the leftovers \
         (0b1010_1010 & 0b0101_0101 == 0)"
    );
}

#[test]
fn the_erase_check_reads_past_the_header() {
    let store = store();
    let slot = geometry();
    store
        .flash
        .write(slot.bank_offset(0) + slot.bank_bytes - 1, &[0x00])
        .expect("dirty the tail");
    assert!(
        !store.is_erased(&slot, 0).expect("scan"),
        "a header-only check would call this bank erased"
    );
}

#[test]
fn a_commit_erases_the_bank_it_superseded() {
    let store = store();
    let slot = geometry();
    write_slice(&store, b"one").expect("first");
    write_slice(&store, b"two").expect("second");
    assert!(
        store.is_erased(&slot, 0).expect("scan"),
        "the superseded bank must be erased after the commit that replaced it"
    );
}

#[test]
fn seq_keeps_increasing_across_a_torn_write() {
    let store = store();
    let slot = geometry();
    write_slice(&store, b"one").expect("first");
    write_slice(&store, b"two").expect("second");
    let seq_before = u32::from_le_bytes(
        store.flash.peek(slot.bank_offset(1) + 4, 4)[..]
            .try_into()
            .unwrap(),
    );

    store.flash.tear_after(2);
    let _ = write_slice(&store, b"three");
    store.flash.heal();
    assert_eq!(store.load(KEY).expect("load"), b"two");

    write_slice(&store, b"four").expect("recovery");
    let readable = store.candidate_banks(&slot).expect("scan");
    let newest = readable.first().expect("a readable bank");
    assert!(
        newest.1.seq > seq_before,
        "a recovery write reused seq {} against {seq_before}",
        newest.1.seq
    );
}

#[test]
fn an_oversize_payload_is_refused() {
    let store = store();
    let slot = geometry();
    let mut session = store.begin_write(KEY).expect("begin");
    let too_big = vec![0u8; (slot.bank_bytes - HEADER_BYTES + 1) as usize];
    assert!(matches!(
        session.append(&too_big),
        Err(StoreError::TooLarge { .. })
    ));
    assert!(session.commit().is_err());
    assert!(matches!(store.load(KEY), Err(StoreError::Missing)));
}

#[test]
fn a_header_claiming_more_than_the_bank_is_ignored() {
    let store = store();
    let slot = geometry();
    write_slice(&store, b"real").expect("write");
    let forged = BankHeader {
        seq: 9_999,
        len: slot.bank_bytes,
        crc: 0,
    };
    store
        .flash
        .write(slot.bank_offset(1), &forged.encode())
        .expect("forge");
    assert_eq!(
        store.load(KEY).expect("load"),
        b"real",
        "an impossible length must not displace a real revision"
    );
}

#[test]
fn a_bank_failing_its_crc_falls_through_to_the_older_one() {
    let store = store();
    let slot = geometry();
    write_slice(&store, b"older revision").expect("first");
    store.flash.fail_erases(true);
    write_slice(&store, b"newer revision").expect("second");
    store.flash.fail_erases(false);
    assert_eq!(store.load(KEY).expect("load"), b"newer revision");
    assert!(
        !store.is_erased(&slot, 0).expect("scan"),
        "precondition: the deferred erase left the older revision in place"
    );

    store
        .flash
        .write(slot.bank_offset(1) + HEADER_BYTES, &[0x00])
        .expect("corrupt");

    assert_eq!(
        store.load(KEY).expect("the older bank must answer"),
        b"older revision",
        "a CRC failure on the newest bank must fall through, not report missing"
    );
}

#[test]
fn a_deferred_erase_is_performed_by_the_next_write() {
    let store = store();
    write_slice(&store, &[0xF0; 8]).expect("first");
    store.flash.fail_erases(true);
    write_slice(&store, &[0x0F; 8]).expect("second");
    store.flash.fail_erases(false);

    write_slice(&store, &[0xCC; 8]).expect("third");
    assert_eq!(
        store.load(KEY).expect("load"),
        vec![0xCC; 8],
        "0xF0 & 0xCC == 0xC0 — anything but 0xCC means the bank was reused dirty"
    );
}
