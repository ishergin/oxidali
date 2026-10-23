mod support;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dali2rust_domain::registry::PollerSettingsReadPort;
use dali2rust_platform::flash_gate;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_registry_runtime::{
    encode_persistence_blob, ImportWriteFailure, PersistablePollerSettingsSlice,
    PersistenceEnvelope, RegistryStore, StagedSlice, HELD_SLICE_REREAD_INTERVAL,
    POLLER_SETTINGS_SLICE_VERSION,
};
use dali2rust_test_support::{remains_false_for, temp_slice_store, wait_until, UnreadableSlices};
use support::{spawn_registry_stack_with, RegistryStackOptions};

static FIRMWARE_WRITE: Mutex<()> = Mutex::new(());

fn hold_the_firmware_gate_alone() -> MutexGuard<'static, ()> {
    FIRMWARE_WRITE.lock().unwrap_or_else(PoisonError::into_inner)
}

const STORED_POLLER_INTERVAL_MS: u32 = 7_000;
const REREAD_ATTEMPTS: u32 = 3;

fn stored_poller_settings() -> Vec<u8> {
    encode_persistence_blob(&PersistenceEnvelope::new(
        POLLER_SETTINGS_SLICE_VERSION,
        PersistablePollerSettingsSlice {
            enabled: false,
            interval_ms: STORED_POLLER_INTERVAL_MS,
            attribute_groups_mask: 1,
            include_dt8_color: false,
            skip_unbound_virtual_lamps: true,
            include_energy: false,
            include_diagnostics: false,
        },
    ))
    .expect("encode poller slice")
}

#[test]
fn an_import_refuses_to_write_while_a_firmware_write_is_open() {
    let _alone = hold_the_firmware_gate_alone();
    let bytes = stored_poller_settings();
    let staged = [StagedSlice { key: SliceKey::PollerSettings, bytes: bytes.clone() }];
    let slices = temp_slice_store("import-during-firmware-write");
    let store = RegistryStore::with_adapter_count(1);

    flash_gate::set_firmware_write_open(true);
    let refused = store.write_staged(&slices, &staged);
    flash_gate::set_firmware_write_open(false);

    assert!(
        matches!(refused, Err(ImportWriteFailure::FirmwareWriteOpen)),
        "an import must not write beside a firmware write: {refused:?}"
    );
    assert!(slices.load(SliceKey::PollerSettings).is_err(), "nothing was written");
    store.write_staged(&slices, &staged).expect("the same import once the write has closed");
    assert_eq!(slices.load(SliceKey::PollerSettings).expect("written"), bytes);
}

fn unread(store: &RegistryStore) -> u32 {
    store.persistence_counters().unread_slices.load(Ordering::Acquire)
}

fn booted_with_poller_settings_unread() -> (Arc<UnreadableSlices>, Arc<RegistryStore>) {
    let slices = Arc::new(UnreadableSlices::default());
    let staged = [StagedSlice { key: SliceKey::PollerSettings, bytes: stored_poller_settings() }];
    let installer = RegistryStore::with_adapter_count(1);
    installer.write_staged(slices.as_ref(), &staged).expect("installed");
    slices.fail_reads_of(SliceKey::PollerSettings);
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    store.hydrate_from_store(slices.as_ref(), 1);
    assert_eq!(unread(&store), 1, "the poller settings slice was read at boot");
    (slices, store)
}

#[test]
fn a_held_slice_is_not_read_again_while_a_firmware_write_is_open() {
    let _alone = hold_the_firmware_gate_alone();
    let (slices, store) = booted_with_poller_settings_unread();
    flash_gate::set_firmware_write_open(true);
    slices.read_everything();
    let options = RegistryStackOptions {
        slices: Some(slices.clone()),
        store: Some(Arc::clone(&store)),
        ..RegistryStackOptions::default()
    };
    let _stack = spawn_registry_stack_with(options);
    let window = HELD_SLICE_REREAD_INTERVAL.saturating_mul(2);
    let held = remains_false_for(|| unread(&store) == 0, window);
    flash_gate::set_firmware_write_open(false);
    assert!(held, "the held slice was read beside an open firmware write");

    let deadline = HELD_SLICE_REREAD_INTERVAL.saturating_mul(REREAD_ATTEMPTS);
    wait_until(|| unread(&store) == 0, deadline.saturating_add(Duration::from_secs(1)));
    assert_eq!(store.poller_settings_view().interval_ms, STORED_POLLER_INTERVAL_MS);
}
