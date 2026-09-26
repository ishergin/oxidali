mod support;

use dali2rust_contracts::msg::{AdapterSettingsUpdateCommand};
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_contracts::msg::{ColorMode, DaliAttributeGroup, DeviceType};
use dali2rust_domain::registry::{
    AdapterReadPort, PollerSettingsReadPort, RegistryReadPort,
    VirtualLampReadPort,
};
use dali2rust_test_support::{temp_slice_store, try_recv_event_matching_envelope, wait_until};

const CONFIRMATION_DEADLINE: Duration = Duration::from_secs(10);
use dali2rust_registry_runtime::{spawn_registry_worker, RegistryStore, RegistryWorkerCounters};
use dali2rust_contracts::msg::PhysicalDeviceOverrideCommand as PdPatch;

const PD_PATCH_ALL: u8 = PdPatch::PATCH_NAME
    | PdPatch::PATCH_DEVICE_TYPE_OVERRIDE
    | PdPatch::PATCH_COLOR_MODE_OVERRIDE;

fn spawn_registry(store: Arc<RegistryStore>) -> (BusPublisher, BusHost) {
    let (host, publisher, registry_rx) = BusHost::spawn(
        BusConfig::default(),
        |reg| reg.subscribe_commands_and_events(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
    );
    let counters = Arc::new(RegistryWorkerCounters::default());
    let _worker = spawn_registry_worker(
        registry_rx,
        publisher.clone(),
        BusId::default(),
        1,
        Arc::clone(&store),
        counters,
        None,
        std::sync::Arc::new(dali2rust_platform::liveness::LivenessBeat::new("test", 60_000)),
    );
    (publisher, host)
}

fn publish(publisher: &BusPublisher, frame: BusFrame) {
    let channel = match &frame {
        BusFrame::Command(_) => BusChannel::Commands,
        BusFrame::Event(_) => BusChannel::Events,
        _ => panic!("unsupported frame kind in test helper"),
    };
    assert_eq!(publisher.try_publish(channel, frame), PublishResult::Queued);
}

#[test]
fn runtime_level_stays_volatile_across_fs_reload() {
    let slices = temp_slice_store("runtime-vs-persistent");
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let (publisher, _host) = spawn_registry(Arc::clone(&store));
    publish(
        &publisher,
        BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 1, BusId::default().0, None, dali2rust_contracts::msg::VirtualLampConfigUpdateCommand { adapter_id: 0, virtual_lamp_id: 12, patch_mask: dali2rust_contracts::msg::VirtualLampConfigUpdateCommand::PATCH_NAME, name: dali2rust_contracts::msg::fixed_text_64("Kitchen"), ha_entity_enabled: false })),
    );
    wait_until(
        || store.virtual_lamp_snapshot(0, 12).name == "Kitchen",
        Duration::from_millis(500),
    );
    store.flush_dirty_slices(&slices);
    publish(
        &publisher,
        BusFrame::command({ let __corr = 2; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::sniffer_level(12, 180, 12_345))) }),
    );
    store.flush_dirty_slices(&slices);

    let reloaded = RegistryStore::with_adapter_count(1);
    let _ = reloaded.hydrate_from_store(&slices, 1);
    let snap = reloaded.virtual_lamp_snapshot(0, 12);
    assert_eq!(snap.name, "Kitchen");
    assert_eq!(snap.runtime_level, 0);
}

#[test]
fn adapter_settings_round_trip_across_fs_reload() {
    let slices = temp_slice_store("adapter-settings");
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let (publisher, _host) = spawn_registry(Arc::clone(&store));
    publish(
        &publisher,
        BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 10, 0, None, dali2rust_contracts::msg::AdapterSettingsUpdateCommand { patch_mask: AdapterSettingsUpdateCommand::PATCH_NAME | AdapterSettingsUpdateCommand::PATCH_ENABLED, name: dali2rust_contracts::msg::fixed_text_64("Adapter-A"), enabled: false })),
    );
    wait_until(
        || {
            let view = store.adapter_view(0).expect("adapter");
            view.name == "Adapter-A" && !view.enabled
        },
        Duration::from_millis(500),
    );
    let live = store.adapter_view(0).expect("adapter");
    assert_eq!(live.name, "Adapter-A");
    assert!(!live.enabled);
    store.flush_dirty_slices(&slices);

    let reloaded = RegistryStore::with_adapter_count(1);
    let _ = reloaded.hydrate_from_store(&slices, 1);
    let view = reloaded.adapter_view(0).expect("adapter");
    assert_eq!(view.name, "Adapter-A");
    assert!(!view.enabled);
}

