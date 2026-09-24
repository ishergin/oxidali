use dali2rust_domain::registry::PollerSettingsReadPort;
use dali2rust_platform::slice_store::SliceKey;
use dali2rust_registry_runtime::{
    encode_persistence_blob, slice_key_from_name, PersistablePollerSettingsSlice,
    PersistenceEnvelope, RegistryStore, POLLER_SETTINGS_SLICE_VERSION,
};
use dali2rust_test_support::{temp_slice_store, write_slice};

const ADAPTERS: u8 = 1;
const SOURCE_INTERVAL_MS: u32 = 9_000;

fn store_with_poller_settings(
    label: &str,
) -> (
    RegistryStore,
    dali2rust_bsp::slice_store_files::FileSliceStore,
) {
    let slices = temp_slice_store(label);
    let envelope = PersistenceEnvelope::new(
        POLLER_SETTINGS_SLICE_VERSION,
        PersistablePollerSettingsSlice {
            enabled: true,
            interval_ms: SOURCE_INTERVAL_MS,
            attribute_groups_mask: 1,
            include_dt8_color: false,
            skip_unbound_virtual_lamps: true,
            include_energy: false,
            include_diagnostics: false,
        },
    );
    let bytes = encode_persistence_blob(&envelope).expect("encode poller slice");
    write_slice(&slices, SliceKey::PollerSettings, &bytes);
    let store = RegistryStore::with_adapter_count(ADAPTERS);
    let report = store.hydrate_from_store(&slices, ADAPTERS);
    assert!(report.errors.iter().count() == 0, "{:?}", report.errors);
    (store, slices)
}

#[test]
fn an_exported_slice_is_the_bytes_the_next_boot_would_hydrate() {
    let (store, slices) = store_with_poller_settings("transfer-export");
    let exported = store
        .export_slice(&slices, SliceKey::PollerSettings)
        .expect("poller settings exported");
    assert!(!exported.is_empty());
    assert_eq!(exported, slices_load(&slices, SliceKey::PollerSettings));
}

#[test]
fn an_imported_slice_survives_a_rehydrate_onto_a_second_controller() {
    let (source, source_slices) = store_with_poller_settings("transfer-source");
    let exported = source
        .export_slice(&source_slices, SliceKey::PollerSettings)
        .expect("exported");

    let target_slices = temp_slice_store("transfer-target");
    let target = RegistryStore::with_adapter_count(ADAPTERS);
    target
        .import_slice(&target_slices, SliceKey::PollerSettings, &exported)
        .expect("import");
    let report = target.hydrate_from_store(&target_slices, ADAPTERS);
    assert!(report.errors.iter().count() == 0, "{:?}", report.errors);

    assert_eq!(
        target.poller_settings_view().interval_ms,
        SOURCE_INTERVAL_MS,
        "the imported interval must be the source's"
    );
    assert_eq!(
        source.poller_settings_view().interval_ms,
        SOURCE_INTERVAL_MS
    );
}

#[test]
fn a_slice_the_transfer_may_not_carry_has_no_key() {
    assert_eq!(slice_key_from_name("dali_settings", ADAPTERS), None);
    assert_eq!(slice_key_from_name("redundancy_settings", ADAPTERS), None);
    assert!(slice_key_from_name("home_assistant_settings", ADAPTERS).is_some());
    assert!(slice_key_from_name("settings", ADAPTERS).is_some());
    assert!(slice_key_from_name("poller_settings", ADAPTERS).is_some());
}

#[test]
fn a_manifest_says_which_slices_are_stored_and_which_are_simply_absent() {
    let (store, slices) = store_with_poller_settings("transfer-manifest");
    let rows = store.slice_manifest(&slices, ADAPTERS);
    let poller = rows
        .iter()
        .find(|r| r.name == "poller_settings")
        .expect("poller row");
    assert!(poller.bytes.is_some_and(|n| n > 0));
    assert!(
        rows.iter().any(|r| r.bytes.is_none()),
        "a fresh controller stores almost nothing; the manifest must say so"
    );
}

fn slices_load(
    slices: &dali2rust_bsp::slice_store_files::FileSliceStore,
    key: SliceKey,
) -> Vec<u8> {
    use dali2rust_platform::slice_store::SliceStore;
    slices.load(key).expect("stored slice")
}
