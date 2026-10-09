mod support;

use support::{publish_event, publish_group_matrix_write, publish_scene_matrix_write};

use dali2rust_contracts::msg::DaliAttributeReadChunk;
use dali2rust_contracts::msg::PhysicalDeviceOverrideCommand as PdPatch;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusFrame, PublishResult};
use dali2rust_contracts::msg::{
    BusEventPayload, ColorMode, DaliProgramTarget, DeviceType, ErrorCode, GroupMatrixDesiredRow,
    GroupMatrixDesiredRowList, GroupMembershipAction,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{
    AdapterReadPort, AdapterView, AttributeSource, GroupApplySnapshot, GroupMembershipMatrixView,
    GroupReadPort, GroupView, SceneReadPort, VirtualLampReadPort,
};
use dali2rust_registry_runtime::{RegistryStore, RegistryWorkerCounters};
use dali2rust_test_support::wait_until;
use dali2rust_domain::dali::banks::part251;

const BUS_TID: u16 = 1;

fn drain_events(rx: &std::sync::mpsc::Receiver<BusFrame>) {
    while rx.recv_timeout(Duration::from_millis(20)).is_ok() {}
}

fn assert_no_pd_changed_for(rx: &std::sync::mpsc::Receiver<BusFrame>, correlation_id: u64) {
    for _ in 0..8 {
        match rx.recv_timeout(Duration::from_millis(80)) {
            Ok(BusFrame::Event(ev))
                if matches!(ev.payload, BusEventPayload::PhysicalDeviceChangedEvent(_))
                    && ev.meta.correlation_id == correlation_id =>
            {
                panic!("unexpected PhysicalDeviceChanged for correlation {correlation_id}");
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn assert_no_group_matrix_changed_for(rx: &std::sync::mpsc::Receiver<BusFrame>, correlation_id: u64) {
    for _ in 0..8 {
        match rx.recv_timeout(Duration::from_millis(80)) {
            Ok(BusFrame::Event(ev))
                if matches!(ev.payload, BusEventPayload::GroupMatrixChangedEvent(_))
                    && ev.meta.correlation_id == correlation_id =>
            {
                panic!("unexpected GroupMatrixChanged for correlation {correlation_id}");
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn spawn_registry_stack(
    store: Arc<RegistryStore>,
    counters: Arc<RegistryWorkerCounters>,
) -> (
    dali2rust_bus::BusPublisher,
    std::sync::mpsc::Receiver<BusFrame>,
    dali2rust_bus::BusHost,
) {
    let s = support::spawn_registry_stack_with(support::RegistryStackOptions {
        store: Some(store),
        counters: Some(counters),
        ..support::RegistryStackOptions::default()
    });
    (s.publisher, s.ev_rx, s._host)
}

fn group_rows(rows: &[GroupMatrixDesiredRow]) -> GroupMatrixDesiredRowList {
    let mut out = GroupMatrixDesiredRowList::new();
    for row in rows {
        out.push(*row).expect("group row");
    }
    out
}

fn publish_discovery(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    progress: dali2rust_contracts::msg::DaliDiscoveryProgressEvent,
) {
    let short = progress.short_address;
    let origin = Some(dali2rust_contracts::msg::Origin::Internal);
    let event = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        1,
        BUS_TID,
        origin,
        progress,
    );
    publish_event(publisher, event);
    wait_until(
        || store.physical_device_view(0, short).is_some(),
        Duration::from_millis(500),
    );
}

fn seed_physical_via_discovery(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    short: u8,
) {
    publish_discovery(publisher, store, dali2rust_contracts::msg::DaliDiscoveryProgressEvent {
        registry_adapter_id: 0,
        short_address: short,
        random_address: None,
        device_type: DeviceType::Dt8Color,
        color_mode: ColorMode::Rgb,
        dt8_xy_capable: true,
        dt8_tc_capable: true,
        dt8_rgb_capable: true,
        dt8_rgbwaf_capable: false,
        supported_device_types: None,
    });
}

fn written_event(
    corr: u64,
    power_on_level: Option<u8>,
    error: Option<dali2rust_contracts::msg::CompactErrorPayload>,
) -> dali2rust_contracts::msg::EventEnvelope {
    dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesWrittenEvent { short_address: 3, fade_time_ms: None, fade_rate: None, power_on_level, system_failure_level: None, extended_fade_time_ms: None, registry_adapter_id: 0, tc_coolest_mirek: None, tc_warmest_mirek: None, min_level: None, max_level: None, dimming_curve: None, error })
}

#[test]
fn a_written_event_that_confirms_nothing_changes_nothing() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_physical_via_discovery(&publisher, &store, 3);
    let before = counters.events.dali_attributes_committed.load(Ordering::Relaxed);
    let unanswered = dali2rust_contracts::msg::CompactErrorPayload::new(
        ErrorCode::VerifyUnanswered,
        "verify_unanswered",
    );

    publish_event(&publisher, written_event(11, None, Some(unanswered)));
    publish_event(&publisher, written_event(12, Some(200), None));
    wait_until(
        || counters.events.dali_attributes_committed.load(Ordering::Relaxed) == before + 1,
        Duration::from_millis(500),
    );

    let changed_by_empty = std::iter::from_fn(|| ev_obs.try_recv().ok()).any(|frame| {
        matches!(&frame, BusFrame::Event(ev)
            if ev.meta.correlation_id == 11
                && matches!(ev.payload, BusEventPayload::PhysicalDeviceChangedEvent(_)))
    });
    assert!(!changed_by_empty, "an outcome with nothing confirmed is no change to the device");
    assert_eq!(counters.events.dali_attributes_committed.load(Ordering::Relaxed), before + 1);
    let power_on = store.physical_device_view(0, 3).expect("pd").attributes.common102.power_on_level;
    assert_eq!(power_on.map(|v| (v.value, v.source)), Some((200, AttributeSource::WriteConfirmed)));
}

#[test]
fn attributes_written_updates_common102_and_emits_physical_device_changed() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_physical_via_discovery(&publisher, &store, 3);

    let before = counters.events.dali_attributes_committed.load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 10, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesWrittenEvent { short_address: 3, fade_time_ms: Some(700), fade_rate: Some(7), power_on_level: Some(200), system_failure_level: Some(201), extended_fade_time_ms: Some(900), registry_adapter_id: 0 , tc_coolest_mirek: None, tc_warmest_mirek: None, min_level: None, max_level: None, dimming_curve: None, error: None }),
    );
    wait_until(
        || counters.events.dali_attributes_committed.load(Ordering::Relaxed) == before + 1,
        Duration::from_millis(500),
    );
    assert_eq!(
        counters.events.dali_attributes_committed.load(Ordering::Relaxed),
        before + 1
    );

    let v = store.physical_device_view(0, 3).expect("pd");
    let c102 = &v.attributes.common102;
    assert_eq!(c102.fade_time_ms.as_ref().unwrap().value, 700);
    assert_eq!(c102.fade_time_ms.as_ref().unwrap().source, AttributeSource::WriteConfirmed);
    assert_eq!(c102.fade_rate.as_ref().unwrap().value, 7);

    let target_corr = 10u64;
    let mut got_pd_change = false;
    for _ in 0..16 {
        let BusFrame::Event(ev) = ev_obs
            .recv_timeout(Duration::from_millis(400))
            .expect("pd changed")
        else {
            panic!("expected event");
        };
        if !matches!(
            ev.payload,
            BusEventPayload::PhysicalDeviceChangedEvent(_)
        ) {
            continue;
        }
        if ev.meta.correlation_id != target_corr {
            continue;
        }
        got_pd_change = true;
        break;
    }
    assert!(got_pd_change, "expected PhysicalDeviceChanged for correlation {target_corr}");
}

#[test]
fn discovery_progress_updates_capabilities() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    let before = counters.events.discovery_progress_applied.load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 2, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: 4, random_address: None, device_type: DeviceType::Dt6Led, color_mode: ColorMode::Xy, dt8_xy_capable: true, dt8_tc_capable: false, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None }),
    );
    wait_until(
        || counters.events.discovery_progress_applied.load(Ordering::Relaxed) == before + 1,
        Duration::from_millis(500),
    );
    assert_eq!(
        counters.events.discovery_progress_applied.load(Ordering::Relaxed),
        before + 1
    );
    let v = store.physical_device_view(0, 4).expect("pd");
    assert!(v.capabilities.xy);
    assert!(!v.capabilities.cct);
}