#[test]
fn virtual_lamp_binding_round_trip_across_fs_reload() {
    let slices = temp_slice_store("vl-binding");
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let (publisher, _host) = spawn_registry(Arc::clone(&store));
    publish(
        &publisher,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 20, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: 7, random_address: None, device_type: DeviceType::Dt6Led, color_mode: ColorMode::Unknown, dt8_xy_capable: false, dt8_tc_capable: false, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None })),
    );
    wait_until(
        || store.physical_device_view(0, 7).is_some(),
        Duration::from_millis(500),
    );
    publish(
        &publisher,
        BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 21, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 12, physical_short_address: 7 })),
    );
    wait_until(
        || store.virtual_lamp_view(0, 12).binding_short == Some(7),
        Duration::from_millis(500),
    );
    store.flush_dirty_slices(&slices);

    let reloaded = RegistryStore::with_adapter_count(1);
    let _ = reloaded.hydrate_from_store(&slices, 1);
    assert_eq!(reloaded.virtual_lamp_view(0, 12).binding_short, Some(7));
}

#[test]
fn physical_device_override_round_trip_across_fs_reload() {
    let slices = temp_slice_store("pd-override");
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let (publisher, _host) = spawn_registry(Arc::clone(&store));
    publish(
        &publisher,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 30, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: 8, random_address: None, device_type: DeviceType::Dt6Led, color_mode: ColorMode::Unknown, dt8_xy_capable: false, dt8_tc_capable: false, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None })),
    );
    wait_until(
        || store.physical_device_view(0, 8).is_some(),
        Duration::from_millis(500),
    );
    publish(
        &publisher,
        BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 31, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceOverrideCommand { adapter_id: 0, short_address: 8, patch_mask: PD_PATCH_ALL, name: dali2rust_contracts::msg::fixed_text_64("PD-8"), clear_device_type_override: false, device_type_override: DeviceType::Dt8Color, clear_color_mode_override: false, color_mode_override: ColorMode::Rgb , dt8_auto_activation_repair: true, dt8_rgbwaf_control_assert: true})),
    );
    publish(
        &publisher,
        BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 32, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceNotesUpdateCommand { adapter_id: 0, short_address: 8, notes: dali2rust_contracts::msg::fixed_text_48("notes") })),
    );
    wait_until(
        || {
            store.physical_device_view(0, 8).is_some_and(|view| {
                view.name == "PD-8"
                    && view.notes.as_deref() == Some("notes")
                    && view.device_type_override.as_deref() == Some("dt8_color")
                    && view.color_mode_override.as_deref() == Some("rgb")
            })
        },
        Duration::from_millis(500),
    );
    store.flush_dirty_slices(&slices);

    let reloaded = RegistryStore::with_adapter_count(1);
    let _ = reloaded.hydrate_from_store(&slices, 1);
    let view = reloaded.physical_device_view(0, 8).expect("pd");
    assert_eq!(view.name, "PD-8");
    assert_eq!(view.notes.as_deref(), Some("notes"));
    assert_eq!(view.device_type_override.as_deref(), Some("dt8_color"));
    assert_eq!(view.color_mode_override.as_deref(), Some("rgb"));
}

