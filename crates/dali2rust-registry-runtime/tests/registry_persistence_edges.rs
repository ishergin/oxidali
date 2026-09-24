use dali2rust_contracts::msg::{
    ColorMode, DeviceType, PersistenceSliceKind, PersistenceSliceList,
};
use dali2rust_domain::registry::{
    AttributeSource, GroupReadPort, HomeAssistantSettingsReadPort, MemoryBankRangeView,
    MemoryBankSummaryView, ObservedValue, PhysicalDeviceAttributesView, VirtualLampReadPort,
};
use dali2rust_registry_runtime::{
    encode_persistence_blob,
    PersistableAdapterSlice, PersistablePhysicalDeviceRecord, PersistablePhysicalDevicesSlice,
    PersistableVirtualLampsSlice, PersistableVlRecord, PersistenceEnvelope, RegistryStore,
    ADAPTERS_SLICE_VERSION, PHYSICAL_DEVICES_SLICE_VERSION, VIRTUAL_LAMPS_SLICE_VERSION,
};
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_test_support::{temp_slice_store, write_slice};


fn persisted_blob<T: serde::Serialize>(envelope: &PersistenceEnvelope<T>) -> Vec<u8> {
    encode_persistence_blob(envelope).expect("encode fixture")
}

fn observed<T>(value: T, last_read_ms: u64) -> ObservedValue<T> {
    ObservedValue {
        value,
        source: AttributeSource::Readback,
        last_read_ms: Some(last_read_ms),
        last_write_confirmed_ms: None,
    }
}

fn slice_list(items: &[PersistenceSliceKind]) -> PersistenceSliceList {
    let mut out = PersistenceSliceList::new();
    for item in items {
        out.push(item.clone()).expect("persistence slice list");
    }
    out
}

#[test]
fn hydrate_at_four_adapters_fits_the_slice_list() {
    let slices = temp_slice_store("four-adapters");
    let store = RegistryStore::with_adapter_count(4);
    let r = store.hydrate_from_store(&slices, 4);
    assert_eq!(r.default_slices.iter().count(), 1 + 4 * 35 + 4 + 6);
    assert_eq!(r.errors.iter().count(), 0);
}

#[test]
fn hydrate_empty_fs_uses_defaults_without_errors() {
    let slices = temp_slice_store("empty");
    let store = RegistryStore::with_adapter_count(1);
    let r = store.hydrate_from_store(&slices, 1);
    let mut expected = vec![
        PersistenceSliceKind::Adapters,
        PersistenceSliceKind::Groups { adapter_id: 0 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 0 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 1 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 2 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 3 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 4 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 5 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 6 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 7 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 8 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 9 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 10 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 11 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 12 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 13 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 14 },
        PersistenceSliceKind::PhysicalDeviceBank { adapter_id: 0, bank: 15 },
        PersistenceSliceKind::PhysicalDevices { adapter_id: 0 },
        PersistenceSliceKind::VirtualLamps { adapter_id: 0 },
    ];
    expected.extend((0..16).map(|scene_id| PersistenceSliceKind::Scenes {
        adapter_id: 0,
        scene_id,
    }));
    expected.extend((0..4).map(|bank| PersistenceSliceKind::InputDevices { bank }));
    expected.push(PersistenceSliceKind::HclSchedules);
    expected.push(PersistenceSliceKind::PollerSettings);
    expected.push(PersistenceSliceKind::DaliSettings);
    expected.push(PersistenceSliceKind::RedundancySettings);
    expected.push(PersistenceSliceKind::Policies);
    expected.push(PersistenceSliceKind::HomeAssistantSettings);
    let expected_defaults = slice_list(&expected);
    assert!(r.loaded_slices.is_empty(), "loaded={:?}", r.loaded_slices);
    assert_eq!(r.default_slices, expected_defaults);
    assert!(r.errors.is_empty(), "errors={:?}", r.errors);
    assert!(r.is_ok());
}

