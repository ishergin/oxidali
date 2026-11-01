#![allow(dead_code, reason = "Each `tests/*.rs` is its own binary and compiles this whole module, so every helper a given binary does not call is \"never used\" from that binary's point of view")]

use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::msg::{
    BusEventPayload, ColorMode, DeliveryStatus, DeviceType, RuntimeStateChangedEvent,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_platform::slice_store::SliceStore;
use dali2rust_test_support::{recv_confirmation_for, wait_until};
use dali2rust_registry_runtime::{
    spawn_registry_worker, RegistryStore, RegistryWorkerCounters, SlicePersistence,
};
use dali2rust_domain::registry::{
    AttributeSectionKind, AttributeSectionView, Common102AttributesView, Dt6LedAttributesView,
    Dt7SwitchingAttributesView,
    Dt8ColorAttributesView, ExtendedAttributesView, GroupReadPort, GroupsAttributesView,
    MemoryBankSummaryView, MemoryIdentityAttributesView, MemoryLuminaireAttributesView,
    MemoryProfileAttributesView, PhysicalDeviceAttributesView, PhysicalDeviceCoreView,
    PhysicalDeviceReadPort, ScenesAttributesView, VirtualLampReadPort,
};

pub const CONFIRMATION_DEADLINE: Duration = Duration::from_secs(4);

const DEVICE_EXISTS: &str = "the physical device exists";

pub fn core_view_of(store: &RegistryStore, short_address: u8) -> PhysicalDeviceCoreView {
    store.physical_device_core_view(0, short_address).expect(DEVICE_EXISTS)
}

pub fn memory_banks_of(store: &RegistryStore, short_address: u8) -> Vec<MemoryBankSummaryView> {
    store.physical_device_memory_banks(0, short_address).expect(DEVICE_EXISTS)
}

macro_rules! section_readers {
    ( $( $read:ident, $variant:ident, $view:ident );+ $(;)? ) => {
        $(
            pub fn $read(store: &RegistryStore, short_address: u8) -> $view {
                let kind = AttributeSectionKind::$variant;
                match store
                    .physical_device_attribute_section(0, short_address, kind)
                    .expect(DEVICE_EXISTS)
                {
                    AttributeSectionView::$variant(view) => *view,
                    other => panic!("asked for {kind:?}, got {:?}", other.kind()),
                }
            }
        )+
    };
}

section_readers! {
    common102_section, Common102, Common102AttributesView;
    dt6_led_section, Dt6Led, Dt6LedAttributesView;
    dt7_switching_section, Dt7Switching, Dt7SwitchingAttributesView;
    dt8_color_section, Dt8Color, Dt8ColorAttributesView;
    extended_section, Extended, ExtendedAttributesView;
    groups_section, Groups, GroupsAttributesView;
    memory_identity_section, MemoryIdentity, MemoryIdentityAttributesView;
    memory_luminaire_section, MemoryLuminaire, MemoryLuminaireAttributesView;
    memory_profile_section, MemoryProfile, MemoryProfileAttributesView;
    scenes_section, Scenes, ScenesAttributesView;
}

pub fn attributes_of(store: &RegistryStore, short_address: u8) -> PhysicalDeviceAttributesView {
    PhysicalDeviceAttributesView {
        common102: common102_section(store, short_address),
        groups: groups_section(store, short_address),
        scenes: scenes_section(store, short_address),
        dt6_led: dt6_led_section(store, short_address),
        dt8_color: dt8_color_section(store, short_address),
        extended: extended_section(store, short_address),
        memory_identity: memory_identity_section(store, short_address),
        memory_profile: memory_profile_section(store, short_address),
    }
}

pub struct RegistryTestStack {
    pub publisher: BusPublisher,
    pub conf_rx: std::sync::mpsc::Receiver<BusFrame>,
    pub ev_rx: std::sync::mpsc::Receiver<BusFrame>,
    pub store: Arc<RegistryStore>,
    pub counters: Arc<RegistryWorkerCounters>,
    pub _host: BusHost,
}

pub struct RegistryStackOptions {
    pub adapter_count: u8,
    pub event_observer_capacity: usize,
    pub slices: Option<Arc<dyn SliceStore>>,
    pub store: Option<Arc<RegistryStore>>,
    pub counters: Option<Arc<RegistryWorkerCounters>>,
    pub production_subscription: bool,
}

impl Default for RegistryStackOptions {
    fn default() -> Self {
        Self {
            adapter_count: 1,
            event_observer_capacity: 32,
            slices: None,
            store: None,
            counters: None,
            production_subscription: false,
        }
    }
}

const REGISTRY_INBOX: usize = 64;

fn registry_events(production_subscription: bool) -> &'static [&'static str] {
    if production_subscription {
        dali2rust_registry_runtime::REGISTRY_EVENTS_HANDLED_EVENTS
    } else {
        dali2rust_contracts::msg::EVENT_VARIANT_NAMES
    }
}