#[test]
fn attributes_read_with_evidence_commits_and_increments_counter() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_physical_via_discovery(&publisher, &store, 5);

    let before = counters
        .events.attribute_read_evidence_applied
        .load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 3, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesReadEvent { registry_adapter_id: 0, short_address: 5, last_chunk: true, chunk: DaliAttributeReadChunk::Common102 { version: Some(2), device_type: None, physical_minimum: None, min_level: None, max_level: None, power_on_level: None, system_failure_level: None, fade_time_ms: None, fade_rate: None, supported_device_types: None, light_source_type: None, light_source_types: None } }),
    );
    wait_until(
        || counters
            .events.attribute_read_evidence_applied
            .load(Ordering::Relaxed)
            == before + 1,
        Duration::from_millis(500),
    );
    assert_eq!(
        counters
            .events.attribute_read_evidence_applied
            .load(Ordering::Relaxed),
        before + 1
    );
    let v = store.physical_device_view(0, 5).expect("pd");
    assert_eq!(
        v.attributes.common102.version.as_ref().unwrap().value,
        2
    );
    assert_eq!(
        v.attributes.common102.version.as_ref().unwrap().source,
        AttributeSource::Readback
    );
}

#[test]
fn attributes_read_chunk_for_unknown_device_is_ignored() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    drain_events(&ev_obs);

    let before = counters
        .events.attribute_read_evidence_applied
        .load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 4, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesReadEvent { registry_adapter_id: 0, short_address: 6, last_chunk: true, chunk: DaliAttributeReadChunk::Identity { random_address: 0xAABBCC } }),
    );
    assert_eq!(
        counters
            .events.attribute_read_evidence_applied
            .load(Ordering::Relaxed),
        before
    );
    assert_no_pd_changed_for(&ev_obs, 4);
}

#[test]
fn memory_bank_commits_only_after_last_chunk() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_physical_via_discovery(&publisher, &store, 8);
    drain_events(&ev_obs);

    let corr = 99u64;
    let mb_before = counters.events.memory_bank_read_committed.load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 8, bank: 1, chunk_index: 0, last_chunk: false, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0x01, 0x02]) }),
    );
    assert_no_pd_changed_for(&ev_obs, corr);
    assert_eq!(
        counters.events.memory_bank_read_committed.load(Ordering::Relaxed),
        mb_before
    );
    assert!(store.physical_device_view(0, 8).unwrap().memory_banks.is_empty());

    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 8, bank: 1, chunk_index: 1, last_chunk: true, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0x03]) }),
    );
    wait_until(
        || counters.events.memory_bank_read_committed.load(Ordering::Relaxed) == mb_before + 1,
        Duration::from_millis(500),
    );
    assert_eq!(
        counters.events.memory_bank_read_committed.load(Ordering::Relaxed),
        mb_before + 1
    );
    let banks = store.physical_device_view(0, 8).unwrap().memory_banks;
    assert_eq!(banks.len(), 1);
    assert_eq!(banks[0].bank, 1);
    assert_eq!(banks[0].total_bytes_read, 3);
}

#[test]
fn memory_bank_abort_clears_partial_stage() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_physical_via_discovery(&publisher, &store, 9);
    drain_events(&ev_obs);

    let corr = 77u64;
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 9, bank: 0, chunk_index: 0, last_chunk: false, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0xAA, 0xBB]) }),
    );
    assert_no_pd_changed_for(&ev_obs, corr);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadAbortedEvent {}),
    );

    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 9, bank: 0, chunk_index: 0, last_chunk: true, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0xCC]) }),
    );
    wait_until(
        || store
            .physical_device_view(0, 9)
            .map(|pd| pd.memory_banks.len())
            == Some(1),
        Duration::from_millis(500),
    );
    let banks = store.physical_device_view(0, 9).unwrap().memory_banks;
    assert_eq!(banks.len(), 1);
    assert_eq!(banks[0].total_bytes_read, 1);
}

#[test]
fn events_for_other_bus_adapter_are_ignored() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    let before = counters.events.discovery_progress_applied.load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 5, 99, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: 11, random_address: None, device_type: DeviceType::Dt6Led, color_mode: ColorMode::Unknown, dt8_xy_capable: false, dt8_tc_capable: false, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None }),
    );
    assert_no_pd_changed_for(&ev_obs, 5);
    assert_eq!(
        counters.events.discovery_progress_applied.load(Ordering::Relaxed),
        before
    );
    assert!(store.physical_device_view(0, 11).is_none());
}

#[test]
fn memory_bank_out_of_order_chunk_aborts_without_commit() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_physical_via_discovery(&publisher, &store, 30);
    drain_events(&ev_obs);

    let corr = 120u64;
    let before = counters.events.memory_bank_read_committed.load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 30, bank: 2, chunk_index: 1, last_chunk: false, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0x01]) }),
    );
    assert_no_pd_changed_for(&ev_obs, corr);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 30, bank: 2, chunk_index: 0, last_chunk: true, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0x02]) }),
    );
    assert_no_pd_changed_for(&ev_obs, corr);
    assert_eq!(
        counters.events.memory_bank_read_committed.load(Ordering::Relaxed),
        before
    );
    assert!(store
        .physical_device_view(0, 30)
        .unwrap()
        .memory_banks
        .is_empty());
    assert_no_pd_changed_for(&ev_obs, corr);
}