#[test]
fn an_unstored_home_assistant_slice_persists_its_derived_default_on_the_first_boot() {
    let slices = temp_slice_store("ha-defaults");
    let store = RegistryStore::with_adapter_count(1);
    store.seed_home_assistant_controller_id([0x3c, 0x84, 0x27, 0xa1, 0xb2, 0xc3]);
    let report = store.hydrate_from_store(&slices, 1);
    assert!(report
        .default_slices
        .contains(&PersistenceSliceKind::HomeAssistantSettings));
    assert!(
        slices.load(SliceKey::HomeAssistantSettings).is_err(),
        "nothing is on flash until the flush runs"
    );

    store.flush_dirty_slices(&slices);

    let reloaded = RegistryStore::with_adapter_count(1);
    let report = reloaded.hydrate_from_store(&slices, 1);
    assert!(report
        .loaded_slices
        .contains(&PersistenceSliceKind::HomeAssistantSettings));
    assert_eq!(
        reloaded.home_assistant_settings_view().controller_id,
        "dali-a1b2c3"
    );
}

#[test]
fn hydrate_malformed_adapters_blob_records_error() {
    let slices = temp_slice_store("bad-adapters");
    let mut blob = vec![ADAPTERS_SLICE_VERSION as u8];
    blob.extend_from_slice(&[0xFF; 8]);
    write_slice(&slices, SliceKey::Adapters, &blob);
    let store = RegistryStore::with_adapter_count(1);
    let r = store.hydrate_from_store(&slices, 1);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].kind, PersistenceSliceKind::Adapters);
    assert!(
        r.errors[0].error.contains("malformed"),
        "got {}",
        r.errors[0].error.as_str()
    );
    assert!(r.default_slices.contains(&PersistenceSliceKind::Adapters));
}

#[test]
fn hydrate_unsupported_adapters_version_records_error() {
    let slices = temp_slice_store("bad-version");
    let blob = persisted_blob(&PersistenceEnvelope {
        version: ADAPTERS_SLICE_VERSION.saturating_add(99),
        data: PersistableAdapterSlice { adapters: vec![] },
    });
    write_slice(&slices, SliceKey::Adapters, &blob);
    let store = RegistryStore::with_adapter_count(1);
    let r = store.hydrate_from_store(&slices, 1);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].kind, PersistenceSliceKind::Adapters);
    assert!(
        r.errors[0]
            .error
            .contains("unsupported persistence version"),
        "got {}",
        r.errors[0].error.as_str()
    );
}

#[test]
fn unreadable_slice_is_marked_for_rewrite_so_the_next_boot_is_clean() {
    let slices = temp_slice_store("stale-version-self-heals");
    let blob = persisted_blob(&PersistenceEnvelope {
        version: ADAPTERS_SLICE_VERSION.saturating_add(99),
        data: PersistableAdapterSlice { adapters: vec![] },
    });
    write_slice(&slices, SliceKey::Adapters, &blob);

    let store = RegistryStore::with_adapter_count(1);
    let first = store.hydrate_from_store(&slices, 1);
    assert_eq!(first.errors.len(), 1, "stale slice must be reported once");

    store.flush_dirty_slices(&slices);

    let second = store.hydrate_from_store(&slices, 1);
    assert!(
        second.errors.is_empty(),
        "slice should read back after the rewrite, got {:?}",
        second.errors
    );
}

#[test]
fn hydrate_malformed_virtual_lamps_blob_records_error() {
    let slices = temp_slice_store("bad-vl");
    write_slice(&slices, SliceKey::VirtualLamps { adapter_id: 0 }, b"[]");
    let store = RegistryStore::with_adapter_count(1);
    let r = store.hydrate_from_store(&slices, 1);
    let vl_errs: Vec<_> = r
        .errors
        .iter()
        .filter(|e| matches!(e.kind, PersistenceSliceKind::VirtualLamps { adapter_id: 0 }))
        .collect();
    assert_eq!(vl_errs.len(), 1, "errors={:?}", r.errors);
}