#[test]
fn physical_device_discovery_evidence_round_trip_across_fs_reload() {
    let slices = temp_slice_store("pd-discovered");
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let (publisher, _host) = spawn_registry(Arc::clone(&store));
    publish(
        &publisher,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 40, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: 1, random_address: Some(0x00C8_ED31), device_type: DeviceType::Dt8Color, color_mode: ColorMode::Cct, dt8_xy_capable: false, dt8_tc_capable: true, dt8_rgb_capable: true, dt8_rgbwaf_capable: false, supported_device_types: None })),
    );
    wait_until(
        || {
            store.physical_device_view(0, 1).is_some_and(|view| {
                view.device_type_discovered == "dt8_color" && view.capabilities.rgb
            })
        },
        Duration::from_millis(500),
    );
    store.flush_dirty_slices(&slices);

    let reloaded = RegistryStore::with_adapter_count(1);
    let _ = reloaded.hydrate_from_store(&slices, 1);
    let view = reloaded.physical_device_view(0, 1).expect("pd");
    assert_eq!(view.device_type_discovered, "dt8_color");
    assert_eq!(view.device_type_effective, "dt8_color");
    assert_eq!(view.color_mode_discovered, "cct");
    assert!(view.capabilities.cct, "cct capability lost across reload");
    assert!(
        view.capabilities.rgb,
        "rgb capability lost across reload — colour-mode seeding cannot recover it"
    );
    assert!(!view.capabilities.xy, "xy was never observed");
    assert!(view.capabilities.brightness);
}

#[test]
fn physical_device_memory_identity_round_trip_across_fs_reload() {
    const BANK0_BYTES: [u8; 27] = [
        0x1C, 0x00, 0x01, 0x00, 0x9D, 0xAD, 0xA2, 0x1B, 0x43, 0x01, 0x00, 0x26, 0x01,
        0x06, 0xFA, 0x9E, 0x78, 0x7D, 0x9B, 0x01, 0x00, 0x08, 0x08, 0xFF, 0x00, 0x01,
        0x00,
    ];
    const BANK1_BYTES: [u8; 17] = [
        0x10, 0x00, 0x06, 0x58, 0x23, 0x32, 0xA8, 0xFC, 0xFF, 0xF9, 0x00, 0x00, 0x00,
        0xE6, 0xFF, 0x9B, 0x01,
    ];

    let slices = temp_slice_store("pd-memory-profile");
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let (publisher, _host) = spawn_registry(Arc::clone(&store));
    publish(
        &publisher,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 40, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: 9, random_address: Some(0x5C_1D_C2), device_type: DeviceType::Dt8Color, color_mode: ColorMode::Cct, dt8_xy_capable: false, dt8_tc_capable: true, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None })),
    );
    wait_until(
        || store.physical_device_view(0, 9).is_some(),
        Duration::from_millis(500),
    );
    publish(
        &publisher,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 41, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 9, bank: 0, chunk_index: 0, last_chunk: false, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&BANK0_BYTES[..24]) })),
    );
    publish(
        &publisher,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 41, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 9, bank: 0, chunk_index: 1, last_chunk: true, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&BANK0_BYTES[24..]) })),
    );
    publish(
        &publisher,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 42, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { start_offset: 0, registry_adapter_id: 0, short_address: 9, bank: 1, chunk_index: 0, last_chunk: true, data: dali2rust_contracts::msg::FixedBytes24::from_slice(&BANK1_BYTES) })),
    );
    wait_until(
        || {
            store.physical_device_view(0, 9).is_some_and(|view| {
                view.memory_banks.len() == 2
                    && view.attributes.memory_identity.gtin.is_some()
                    && view.attributes.memory_profile.oem_gtin.is_some()
            })
        },
        Duration::from_millis(500),
    );
    store.flush_dirty_slices(&slices);

    let reloaded = RegistryStore::with_adapter_count(1);
    let _ = reloaded.hydrate_from_store(&slices, 1);
    let view = reloaded.physical_device_view(0, 9).expect("pd");
    assert_eq!(view.random_address, Some(0x5C_1D_C2));
    assert_eq!(view.memory_banks.len(), 2);
    assert_eq!(view.memory_banks[0].total_bytes_read, BANK0_BYTES.len() as u16);
    assert_eq!(view.memory_banks[1].total_bytes_read, BANK1_BYTES.len() as u16);
    assert_eq!(
        view.attributes
            .memory_identity
            .gtin
            .as_ref()
            .map(|value| value.value),
        Some(0x009D_ADA2_1B43),
    );
    assert_eq!(
        view.attributes
            .memory_profile
            .oem_gtin
            .as_ref()
            .map(|value| value.value),
        Some(0x5823_32A8_FCFF),
    );
}