#[test]
fn memory_bank_identity_mismatch_aborts_staging() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_physical_via_discovery(&publisher, &store, 31);
    drain_events(&ev_obs);

    let corr = 121u64;
    let before = counters.events.memory_bank_read_committed.load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 31, bank: 0, chunk_index: 0, last_chunk: false, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0xAA]) }),
    );
    assert_no_pd_changed_for(&ev_obs, corr);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 31, bank: 1, chunk_index: 1, last_chunk: true, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0xBB]) }),
    );
    assert_no_pd_changed_for(&ev_obs, corr);
    assert_eq!(
        counters.events.memory_bank_read_committed.load(Ordering::Relaxed),
        before
    );
    assert!(store
        .physical_device_view(0, 31)
        .unwrap()
        .memory_banks
        .is_empty());
    assert_no_pd_changed_for(&ev_obs, corr);
}

#[test]
fn memory_bank_final_chunk_without_physical_record_does_not_commit() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    let corr = 122u64;
    let before = counters.events.memory_bank_read_committed.load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 40, bank: 0, chunk_index: 0, last_chunk: true, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&[0xCC]) }),
    );
    assert_no_pd_changed_for(&ev_obs, corr);
    assert_eq!(
        counters.events.memory_bank_read_committed.load(Ordering::Relaxed),
        before
    );
    assert!(store.physical_device_view(0, 40).is_none());
    assert_no_pd_changed_for(&ev_obs, corr);
}

fn setup_discovered_short_and_binding(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    short: u8,
    virtual_lamp_id: u8,
    corr_base: u64,
) {
    publish_event(
        publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr_base, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: short, random_address: None, device_type: DeviceType::Dt6Led, color_mode: ColorMode::Unknown, dt8_xy_capable: false, dt8_tc_capable: false, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None }),
    );
    wait_until(
        || store.physical_device_view(0, short).is_some(),
        Duration::from_millis(500),
    );
    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr_base + 1, BUS_TID, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id, physical_short_address: short })),
        ),
        PublishResult::Queued,
    );
    wait_until(
        || store.virtual_lamp_view(0, virtual_lamp_id).binding_short == Some(short),
        Duration::from_millis(500),
    );
}

#[test]
fn successful_group_membership_event_updates_applied_matrix() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    setup_discovered_short_and_binding(&publisher, &store, 7, 1, 68);
    drain_events(&ev_obs);
    let before = counters
        .events
        .group_membership_programmed_applied
        .load(Ordering::Relaxed);
    publish_group_matrix_write(&publisher, 70, 0, group_rows(&[GroupMatrixDesiredRow {
            virtual_lamp_id: 1,
            desired_groups_mask: 1 << 2,
        }]));
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[1].desired[2] && !view.rows[1].applied[2])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 71, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 1 }, group_id: 2, action: GroupMembershipAction::Add, physical_short_address: Some(7), membership: Some(1 << 2), error: None }),
    );
    wait_until(
        || {
            store
                .group_view(0, 2)
                .map(|group| !group.dirty && group.member_count_applied == 1)
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    let matrix = store.group_membership_matrix_view(0).expect("group matrix");
    assert!(matrix.rows[1].desired[2]);
    assert!(matrix.rows[1].applied[2]);
    let group = store.group_view(0, 2).expect("group view");
    assert!(!group.dirty);
    assert_eq!(group.member_count_desired, 1);
    assert_eq!(group.member_count_applied, 1);
    assert_eq!(
        counters
            .events
            .group_membership_programmed_applied
            .load(Ordering::Relaxed),
        before + 1
    );
    let mut saw_matrix_changed = false;
    for _ in 0..16 {
        let Ok(BusFrame::Event(event)) = ev_obs.recv_timeout(Duration::from_millis(80)) else {
            break;
        };
        if event.meta.correlation_id == 71
            && matches!(event.payload, BusEventPayload::GroupMatrixChangedEvent(_))
        {
            saw_matrix_changed = true;
            break;
        }
    }
    assert!(saw_matrix_changed, "expected GroupMatrixChangedEvent for success projection");
}

#[test]
fn successful_scene_programmed_event_projects_applied_row_and_echo() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    setup_discovered_short_and_binding(&publisher, &store, 7, 1, 90);
    drain_events(&ev_obs);

    let target = dali2rust_contracts::msg::DaliSceneTargetState {
        power: Some(dali2rust_contracts::msg::PowerState::On),
        level: Some(179),
        color: Some(dali2rust_contracts::msg::ColorValue {
            mode: dali2rust_contracts::msg::ColorMode::Cct,
            color_temperature_kelvin: 2700,
            ..Default::default()
        }),
    };
    let mut rows = dali2rust_contracts::msg::SceneMatrixDesiredRowList::new();
    rows.push(dali2rust_contracts::msg::SceneMatrixDesiredRow {
        virtual_lamp_id: 1,
        included: true,
        target: Some(target),
    })
    .expect("row capacity");
    publish_scene_matrix_write(&publisher, 92, 0, 3, rows);
    wait_until(
        || {
            store
                .scene_view(0, 3)
                .map(|scene| scene.dirty && scene.row_count_included == 1)
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    let before = counters.events.scene_programmed_applied.load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            93,
            BUS_TID,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliSceneProgrammedEvent {
                registry_adapter_id: 0,
                target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 1 },
                scene_id: 3,
                action: dali2rust_contracts::msg::SceneProgramAction::Write,
                physical_short_address: Some(7),
                target_state: Some(target),
                scene_level: Some(179),
                error: None,
            },
        ),
    );
    wait_until(
        || {
            store
                .scene_view(0, 3)
                .map(|scene| !scene.dirty)
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    let matrix = store.scene_matrix_view(0, 3).expect("scene matrix");
    let row = &matrix.rows[1];
    assert!(row.applied.included);
    assert_eq!(row.applied.level, Some(179));
    assert_eq!(row.applied.color_mode.as_deref(), Some("cct"));
    assert_eq!(row.applied.color_temperature_kelvin, Some(2700));
    assert!(!row.dirty);
    assert_eq!(
        counters.events.scene_programmed_applied.load(Ordering::Relaxed),
        before + 1
    );
    let mut saw_matrix_changed = false;
    for _ in 0..16 {
        let Ok(BusFrame::Event(event)) = ev_obs.recv_timeout(Duration::from_millis(80)) else {
            break;
        };
        if event.meta.correlation_id == 93
            && matches!(event.payload, BusEventPayload::SceneMatrixChangedEvent(_))
        {
            saw_matrix_changed = true;
            break;
        }
    }
    assert!(saw_matrix_changed, "expected SceneMatrixChangedEvent for scene projection");
}

#[test]
fn failed_group_membership_event_leaves_dirty_matrix() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    publish_group_matrix_write(&publisher, 72, 0, group_rows(&[GroupMatrixDesiredRow {
            virtual_lamp_id: 2,
            desired_groups_mask: 1 << 4,
        }]));
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[2].desired[4] && !view.rows[2].applied[4])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
    drain_events(&ev_obs);

    let before = counters
        .events
        .group_membership_programmed_applied
        .load(Ordering::Relaxed);
    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 73, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 2 }, group_id: 4, action: GroupMembershipAction::Add, physical_short_address: None, membership: None, error: (Some((ErrorCode::OperationFailed, "dali_transport_error"))).map(|(code, message)| dali2rust_contracts::msg::CompactErrorPayload::new(code, message)) }),
    );
    assert_no_group_matrix_changed_for(&ev_obs, 73);
    let matrix = store.group_membership_matrix_view(0).expect("group matrix");
    assert!(matrix.rows[2].desired[4]);
    assert!(!matrix.rows[2].applied[4]);
    let group = store.group_view(0, 4).expect("group view");
    assert!(group.dirty);
    assert_eq!(group.member_count_desired, 1);
    assert_eq!(group.member_count_applied, 0);
    assert_eq!(
        counters
            .events
            .group_membership_programmed_applied
            .load(Ordering::Relaxed),
        before
    );
}