#[test]
fn flush_dirty_slices_no_op_when_store_clean() {
    let slices = temp_slice_store("clean-flush");
    let store = RegistryStore::with_adapter_count(1);
    store.flush_dirty_slices(&slices);
    assert!(
        !slices.load(SliceKey::Adapters).is_ok(),
        "unexpected adapters file after no-op flush"
    );
}

#[test]
fn hydrate_drops_virtual_lamp_binding_without_physical_peer() {
    let slices = temp_slice_store("vl-orphan-binding");
    let blob = persisted_blob(&PersistenceEnvelope::new(VIRTUAL_LAMPS_SLICE_VERSION, PersistableVirtualLampsSlice {
        adapter_id: 0,
        lamps: vec![PersistableVlRecord {
            virtual_lamp_id: 1,
            name: "L1".to_string(),
            ha_entity_enabled: true,
            binding_short: Some(9),
        }],
    }));
    write_slice(&slices, SliceKey::VirtualLamps { adapter_id: 0 }, &blob);

    let store = RegistryStore::with_adapter_count(1);
    let _r = store.hydrate_from_store(&slices, 1);
    assert_eq!(store.virtual_lamp_view(0, 1).binding_short, None);
}

#[test]
fn hydrate_physical_devices_blob_preserves_attributes_and_random_address() {
    let slices = temp_slice_store("pd-attributes");
    let mut attributes = PhysicalDeviceAttributesView::default();
    attributes.common102.version = Some(observed(8u8, 11));
    attributes.groups.membership = Some(observed(2u16, 22));
    attributes.dt6_led.operating_mode = Some(observed(1u8, 33));
    let memory_banks = vec![MemoryBankSummaryView {
        bank: 0,
        total_bytes_read: 64,
        last_read_ms: 44,
        ranges: vec![MemoryBankRangeView {
            start: 0,
            length: 32,
        }],
    }];
    let blob = persisted_blob(&PersistenceEnvelope::new(PHYSICAL_DEVICES_SLICE_VERSION, PersistablePhysicalDevicesSlice {
        adapter_id: 0,
        devices: vec![PersistablePhysicalDeviceRecord {
            short_address: 3,
            random_address: Some(0x123456),
            name: "PD3".to_string(),
            notes: Some("CCT luminaire".to_string()),
            device_type_override: None,
            color_mode_override: None,
            attributes: attributes.clone(),
            memory_banks: memory_banks.clone(),
            device_type_discovered: DeviceType::Dt8Color,
            color_mode_discovered: ColorMode::Cct,
            cap_cct: true,
            cap_xy: false,
            cap_rgb: false,
            tc_coolest_mirek: Some(153),
            tc_warmest_mirek: Some(370),
            dt8_auto_activation_repair: None,
            dt8_rgbwaf_control_assert: None,
            cap_rgbwaf: false,
                supported_device_types: None,
        }],
    }));
    write_slice(&slices, SliceKey::PhysicalDevices { adapter_id: 0 }, &blob);

    let store = RegistryStore::with_adapter_count(1);
    let _r = store.hydrate_from_store(&slices, 1);
    let view = store
        .physical_device_view(0, 3)
        .expect("physical device should hydrate");
    assert_eq!(view.random_address, Some(0x123456));
    assert_eq!(view.name, "PD3");
    assert_eq!(view.notes.as_deref(), Some("CCT luminaire"));
    assert_eq!(view.attributes, attributes);
    assert_eq!(view.memory_banks, memory_banks);
    assert_eq!(view.device_type_discovered, "dt8_color");
    assert_eq!(view.color_mode_discovered, "cct");
    assert!(view.capabilities.cct, "the fixture's CCT evidence hydrates");
    let range = view
        .color_temperature_range
        .expect("the fixture's own Tc range hydrates");
    assert_eq!(range.min_kelvin, 2702);
    assert_eq!(range.max_kelvin, 6535);
}