fn registry_commands(production_subscription: bool) -> &'static [&'static str] {
    if production_subscription {
        dali2rust_registry_runtime::REGISTRY_WORKER_HANDLED_COMMANDS
    } else {
        dali2rust_contracts::msg::COMMAND_VARIANT_NAMES
    }
}

pub fn spawn_registry_stack(adapter_count: u8, event_observer_capacity: usize) -> RegistryTestStack {
    spawn_registry_stack_with(RegistryStackOptions {
        adapter_count,
        event_observer_capacity,
        ..RegistryStackOptions::default()
    })
}

pub fn spawn_registry_stack_with(options: RegistryStackOptions) -> RegistryTestStack {
    let RegistryStackOptions {
        adapter_count,
        event_observer_capacity,
        slices,
        store,
        counters,
        production_subscription,
    } = options;
    let (commands, events) =
        (registry_commands(production_subscription), registry_events(production_subscription));
    let (host, publisher, (registry_rx, conf_rx, ev_rx)) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            (
                reg.subscribe_commands_and_events(REGISTRY_INBOX, commands, events),
                reg.subscribe_confirmations(32),
                reg.subscribe_events(event_observer_capacity, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            )
        },
    );
    let store = store.unwrap_or_else(|| Arc::new(RegistryStore::with_adapter_count(adapter_count)));
    let counters = counters.unwrap_or_default();
    let _registry = spawn_registry_worker(
        registry_rx,
        publisher.clone(),
        BusId::default(),
        adapter_count,
        Arc::clone(&store),
        Arc::clone(&counters),
        slices.map(SlicePersistence::registry_owned_only),
        std::sync::Arc::new(dali2rust_platform::liveness::LivenessBeat::new("test", 60_000)),
    );
    RegistryTestStack {
        publisher,
        conf_rx,
        ev_rx,
        store,
        counters,
        _host: host,
    }
}

pub fn publish_cmd(publisher: &BusPublisher, cmd: dali2rust_contracts::msg::CommandEnvelope) {
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, BusFrame::command(cmd)),
        PublishResult::Queued
    );
}

pub fn publish_event(publisher: &BusPublisher, ev: dali2rust_contracts::msg::EventEnvelope) {
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
}

pub fn publish_group_matrix_write(
    publisher: &BusPublisher,
    corr: u64,
    adapter_id: u8,
    rows: dali2rust_contracts::msg::GroupMatrixDesiredRowList,
) {
    publish_cmd(
        publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::GroupMatrixDesiredPatchCommand { adapter_id, rows },
        ),
    );
    publish_config_write_commit(
        publisher,
        corr,
        dali2rust_contracts::msg::ConfigWriteResource::GroupMatrix,
        adapter_id,
        None,
        1,
    );
}