#[test]
fn groups_read_before_the_lamp_is_bound_are_adopted_and_not_reported_dirty() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 900, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: 9, random_address: None, device_type: DeviceType::Dt6Led, color_mode: ColorMode::Unknown, dt8_xy_capable: false, dt8_tc_capable: false, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None }),
    );
    wait_until(
        || store.physical_device_view(0, 9).is_some(),
        Duration::from_millis(500),
    );

    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 901, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesReadEvent { registry_adapter_id: 0, short_address: 9, last_chunk: true, chunk: DaliAttributeReadChunk::Groups { membership: Some(1 << 3) } }),
    );
    wait_until(
        || {
            store
                .physical_device_view(0, 9)
                .map(|pd| pd.attributes.groups.membership.is_some())
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 902, BUS_TID, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 4, physical_short_address: 9 })),
        ),
        PublishResult::Queued,
    );
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[4].applied[3])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
    drain_events(&ev_obs);

    let matrix = store.group_membership_matrix_view(0).expect("group matrix");
    assert!(
        matrix.rows[4].desired[3],
        "membership read before binding must be adopted as desired"
    );
    let group = store.group_view(0, 3).expect("group view");
    assert!(
        !group.dirty,
        "a lamp whose groups were merely READ must not report dirty: {group:?}"
    );
}

fn replaced_event(correlation_id: u64, failed: u8, replacement: u8) -> dali2rust_contracts::msg::EventEnvelope {
    dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDeviceReplacedEvent { registry_adapter_id: 0, failed_short_address: failed, replacement_short_address: replacement, restored_metadata_and_overrides: true, operation_key: dali2rust_contracts::msg::FixedText32::new(), error: None })
}

fn apply_row(store: &RegistryStore, virtual_lamp_id: u8) -> dali2rust_domain::registry::GroupApplyRowView {
    store
        .group_apply_snapshot(0)
        .expect("group apply snapshot")
        .rows
        .into_iter()
        .find(|row| row.virtual_lamp_id == virtual_lamp_id)
        .expect("apply row")
}

const CHANGED_EVENTS_DEADLINE: Duration = Duration::from_secs(2);
const CHANGED_EVENTS_QUIET: Duration = Duration::from_millis(300);

type Changed = (Vec<u8>, Vec<u8>, usize);

fn note_changed(changed: &mut Changed, payload: &BusEventPayload) {
    match payload {
        BusEventPayload::PhysicalDeviceChangedEvent(body) => changed.0.push(body.short_address),
        BusEventPayload::VirtualLampChangedEvent(body) => changed.1.push(body.virtual_lamp_id),
        BusEventPayload::GroupMatrixChangedEvent(_) => changed.2 += 1,
        _ => {}
    }
}

fn changed_events_for(
    rx: &std::sync::mpsc::Receiver<BusFrame>,
    correlation_id: u64,
    expected: &Changed,
) -> Changed {
    let counts = |c: &Changed| (c.0.len(), c.1.len(), c.2);
    let mut changed = Changed::default();
    let mut until = std::time::Instant::now() + CHANGED_EVENTS_DEADLINE;
    loop {
        let left = until.saturating_duration_since(std::time::Instant::now());
        let Ok(BusFrame::Event(ev)) = rx.recv_timeout(left) else {
            break;
        };
        if ev.meta.correlation_id == correlation_id {
            note_changed(&mut changed, &ev.payload);
        }
        if counts(&changed) == counts(expected) {
            until = until.min(std::time::Instant::now() + CHANGED_EVENTS_QUIET);
        }
    }
    changed.0.sort_unstable();
    changed.1.sort_unstable();
    changed
}

#[test]
fn a_replacement_keeps_its_own_evidence_so_the_lamps_rows_turn_dirty() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    seed_device_reporting_groups(&publisher, &store, 950, 0, 1 << 3);
    seed_device_reporting_groups(&publisher, &store, 951, 11, 1 << 7);
    bind_lamp(&publisher, &store, 953, 2, 0);
    drain_events(&ev_obs);

    publish_event(&publisher, replaced_event(952, 0, 11));
    wait_until(
        || store.physical_device_view(0, 11).is_none(),
        Duration::from_millis(500),
    );

    let survivor = store
        .physical_device_view(0, 0)
        .expect("the failed device's role address must survive the handover");
    assert_eq!(
        survivor.attributes.groups.membership.map(|observed| observed.value),
        Some(1 << 7),
        "the record at the role address describes the gear that now answers there"
    );
    let row = apply_row(&store, 2);
    assert_eq!(row.binding_short, Some(0));
    assert_eq!(row.desired_groups_mask, 1 << 3, "the lamp keeps the failed device's role");
    assert_eq!(row.applied_groups_mask, 1 << 7, "applied is what the replacement holds");
    let expected: Changed = (vec![0, 11], vec![2], 1);
    assert_eq!(changed_events_for(&ev_obs, 952, &expected), expected);
}