#[test]
fn a_stale_physical_devices_slice_does_not_cost_the_poller_its_settings() {
    use dali2rust_domain::registry::PollerSettingsReadPort;
    use dali2rust_registry_runtime::{
        PersistablePollerSettingsSlice, POLLER_SETTINGS_SLICE_VERSION,
    };

    let slices = temp_slice_store("slice-versions-are-independent");
    write_slice(
        &slices,
        SliceKey::PollerSettings,
        &persisted_blob(&PersistenceEnvelope::new(
            POLLER_SETTINGS_SLICE_VERSION,
            PersistablePollerSettingsSlice {
                enabled: true,
                interval_ms: 7_777,
                attribute_groups_mask: 1,
                include_dt8_color: true,
                include_energy: false,
                include_diagnostics: false,
                skip_unbound_virtual_lamps: false,
            },
        )),
    );
    write_slice(
        &slices,
        SliceKey::PhysicalDevices { adapter_id: 0 },
        &persisted_blob(&PersistenceEnvelope::new(
            PHYSICAL_DEVICES_SLICE_VERSION.saturating_add(1),
            PersistablePhysicalDevicesSlice {
                adapter_id: 0,
                devices: vec![],
            },
        )),
    );

    let store = RegistryStore::with_adapter_count(1);
    let report = store.hydrate_from_store(&slices, 1);

    assert!(
        report
            .errors
            .iter()
            .any(|e| matches!(e.kind, PersistenceSliceKind::PhysicalDevices { adapter_id: 0 })),
        "the stale slice should still be reported: {:?}",
        report.errors
    );
    let settings = store.poller_settings_view();
    assert_eq!(settings.interval_ms, 7_777, "poller settings were collateral");
    assert!(settings.enabled);
}

