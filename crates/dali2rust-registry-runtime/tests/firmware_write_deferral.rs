use dali2rust_platform::flash_gate;
use dali2rust_platform::slice_store::{SliceKey, StoreError};
use dali2rust_registry_runtime::{
    encode_persistence_blob, PersistablePollerSettingsSlice, PersistenceEnvelope, RegistryStore,
    POLLER_SETTINGS_SLICE_VERSION,
};
use dali2rust_test_support::temp_slice_store;

#[test]
fn an_import_waits_out_a_firmware_write_by_refusing_it() {
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
    let slices = temp_slice_store("import-during-firmware-write");
    let store = RegistryStore::with_adapter_count(1);

    flash_gate::set_firmware_write_open(true);
    let refused = store.import_slice(&slices, SliceKey::PollerSettings, &bytes);
    flash_gate::set_firmware_write_open(false);

    assert!(
        matches!(refused, Err(StoreError::Backend(ref why)) if why.contains("firmware update")),
        "an import must not queue behind a firmware write on the httpd task: {refused:?}"
    );
    store
        .import_slice(&slices, SliceKey::PollerSettings, &bytes)
        .expect("the same import once the write has closed");
}