fn scene_programmed(
    correlation_id: u64,
    short: u8,
    target_state: Option<dali2rust_contracts::msg::DaliSceneTargetState>,
) -> dali2rust_contracts::msg::EventEnvelope {
    let programmed = dali2rust_contracts::msg::DaliSceneProgrammedEvent {
        registry_adapter_id: 0,
        target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 1 },
        scene_id: 3,
        action: dali2rust_contracts::msg::SceneProgramAction::Write,
        physical_short_address: Some(short),
        target_state,
        scene_level: Some(179),
        error: None,
    };
    let origin = Some(dali2rust_contracts::msg::Origin::Internal);
    dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BUS_TID,
        origin,
        programmed,
    )
}

#[test]
fn what_was_programmed_into_the_old_gear_does_not_vouch_for_the_new_one() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    setup_discovered_short_and_binding(&publisher, &store, 7, 1, 990);
    seed_physical_via_discovery(&publisher, &store, 8);
    let target = dali2rust_contracts::msg::DaliSceneTargetState {
        power: Some(dali2rust_contracts::msg::PowerState::On),
        level: Some(179),
        color: Some(dali2rust_contracts::msg::ColorValue {
            mode: dali2rust_contracts::msg::ColorMode::Cct,
            color_temperature_kelvin: 2700,
            ..Default::default()
        }),
    };
    let mut rows = dali2rust_contracts::msg::SceneMatrixDesiredRowList::new();
    let row = dali2rust_contracts::msg::SceneMatrixDesiredRow {
        virtual_lamp_id: 1,
        included: true,
        target: Some(target),
    };
    rows.push(row).expect("row capacity");
    publish_scene_matrix_write(&publisher, 992, 0, 3, rows);
    publish_event(&publisher, scene_programmed(993, 7, Some(target)));
    publish_event(&publisher, scene_programmed(994, 8, None));
    let scene_dirty = || store.scene_view(0, 3).map(|scene| scene.dirty);
    wait_until(|| scene_dirty() == Some(false), Duration::from_millis(500));
    assert_eq!(scene_dirty(), Some(false), "the old gear holds what the lamp's row asks");
    drain_events(&ev_obs);

    publish_event(&publisher, replaced_event(995, 7, 8));
    wait_until(|| store.physical_device_view(0, 8).is_none(), Duration::from_millis(500));

    assert_eq!(
        scene_dirty(),
        Some(true),
        "the new gear holds the level, but nothing says it holds the colour the row asks"
    );
}

const DT6_ONLY: u64 = 1 << 6;
const DT8_ONLY: u64 = 1 << 8;
const DT6_AND_DT8: u64 = DT6_ONLY | DT8_ONLY;

fn discovered_gear(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    short: u8,
    declared: u64,
) {
    let declared = dali2rust_contracts::msg::DeviceTypeSet::from_bits(declared);
    let colour = DeviceType::Dt8Color.dali_code().is_some_and(|dt8| declared.contains(dt8));
    publish_discovery(publisher, store, dali2rust_contracts::msg::DaliDiscoveryProgressEvent {
        registry_adapter_id: 0,
        short_address: short,
        random_address: None,
        device_type: if colour { DeviceType::Dt8Color } else { DeviceType::Dt6Led },
        color_mode: ColorMode::Unknown,
        dt8_xy_capable: false,
        dt8_tc_capable: colour,
        dt8_rgb_capable: false,
        dt8_rgbwaf_capable: false,
        supported_device_types: Some(declared),
    });
}

fn give_overrides(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    (correlation_id, short): (u64, u8),
    (device_type, colour): (Option<DeviceType>, ColorMode),
) {
    let types = device_type.map_or(0, |_| PdPatch::PATCH_DEVICE_TYPE_OVERRIDE);
    let patch = PdPatch {
        adapter_id: 0,
        short_address: short,
        patch_mask: types | PdPatch::PATCH_COLOR_MODE_OVERRIDE,
        name: dali2rust_contracts::msg::FixedText64::new(),
        clear_device_type_override: false,
        device_type_override: device_type.unwrap_or(DeviceType::Unknown),
        clear_color_mode_override: false,
        color_mode_override: colour,
        dt8_auto_activation_repair: true,
        dt8_rgbwaf_control_assert: true,
    };
    let origin = Some(dali2rust_contracts::msg::Origin::Api);
    let command = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BUS_TID,
        origin,
        patch,
    );
    support::publish_cmd(publisher, command);
    let given = || {
        store.physical_device_view(0, short).is_some_and(|pd| pd.color_mode_override.is_some())
    };
    wait_until(given, Duration::from_millis(500));
}

type Overrides = (Option<String>, Option<String>);

fn hand_over(declared_by_new_gear: u64, own_colour: Option<ColorMode>) -> Overrides {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev_obs, _host) =
        spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    discovered_gear(&publisher, &store, 0, DT8_ONLY);
    discovered_gear(&publisher, &store, 11, declared_by_new_gear);
    let role_overrides = (Some(DeviceType::Dt8Color), ColorMode::Xy);
    give_overrides(&publisher, &store, (1002, 0), role_overrides);
    if let Some(colour) = own_colour {
        give_overrides(&publisher, &store, (1003, 11), (None, colour));
    }
    publish_event(&publisher, replaced_event(1004, 0, 11));
    wait_until(|| store.physical_device_view(0, 11).is_none(), Duration::from_millis(500));
    let record = store.physical_device_view(0, 0).expect("the role address");
    (record.device_type_override, record.color_mode_override)
}

#[test]
fn a_replacement_takes_only_the_overrides_the_new_gear_declares() {
    let declared_both: Overrides = (Some("dt8_color".into()), Some("xy".into()));
    let colour_gear = hand_over(DT6_AND_DT8, None);
    assert_eq!(colour_gear, declared_both, "xy needs DT8, not an xy-capable flag");
    assert_eq!(hand_over(DT6_ONLY, None), (None, None), "a DT6 dimmer is no DT8 colour gear");
    let own: Overrides = (None, Some("brightness".into()));
    let dimmer_with_its_own = hand_over(DT6_ONLY, Some(ColorMode::Brightness));
    assert_eq!(dimmer_with_its_own, own, "the new gear keeps its own");
}

#[test]
fn a_replacement_never_leaves_two_lamps_on_one_device() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    seed_device_reporting_groups(&publisher, &store, 960, 0, 1 << 3);
    seed_device_reporting_groups(&publisher, &store, 961, 11, 1 << 7);
    bind_lamp(&publisher, &store, 962, 2, 0);
    bind_lamp(&publisher, &store, 963, 5, 11);
    drain_events(&ev_obs);

    publish_event(&publisher, replaced_event(964, 0, 11));
    wait_until(
        || store.virtual_lamp_view(0, 5).binding_short.is_none(),
        Duration::from_millis(500),
    );

    assert_eq!(store.virtual_lamp_view(0, 2).binding_short, Some(0), "the role keeps its lamp");
    assert_eq!(
        store.virtual_lamp_view(0, 5).binding_short,
        None,
        "the replacement's own lamp loses its device, as forget would leave it"
    );
    let expected: Changed = (vec![0, 11], vec![2, 5], 1);
    assert_eq!(changed_events_for(&ev_obs, 964, &expected), expected);
}