#[test]
fn poller_settings_round_trip_across_fs_reload() {
    let slices = temp_slice_store("poller-settings");
    let store = Arc::new(RegistryStore::with_adapter_count(1));
    let (publisher, _host) = spawn_registry(Arc::clone(&store));
    publish(
        &publisher,
        BusFrame::command(dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            1,
            BusId::default().0,
            None,
            dali2rust_contracts::msg::PollerSettingsUpdateCommand {
                patch_mask: dali2rust_contracts::msg::PollerSettingsUpdateCommand::PATCH_ENABLED
                    | dali2rust_contracts::msg::PollerSettingsUpdateCommand::PATCH_INTERVAL_MS,
                enabled: true,
                interval_ms: 60_000,
                attribute_groups_mask: 0,
                include_dt8_color: false,
                include_energy: false,
                include_diagnostics: false,
                skip_unbound_virtual_lamps: false,
            },
        )),
    );
    wait_until(
        || store.poller_settings_view().interval_ms == 60_000,
        Duration::from_millis(500),
    );
    store.flush_dirty_slices(&slices);

    let reloaded = RegistryStore::with_adapter_count(1);
    let _ = reloaded.hydrate_from_store(&slices, 1);
    let view = reloaded.poller_settings_view();
    assert!(view.enabled);
    assert_eq!(view.interval_ms, 60_000);
    assert_eq!(view.attribute_groups_mask, DaliAttributeGroup::RuntimeStatus.mask_bit());
    assert!(view.include_dt8_color);
    assert!(view.skip_unbound_virtual_lamps);
}

fn wait_for_pd_changed(rx: &std::sync::mpsc::Receiver<BusFrame>, corr: u64) {
    let found = try_recv_event_matching_envelope(
        rx,
        CONFIRMATION_DEADLINE,
        |event| {
            event.meta.correlation_id == corr
                && matches!(
                    &event.payload,
                    dali2rust_contracts::msg::BusEventPayload::PhysicalDeviceChangedEvent(_)
                )
        },
    );
    assert!(
        found.is_some(),
        "PhysicalDeviceChangedEvent for correlation {corr}"
    );
}

#[test]
fn unchanged_attribute_read_event_does_not_rewrite_flash() {
    use dali2rust_contracts::msg::{DaliAttributeReadChunk, DaliAttributesReadEvent};
    use std::sync::atomic::Ordering;

    const SHORT: u8 = 7;
    let slices = temp_slice_store("unchanged-attr-read");
    let stack = support::spawn_registry_stack(1, 64);
    support::seed_physical_via_discovery(&stack.publisher, 1, SHORT, DeviceType::Dt8Color, &stack.store);

    let read_event = |corr: u64| {
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            DaliAttributesReadEvent {
                registry_adapter_id: 0,
                short_address: SHORT,
                last_chunk: true,
                chunk: DaliAttributeReadChunk::Common102 {
                    version: None,
                    device_type: None,
                    physical_minimum: None,
                    min_level: None,
                    max_level: None,
                    power_on_level: None,
                    system_failure_level: None,
                    fade_time_ms: Some(700),
                    fade_rate: None,
                    supported_device_types: None,
                    light_source_type: None,
                    light_source_types: None,
                },
            },
        )
    };

    support::publish_event(&stack.publisher, read_event(2));
    wait_for_pd_changed(&stack.ev_rx, 2);
    stack.store.flush_dirty_slices(&slices);
    let flushes_after_first = stack
        .store
        .persistence_counters()
        .flush_success_total
        .load(Ordering::Acquire);
    assert!(flushes_after_first >= 1, "the first read evidence must persist");

    support::publish_event(&stack.publisher, read_event(3));
    wait_for_pd_changed(&stack.ev_rx, 3);
    stack.store.flush_dirty_slices(&slices);
    assert_eq!(
        stack
            .store
            .persistence_counters()
            .flush_success_total
            .load(Ordering::Acquire),
        flushes_after_first,
        "identical evidence re-applied must not rewrite the slice"
    );
}