pub fn publish_scene_matrix_write(
    publisher: &BusPublisher,
    corr: u64,
    adapter_id: u8,
    scene_id: u8,
    rows: dali2rust_contracts::msg::SceneMatrixDesiredRowList,
) {
    publish_cmd(
        publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::SceneMatrixDesiredPatchCommand {
                adapter_id,
                scene_id,
                rows,
            },
        ),
    );
    publish_config_write_commit(
        publisher,
        corr,
        dali2rust_contracts::msg::ConfigWriteResource::SceneMatrix,
        adapter_id,
        Some(scene_id),
        1,
    );
}

pub fn publish_config_write_commit(
    publisher: &BusPublisher,
    corr: u64,
    resource: dali2rust_contracts::msg::ConfigWriteResource,
    adapter_id: u8,
    scene_id: Option<u8>,
    chunks: u8,
) {
    publish_cmd(
        publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::ConfigWriteCommitCommand {
                resource,
                adapter_id,
                scene_id,
                chunks,
            },
        ),
    );
}

pub fn seed_physical_via_discovery(
    publisher: &BusPublisher,
    correlation_id: u64,
    short_address: u8,
    device_type: DeviceType,
    store: &RegistryStore,
) {
    publish_event(
        publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address, random_address: None, device_type, color_mode: ColorMode::Unknown, dt8_xy_capable: false, dt8_tc_capable: false, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None }),
    );
    wait_until(
        || store.physical_device_exists(0, short_address),
        Duration::from_millis(500),
    );
}

pub fn seed_bound_group_member(
    stack: &RegistryTestStack,
    virtual_lamp_id: u8,
    short_address: u8,
    group_id: u8,
    correlation_id: u64,
) {
    seed_physical_via_discovery(&stack.publisher, correlation_id, short_address, DeviceType::Dt8Color, &stack.store);
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id + 1, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id, physical_short_address: short_address }),
    );
    wait_until(
        || {
            stack.store.virtual_lamp_view(0, virtual_lamp_id).binding_short == Some(short_address)
        },
        CONFIRMATION_DEADLINE,
    );
    publish_event(
        &stack.publisher,
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id + 2, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesReadEvent { registry_adapter_id: 0, short_address, last_chunk: true, chunk: dali2rust_contracts::msg::DaliAttributeReadChunk::Groups { membership: Some(1u16 << group_id) } }),
    );
    wait_until(
        || {
            stack
                .store
                .applied_group_member_mask(0, group_id)
                .is_some_and(|mask| mask & (1u64 << virtual_lamp_id) != 0)
        },
        CONFIRMATION_DEADLINE,
    );
}

pub fn recv_confirm_for(
    rx: &std::sync::mpsc::Receiver<BusFrame>,
    correlation_id: u64,
) -> std::sync::Arc<dali2rust_contracts::msg::ConfirmationEnvelope> {
    recv_confirmation_for(rx, correlation_id, CONFIRMATION_DEADLINE)
}

pub fn recv_runtime_state_changed(
    rx: &std::sync::mpsc::Receiver<BusFrame>,
    correlation_id: u64,
) -> RuntimeStateChangedEvent {
    for _ in 0..24 {
        let Ok(BusFrame::Event(event)) = rx.recv_timeout(Duration::from_millis(400)) else {
            continue;
        };
        if event.meta.correlation_id != correlation_id {
            continue;
        }
        if let BusEventPayload::RuntimeStateChangedEvent(body) = &event.payload {
            return body.clone();
        }
    }
    panic!("RuntimeStateChangedEvent for correlation {correlation_id}");
}

pub fn assert_exec_failed_msg(c: &dali2rust_contracts::msg::ConfirmationEnvelope, needle: &str) {
    assert_eq!(c.status, DeliveryStatus::ExecutionFailed);
    let err = c
        .confirmation
        .error
        .as_ref()
        .expect("product error");
    assert!(
        err.message.as_str().contains(needle),
        "got {:?}",
        err.message.as_str()
    );
}