#[test]
fn a_replacement_without_a_lamp_at_the_role_brings_its_own_lamp_along() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    seed_device_reporting_groups(&publisher, &store, 970, 0, 1 << 3);
    seed_device_reporting_groups(&publisher, &store, 971, 11, 1 << 7);
    bind_lamp(&publisher, &store, 972, 5, 11);
    drain_events(&ev_obs);

    publish_event(&publisher, replaced_event(973, 0, 11));
    wait_until(
        || store.virtual_lamp_view(0, 5).binding_short == Some(0),
        Duration::from_millis(500),
    );

    let row = apply_row(&store, 5);
    assert_eq!(row.binding_short, Some(0), "the lamp follows its gear to the role address");
    assert_eq!(row.applied_groups_mask, 1 << 7);
    let expected: Changed = (vec![0, 11], vec![5], 0);
    assert_eq!(changed_events_for(&ev_obs, 973, &expected), expected);
}

#[test]
fn an_address_change_announces_both_addresses_and_the_moved_lamp() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    seed_device_reporting_groups(&publisher, &store, 980, 4, 1 << 3);
    bind_lamp(&publisher, &store, 981, 6, 4);
    drain_events(&ev_obs);

    publish_event(
        &publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 982, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAddressingCompletedEvent { registry_adapter_id: 0, old_short_address: 4, new_short_address: 9, operation_key: dali2rust_contracts::msg::FixedText32::new(), error: None }),
    );
    wait_until(
        || store.virtual_lamp_view(0, 6).binding_short == Some(9),
        Duration::from_millis(500),
    );

    let expected: Changed = (vec![4, 9], vec![6], 0);
    assert_eq!(changed_events_for(&ev_obs, 982, &expected), expected);
}

fn seed_device_reporting_groups(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    corr_base: u64,
    short: u8,
    membership: u16,
) {
    seed_physical_via_discovery(publisher, store, short);
    publish_event(
        publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, corr_base, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesReadEvent { registry_adapter_id: 0, short_address: short, last_chunk: true, chunk: DaliAttributeReadChunk::Groups { membership: Some(membership) } }),
    );
    wait_until(
        || {
            store
                .physical_device_view(0, short)
                .map(|pd| pd.attributes.groups.membership.is_some())
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
}

fn bind_lamp(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    correlation_id: u64,
    virtual_lamp_id: u8,
    short: u8,
) {
    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, BUS_TID, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id, physical_short_address: short })),
        ),
        PublishResult::Queued,
    );
    wait_until(
        || store.virtual_lamp_view(0, virtual_lamp_id).binding_short == Some(short),
        Duration::from_millis(500),
    );
}

fn unbind_lamp(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    correlation_id: u64,
    virtual_lamp_id: u8,
) {
    assert_eq!(
        publisher.try_publish(
            BusChannel::Commands,
            BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, BUS_TID, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampUnbindCommand { adapter_id: 0, virtual_lamp_id })),
        ),
        PublishResult::Queued,
    );
    wait_until(
        || store.virtual_lamp_view(0, virtual_lamp_id).binding_short.is_none(),
        Duration::from_millis(500),
    );
}

fn apply_row_desired_mask(store: &RegistryStore, virtual_lamp_id: u8) -> u16 {
    apply_row(store, virtual_lamp_id).desired_groups_mask
}

#[test]
fn unbinding_then_binding_another_device_does_not_inherit_the_old_devices_groups() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_device_reporting_groups(&publisher, &store, 940, 10, 1 << 3);
    seed_device_reporting_groups(&publisher, &store, 941, 11, 1 << 5);

    bind_lamp(&publisher, &store, 942, 4, 10);
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[4].desired[3])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    unbind_lamp(&publisher, &store, 943, 4);
    bind_lamp(&publisher, &store, 944, 4, 11);
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[4].applied[5])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    let matrix = store.group_membership_matrix_view(0).expect("group matrix");
    assert!(
        !matrix.rows[4].desired[3],
        "the old device's group must not follow the lamp onto the new one"
    );
    assert!(
        matrix.rows[4].desired[5],
        "the new device's own membership is adopted, exactly as on a first bind"
    );
    assert_eq!(
        apply_row_desired_mask(&store, 4),
        1 << 5,
        "an apply must not program the new device into the old device's groups"
    );
    for group_id in [3u8, 5] {
        let group = store.group_view(0, group_id).expect("group view");
        assert!(
            !group.dirty,
            "group {group_id} reports a dirty nobody configured: {group:?}"
        );
    }
}

#[test]
fn unbinding_then_binding_the_same_device_leaves_the_adopted_groups_alone() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_device_reporting_groups(&publisher, &store, 950, 10, 1 << 3);

    bind_lamp(&publisher, &store, 951, 4, 10);
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[4].desired[3])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    unbind_lamp(&publisher, &store, 952, 4);
    bind_lamp(&publisher, &store, 953, 4, 10);
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[4].applied[3])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    let matrix = store.group_membership_matrix_view(0).expect("group matrix");
    assert!(matrix.rows[4].desired[3], "the same device's group is adopted again");
    assert_eq!(apply_row_desired_mask(&store, 4), 1 << 3);
    let group = store.group_view(0, 3).expect("group view");
    assert!(!group.dirty, "rebinding to the same gear changes nothing: {group:?}");
}

