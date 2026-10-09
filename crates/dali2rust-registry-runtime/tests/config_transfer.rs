mod support;

use std::sync::Arc;

use dali2rust_bus::BusId;
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    fixed_text_32, BusEventPayload, ErrorCode, OperationWorkerSignal, OperationWorkerSignalEvent,
    Origin, RegistrySliceReloadCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::PollerSettingsReadPort;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_registry_runtime::{
    encode_persistence_blob, slice_key_from_name, PersistablePollerSettingsSlice,
    PersistenceEnvelope, RegistryStore, StagedSlice, POLLER_SETTINGS_SLICE_VERSION,
};
use dali2rust_test_support::{recv_event_matching, temp_slice_store, write_slice};
use support::{
    publish_cmd, spawn_registry_stack_with, RegistryStackOptions, RegistryTestStack,
    CONFIRMATION_DEADLINE,
};

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
fn an_import_is_written_and_reloaded_by_the_worker_and_closes_its_operation() {
    let (source, source_slices) = store_with_poller_settings("transfer-source");
    let exported = source
        .export_slice(&source_slices, SliceKey::PollerSettings)
        .expect("exported");
    let target_slices: Arc<dyn SliceStore> = Arc::new(temp_slice_store("transfer-target"));
    let stack = spawn_registry_stack_with(RegistryStackOptions {
        slices: Some(Arc::clone(&target_slices)),
        ..RegistryStackOptions::default()
    });

    let signal = import(&stack, IMPORT_WORKFLOW, &exported);
    assert_eq!(signal.signal, OperationWorkerSignal::WorkerSucceeded, "{signal:?}");
    assert_eq!(stack.store.poller_settings_view().interval_ms, SOURCE_INTERVAL_MS);
    assert_eq!(slices_load(target_slices.as_ref(), SliceKey::PollerSettings), exported);
}

#[test]
fn a_slice_that_does_not_decode_is_refused_without_a_write() {
    let (source, source_slices) = store_with_poller_settings("transfer-broken-source");
    let stored = source
        .export_slice(&source_slices, SliceKey::PollerSettings)
        .expect("exported");
    let target_slices: Arc<dyn SliceStore> = Arc::new(temp_slice_store("transfer-broken"));
    write_slice(target_slices.as_ref(), SliceKey::PollerSettings, &stored);
    let stack = spawn_registry_stack_with(RegistryStackOptions {
        slices: Some(Arc::clone(&target_slices)),
        ..RegistryStackOptions::default()
    });

    let truncated = &stored[..stored.len() - 1];
    for (workflow, broken) in [(IMPORT_WORKFLOW, &b"not a persistence envelope"[..]), (IMPORT_WORKFLOW + 1, truncated)] {
        let signal = import(&stack, workflow, broken);
        assert_eq!(signal.signal, OperationWorkerSignal::WorkerFailed);
        let error = signal.error.as_ref().expect("a refusal names its cause");
        assert_eq!((error.code, error.message.as_str()), (ErrorCode::InvalidValue, "invalid_slice"));
        assert_eq!(slices_load(target_slices.as_ref(), SliceKey::PollerSettings), stored);
    }
}

#[test]
fn a_pass_writes_the_slices_that_decode_and_refuses_the_one_that_does_not() {
    let (source, source_slices) = store_with_poller_settings("transfer-partial-source");
    let exported = source
        .export_slice(&source_slices, SliceKey::PollerSettings)
        .expect("exported");
    let target_slices: Arc<dyn SliceStore> = Arc::new(temp_slice_store("transfer-partial"));
    let stack = spawn_registry_stack_with(RegistryStackOptions {
        slices: Some(Arc::clone(&target_slices)),
        ..RegistryStackOptions::default()
    });
    let staged = vec![
        StagedSlice { key: SliceKey::Policies, bytes: b"not a persistence envelope".to_vec() },
        StagedSlice { key: SliceKey::PollerSettings, bytes: exported.clone() },
    ];
    stack.store.stage_import(IMPORT_WORKFLOW, staged).expect("stage");

    let signal = commit(&stack, IMPORT_WORKFLOW);
    let error = signal.error.as_ref().expect("the refused slice is named");
    assert_eq!((error.code, error.message.as_str()), (ErrorCode::InvalidValue, "invalid_slice"));
    assert_eq!(slices_load(target_slices.as_ref(), SliceKey::PollerSettings), exported);
    assert!(target_slices.load(SliceKey::Policies).is_err(), "the broken slice was not written");
    assert_eq!(stack.store.poller_settings_view().interval_ms, SOURCE_INTERVAL_MS);
}

