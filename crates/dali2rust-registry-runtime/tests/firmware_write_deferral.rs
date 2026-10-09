use dali2rust_platform::flash_gate;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_registry_runtime::{
    encode_persistence_blob, ImportWriteFailure, PersistablePollerSettingsSlice,
    PersistenceEnvelope, RegistryStore, StagedSlice, POLLER_SETTINGS_SLICE_VERSION,
};
use dali2rust_test_support::temp_slice_store;

#[test]
fn an_import_refuses_to_write_while_a_firmware_write_is_open() {
    let bytes = encode_persistence_blob(&PersistenceEnvelope::new(
        POLLER_SETTINGS_SLICE_VERSION,
        PersistablePollerSettingsSlice {
            enabled: false,
            interval_ms: 7_000,
            attribute_groups_mask: 1,
            include_dt8_color: false,
            skip_unbound_virtual_lamps: true,
            include_energy: false,
            include_diagnostics: false,
        },
    ))
    .expect("encode poller slice");
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