#[test]
fn an_operator_authored_mask_survives_unbinding_and_moving_the_lamp() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_device_reporting_groups(&publisher, &store, 960, 10, 1 << 3);
    seed_device_reporting_groups(&publisher, &store, 961, 11, 1 << 5);
    bind_lamp(&publisher, &store, 962, 4, 10);

    publish_group_matrix_write(&publisher, 963, 0, group_rows(&[GroupMatrixDesiredRow {
            virtual_lamp_id: 4,
            desired_groups_mask: 1 << 7,
        }]));
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[4].desired[7])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    unbind_lamp(&publisher, &store, 964, 4);
    let unbound = store.group_membership_matrix_view(0).expect("group matrix");
    assert!(
        unbound.rows[4].desired[7],
        "unbinding says nothing about which groups the lamp belongs in"
    );

    bind_lamp(&publisher, &store, 965, 4, 11);
    wait_until(
        || {
            store
                .group_membership_matrix_view(0)
                .map(|view| view.rows[4].applied[5])
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
    let matrix = store.group_membership_matrix_view(0).expect("group matrix");
    assert!(matrix.rows[4].desired[7], "the operator's intent follows the lamp");
    assert!(
        !matrix.rows[4].desired[5],
        "and the new device's membership must not overwrite it"
    );
    assert_eq!(apply_row_desired_mask(&store, 4), 1 << 7);
}

#[test]
fn a_part251_read_that_starts_above_zero_commits_under_the_right_fields() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));
    seed_physical_via_discovery(&publisher, &store, 8);
    drain_events(&ev_obs);

    let start = u16::from(part251::CONTENT_FORMAT_ID_OFFSET);
    let mut data = vec![0u8; 0x24 - usize::from(start)];
    let put = |d: &mut Vec<u8>, off: u8, bytes: &[u8]| {
        let at = usize::from(off) - usize::from(start);
        d[at..at + bytes.len()].copy_from_slice(bytes);
    };
    put(&mut data, part251::CONTENT_FORMAT_ID_OFFSET, &[0x00, 0x03]);
    put(&mut data, part251::YEAR_OFFSET, &[24]);
    put(&mut data, part251::WEEK_OFFSET, &[35]);
    put(&mut data, part251::NOMINAL_INPUT_POWER_OFFSET, &[0x00, 0x2C]);
    put(&mut data, part251::CRI_OFFSET, &[94]);
    put(&mut data, part251::CCT_OFFSET, &[0xFF, 0xFE]);

    for (index, chunk) in data.chunks(24).enumerate() {
        let last = usize::from(start) + (index + 1) * 24 >= 0x24;
        publish_event(
            &publisher,
            dali2rust_contracts::bus::event_envelope(
                SOURCE_ID_UNSPECIFIED,
                7,
                BUS_TID,
                Some(dali2rust_contracts::msg::Origin::Internal),
                dali2rust_contracts::msg::DaliMemoryBankReadEvent {
                    start_offset: start,
                    registry_adapter_id: 0,
                    short_address: 8,
                    bank: 1,
                    chunk_index: index as u16,
                    last_chunk: last,
                    data: dali2rust_contracts::msg::FixedBytes24::from_slice(chunk),
                },
            ),
        );
    }

    wait_until(
        || {
            store
                .physical_device_view(0, 8)
                .is_some_and(|d| d.luminaire_info.is_some())
        },
        Duration::from_secs(2),
    );
    let dev = store.physical_device_view(0, 8).expect("device");
    let lum = dev.luminaire_info.as_deref().expect("luminaire view allocated");
    assert_eq!(lum.content_format_id.as_ref().map(|v| v.value), Some(3));
    assert_eq!(lum.year.as_ref().map(|v| v.value.value), Some(Some(24)));
    assert_eq!(lum.week.as_ref().map(|v| v.value.value), Some(Some(35)));
    assert_eq!(
        lum.nominal_input_power_w.as_ref().map(|v| v.value.value),
        Some(Some(44)),
        "0x15/0x16 landed on the power field, not on whatever chunk_index*24 said"
    );
    assert_eq!(lum.cri.as_ref().map(|v| v.value.value), Some(Some(94)));
    let cct = lum.cct_kelvin.as_ref().expect("cct").value;
    assert!(cct.part209_implemented, "MASK - 1 is Part 209, not TMASK");
    assert_eq!(cct.value, None);
    assert!(dev.attributes.memory_profile.oem_gtin.is_none());
}


struct DefaultedGroupPort<'a>(&'a RegistryStore);

impl AdapterReadPort for DefaultedGroupPort<'_> {
    fn adapter_count(&self) -> u8 {
        self.0.adapter_count()
    }
    fn adapter_view(&self, adapter_id: u8) -> Option<AdapterView> {
        self.0.adapter_view(adapter_id)
    }
    fn list_adapter_views(&self) -> Vec<AdapterView> {
        self.0.list_adapter_views()
    }
}

impl GroupReadPort for DefaultedGroupPort<'_> {
    fn group_view(
        &self,
        adapter_id: u8,
        group_id: u8,
    ) -> Option<GroupView> {
        self.0.group_view(adapter_id, group_id)
    }
    fn list_group_views(&self, adapter_id: u8) -> Vec<GroupView> {
        self.0.list_group_views(adapter_id)
    }
    fn group_membership_matrix_view(
        &self,
        adapter_id: u8,
    ) -> Option<GroupMembershipMatrixView> {
        self.0.group_membership_matrix_view(adapter_id)
    }
    fn group_apply_snapshot(
        &self,
        adapter_id: u8,
    ) -> Option<GroupApplySnapshot> {
        self.0.group_apply_snapshot(adapter_id)
    }
}

#[test]
fn the_stores_member_mask_agrees_with_the_trait_default() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev_obs, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    for (corr, short, lamp, groups) in [
        (1_400u64, 11u8, 2u8, (1u16 << 0) | (1 << 3) | (1 << 15)),
        (1_410, 12, 37, 1u16 << 3),
        (1_420, 13, 9, 0u16),
    ] {
        seed_device_reporting_groups(&publisher, &store, corr, short, groups);
        bind_lamp(&publisher, &store, corr + 1, lamp, short);
    }

    let defaulted = DefaultedGroupPort(&store);
    let mut seen_any_member = false;
    for group_id in 0..=u8::MAX {
        let want = defaulted.applied_group_member_mask(0, group_id);
        assert_eq!(
            store.applied_group_member_mask(0, group_id),
            want,
            "group {group_id}"
        );
        seen_any_member |= want.is_some_and(|mask| mask != 0);
    }
    assert!(
        seen_any_member,
        "the seeding produced no members at all — this test would pass on two \
         implementations that both answer zero"
    );

    assert_eq!(store.applied_group_member_mask(7, 3), None);
    assert_eq!(defaulted.applied_group_member_mask(7, 3), None);
}

#[test]
fn a_starting_scan_blanks_presence_so_a_removed_panel_stops_reading_present() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    publish_event(&publisher, dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED, 1, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::Dali103ScanProgressEvent {
            registry_adapter_id: 0,
            short_address: 7,
            presence_unproven: false,
            instance_count: 2,
            device_capabilities: Some(0b10),
            device_status: Some(0b1000),
            version_number: Some(8),
        }));
    wait_until(
        || store.input_device_detail(0, 7).is_some_and(|d| d.summary.present),
        Duration::from_millis(500),
    );
    let seen = store.input_device_detail(0, 7).expect("record").summary;
    assert!(seen.present && seen.last_seen_ms.is_some(), "the scan must record presence");

    publish_event(&publisher, dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED, 2, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::Dali103ScanStartedEvent { registry_adapter_id: 0 }));
    wait_until(
        || store.input_device_detail(0, 7).is_some_and(|d| !d.summary.present),
        Duration::from_millis(500),
    );
    let after = store.input_device_detail(0, 7).expect("record still there").summary;
    assert!(!after.present, "a starting scan withdraws the claim it is about to re-test");
    assert!(
        after.last_seen_ms.is_none(),
        "last_seen must go with it: present=false plus a stale last_seen reads as \"probed and \
         silent\", which is a diagnosis nobody made"
    );
    assert!(
        counters.events.input_presence_cleared.load(Ordering::Relaxed) >= 1,
        "the clear is counted, not silent"
    );
}