#[test]
fn an_import_that_meets_another_writer_of_its_slot_is_busy_not_failed() {
    let (source, source_slices) = store_with_poller_settings("transfer-busy-source");
    let exported = source
        .export_slice(&source_slices, SliceKey::PollerSettings)
        .expect("exported");
    let target_slices: Arc<dyn SliceStore> = Arc::new(temp_slice_store("transfer-busy"));
    let stack = spawn_registry_stack_with(RegistryStackOptions {
        slices: Some(Arc::clone(&target_slices)),
        ..RegistryStackOptions::default()
    });

    let other_writer = target_slices.begin_write(SliceKey::PollerSettings).expect("claim");
    let signal = import(&stack, IMPORT_WORKFLOW, &exported);
    drop(other_writer);
    let error = signal.error.as_ref().expect("a busy slot names its cause");
    assert_eq!((error.code, error.message.as_str()), (ErrorCode::OperationFailed, "flash_busy"));
}

#[test]
fn a_commit_with_nothing_staged_under_its_workflow_writes_nothing() {
    let target_slices: Arc<dyn SliceStore> = Arc::new(temp_slice_store("transfer-unstaged"));
    let stack = spawn_registry_stack_with(RegistryStackOptions {
        slices: Some(Arc::clone(&target_slices)),
        ..RegistryStackOptions::default()
    });
    stack
        .store
        .stage_import(IMPORT_WORKFLOW + 1, vec![StagedSlice { key: SliceKey::PollerSettings, bytes: vec![1] }])
        .expect("stage");

    let signal = commit(&stack, IMPORT_WORKFLOW);
    let error = signal.error.as_ref().expect("a refusal names its cause");
    assert_eq!((error.code, error.message.as_str()), (ErrorCode::InvalidValue, "import_no_stage"));
    assert!(target_slices.load(SliceKey::PollerSettings).is_err());
}

const IMPORT_WORKFLOW: u64 = 41;

fn import(stack: &RegistryTestStack, workflow: u64, bytes: &[u8]) -> OperationWorkerSignalEvent {
    stack
        .store
        .stage_import(workflow, vec![StagedSlice { key: SliceKey::PollerSettings, bytes: bytes.to_vec() }])
        .expect("stage");
    commit(stack, workflow)
}

fn commit(stack: &RegistryTestStack, workflow: u64) -> OperationWorkerSignalEvent {
    publish_cmd(
        &stack.publisher,
        command_envelope(
            SOURCE_ID_UNSPECIFIED,
            workflow,
            BusId::default().0,
            Some(Origin::Api),
            RegistrySliceReloadCommand { slice_name: fixed_text_32("poller_settings") },
        ),
    );
    let event = recv_event_matching(&stack.ev_rx, CONFIRMATION_DEADLINE, |payload| {
        matches!(payload, BusEventPayload::OperationWorkerSignalEvent(s) if s.workflow_correlation_id == workflow)
    });
    let BusEventPayload::OperationWorkerSignalEvent(signal) = &event.payload else {
        unreachable!("matched above");
    };
    signal.clone()
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

fn slices_load(slices: &dyn SliceStore, key: SliceKey) -> Vec<u8> {
    slices.load(key).expect("stored slice")
}