mod slice_versioning {
    use dali2rust_registry_runtime::{
        decode_versioned_slice, encode_persistence_blob, PersistenceEnvelope,
    };
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct RowV1 {
        virtual_lamp_id: u8,
        desired_groups_mask: u16,
        desired_seeded: bool,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct RowV2 {
        virtual_lamp_id: u8,
        desired_groups_mask: u16,
        desired_seeded: bool,
        desired_from_operator: bool,
    }

    fn v1_blob(version: u32) -> Vec<u8> {
        let rows = vec![
            RowV1 { virtual_lamp_id: 1, desired_groups_mask: 0x0102, desired_seeded: true },
            RowV1 { virtual_lamp_id: 2, desired_groups_mask: 0x0304, desired_seeded: false },
        ];
        encode_persistence_blob(&PersistenceEnvelope::new(version, rows)).expect("encode")
    }

    #[test]
    fn a_slice_from_the_previous_shape_is_refused_by_version() {
        let blob = v1_blob(9);
        let err = decode_versioned_slice::<Vec<RowV2>>(&blob, 10)
            .expect_err("version 9 must not be read as version 10");
        let message = err.to_string();
        assert!(
            message.contains("unsupported persistence version: 9"),
            "the version must be what refuses it, not a body parse error: {message}"
        );
    }

    #[test]
    fn the_matching_version_still_round_trips() {
        let rows = vec![RowV2 {
            virtual_lamp_id: 7,
            desired_groups_mask: 0x00F0,
            desired_seeded: true,
            desired_from_operator: true,
        }];
        let blob = encode_persistence_blob(&PersistenceEnvelope::new(10, &rows)).expect("encode");
        let back: Vec<RowV2> = decode_versioned_slice(&blob, 10).expect("same version decodes");
        assert_eq!(back, rows);
    }
}

#[test]
fn a_rejected_physical_devices_slice_orphans_every_binding_at_once() {
    const LAMPS: [(u8, u8); 3] = [(1, 4), (2, 5), (3, 6)];
    let slices = temp_slice_store("pd-bump-orphans-all");

    let vl_blob = persisted_blob(&PersistenceEnvelope::new(
        VIRTUAL_LAMPS_SLICE_VERSION,
        PersistableVirtualLampsSlice {
            adapter_id: 0,
            lamps: LAMPS
                .iter()
                .map(|(vl, short)| PersistableVlRecord {
                    virtual_lamp_id: *vl,
                    name: format!("L{vl}"),
                    ha_entity_enabled: true,
                    binding_short: Some(*short),
                })
                .collect(),
        },
    ));
    write_slice(&slices, SliceKey::VirtualLamps { adapter_id: 0 }, &vl_blob);
    let stale_pd = persisted_blob(&PersistenceEnvelope::new(
        PHYSICAL_DEVICES_SLICE_VERSION - 1,
        PersistablePhysicalDevicesSlice {
            adapter_id: 0,
            devices: LAMPS
                .iter()
                .map(|(_, short)| PersistablePhysicalDeviceRecord {
                    short_address: *short,
                    random_address: None,
                    name: format!("PD{short}"),
                    notes: None,
                    device_type_override: None,
                    color_mode_override: None,
                    attributes: PhysicalDeviceAttributesView::default(),
                    memory_banks: Vec::new(),
                    device_type_discovered: DeviceType::Unknown,
                    color_mode_discovered: ColorMode::Unknown,
                    cap_cct: false,
                    cap_xy: false,
                    cap_rgb: false,
                    tc_coolest_mirek: None,
                    tc_warmest_mirek: None,
                    dt8_auto_activation_repair: None,
                    dt8_rgbwaf_control_assert: None,
                    cap_rgbwaf: false,
                    supported_device_types: None,
                })
                .collect(),
        },
    ));
    write_slice(
        &slices,
        SliceKey::PhysicalDevices { adapter_id: 0 },
        &stale_pd,
    );

    let store = RegistryStore::with_adapter_count(1);
    let _report = store.hydrate_from_store(&slices, 1);

    for (vl, short) in LAMPS {
        assert!(
            store.physical_device_view(0, short).is_none(),
            "the bump must reject the whole slice, not repair it"
        );
        assert_eq!(
            store.virtual_lamp_view(0, vl).binding_short,
            None,
            "lamp {vl} kept a binding whose physical peer the bump erased"
        );
    }

    let snapshot = GroupReadPort::group_apply_snapshot(&store, 0)
        .expect("the adapter still exists");
    assert!(
        snapshot.rows.iter().all(|row| row.binding_short.is_none()),
        "a group apply would still find something to program: {:?}",
        snapshot
            .rows
            .iter()
            .filter(|row| row.binding_short.is_some())
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_counting_hydrate_agrees_with_the_collecting_one() {
    for adapters in [1u8, 2, 4] {
        let slices = temp_slice_store(&format!("shapes-agree-{adapters}"));
        write_slice(&slices, SliceKey::Adapters, b"not a persistence envelope");

        let collected = RegistryStore::with_adapter_count(adapters)
            .hydrate_from_store(&slices, adapters);
        let counted = RegistryStore::with_adapter_count(adapters)
            .hydrate_counts_from_store(&slices, adapters);

        assert_eq!(
            counted.loaded as usize,
            collected.loaded_slices.iter().count(),
            "loaded disagrees at {adapters} adapters"
        );
        assert_eq!(
            counted.defaulted as usize,
            collected.default_slices.iter().count(),
            "defaulted disagrees at {adapters} adapters"
        );
        assert_eq!(
            counted.errors as usize,
            collected.errors.iter().count(),
            "errors disagrees at {adapters} adapters"
        );
        assert!(counted.errors > 0, "fixture must produce a hydrate error");
    }
}