#[test]
fn instance_facts_reach_the_read_model_and_only_a_write_stamps_settling() {
    let store = Arc::new(RegistryStore::new());
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    publish_event(&publisher, dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED, 1, BUS_TID, Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::Dali103ScanProgressEvent {
            registry_adapter_id: 0,
            short_address: 9,
            presence_unproven: false,
            instance_count: 1,
            device_capabilities: Some(0b10),
            device_status: Some(0),
            version_number: Some(8),
        }));
    wait_until(
        || store.input_device_detail(0, 9).is_some_and(|d| d.summary.present),
        Duration::from_millis(500),
    );

    let probe = dali2rust_contracts::msg::Dali103InstanceConfiguredEvent {
        instance_type: Some(4),
        instance_status: Some(0b10),
        resolution: Some(8),
        instance_status_written: false,
        ..configured_base(9, 0)
    };
    publish_event(&publisher, dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED, 2, BUS_TID, Some(dali2rust_contracts::msg::Origin::Api), probe));
    wait_until(
        || store.input_device_detail(0, 9)
            .is_some_and(|d| d.instances.first().is_some_and(|i| i.instance_status == Some(0b10))),
        Duration::from_millis(500),
    );

    let detail = store.input_device_detail(0, 9).expect("record");
    let inst = detail.instances.first().expect("instance");
    assert_eq!(inst.instance_type, Some(4), "the scan's per-instance event carries the type");
    assert_eq!(inst.instance_status, Some(0b10), "Table 16 status must land in the status field");
    assert_eq!(inst.resolution, Some(8), "resolution must land in the resolution field");
    assert_eq!(
        detail.nvm_settling_until_ms, None,
        "a scan PROBE reads and writes nothing, so it owes no settling window — \
         stamping here would put the banner on every instance of every scan"
    );

    let write = dali2rust_contracts::msg::Dali103InstanceConfiguredEvent {
        instance_status: Some(0),
        instance_status_written: true,
        ..configured_base(9, 0)
    };
    publish_event(&publisher, dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED, 3, BUS_TID, Some(dali2rust_contracts::msg::Origin::Api), write));
    wait_until(
        || store.input_device_detail(0, 9).is_some_and(|d| d.nvm_settling_until_ms.is_some()),
        Duration::from_millis(500),
    );

    let detail = store.input_device_detail(0, 9).expect("record");
    assert!(
        detail.nvm_settling_until_ms.is_some(),
        "`instanceActive` is NVM (103 Table 18), so a proved write owes the \
         operator the settling window — without it a mains cycle inside 30 s \
         silently reverts the disable"
    );
    assert_eq!(
        detail.instances.first().expect("instance").resolution,
        Some(8),
        "a read-back that did not ask for resolution must not erase it"
    );
}

fn scan_progress(short_address: u8, presence_unproven: bool)
    -> dali2rust_contracts::msg::Dali103ScanProgressEvent
{
    dali2rust_contracts::msg::Dali103ScanProgressEvent {
        registry_adapter_id: 0,
        short_address,
        presence_unproven,
        instance_count: 1,
        device_capabilities: Some(0b10),
        device_status: Some(0b1000),
        version_number: Some(8),
    }
}

fn publish_scan(
    publisher: &dali2rust_bus::BusPublisher,
    progress: dali2rust_contracts::msg::Dali103ScanProgressEvent,
) {
    publish_event(publisher, dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED, 1, BUS_TID,
        Some(dali2rust_contracts::msg::Origin::Internal), progress));
}

#[test]
fn a_presence_that_was_never_readable_earns_no_home_assistant_entity() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    publish_scan(&publisher, scan_progress(11, true));
    wait_until(
        || store.input_device_detail(0, 11).is_some(),
        Duration::from_millis(500),
    );

    let detail = store.input_device_detail(0, 11).expect("the address is still published");
    assert!(
        !detail.summary.ha_expose,
        "an address whose only evidence is a frame nobody could read must not \
         be offered to Home Assistant — that is a button in the owner's house \
         for a device that does not exist"
    );
}

#[test]
fn a_presence_that_was_read_is_exposed_as_it_always_was() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    publish_scan(&publisher, scan_progress(12, false));
    wait_until(
        || store.input_device_detail(0, 12).is_some(),
        Duration::from_millis(500),
    );

    assert!(
        store.input_device_detail(0, 12).expect("record").summary.ha_expose,
        "a panel that answered readably is a panel"
    );
}

#[test]
fn a_later_unreadable_scan_does_not_revert_a_device_that_already_exists() {
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let counters = Arc::new(RegistryWorkerCounters::default());
    let (publisher, _ev, _host) = spawn_registry_stack(Arc::clone(&store), Arc::clone(&counters));

    publish_scan(&publisher, scan_progress(13, false));
    wait_until(
        || store.input_device_detail(0, 13).is_some(),
        Duration::from_millis(500),
    );

    let mut second = scan_progress(13, true);
    second.device_status = Some(0);
    publish_scan(&publisher, second);
    wait_until(
        || store.input_device_detail(0, 13).is_some_and(|d| d.summary.device_status == Some(0)),
        Duration::from_millis(500),
    );
    assert_eq!(
        store.input_device_detail(0, 13).expect("record").summary.device_status,
        Some(0),
        "the second scan must actually have been applied before anything is \
         concluded from it"
    );

    assert!(
        store.input_device_detail(0, 13).expect("record").summary.ha_expose,
        "a re-scan may not silently un-expose a device an operator is already \
         using — the flag applies when the record is CREATED and never after"
    );
}

fn configured_base(short_address: u8, instance_number: u8) -> dali2rust_contracts::msg::Dali103InstanceConfiguredEvent {
    dali2rust_contracts::msg::Dali103InstanceConfiguredEvent {
        registry_adapter_id: 0,
        short_address,
        instance_number,
        event_scheme: None,
        event_filter: None,
        event_priority: None,
        instance_groups: [None; 3],
        timers: [None; 4],
        manual_config_active: None,
        feedback_opcode_map: None,
        feedback_capability: None,
        feedback_colour_capability: None,
        feedback_timing: None,
        feedback_active_brightness: None,
        feedback_active_colour: None,
        feedback_inactive_brightness: None,
        feedback_inactive_colour: None,
        instance_status: None,
        resolution: None,
        instance_status_written: false,
        instance_type: None,
    }
}
