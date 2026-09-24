use std::sync::Arc;

use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, BusPublisher};
use dali2rust_contracts::msg::PersistenceSliceOutcome;
use dali2rust_domain::registry::{
    GroupReadPort, RegistryReadPort, SceneReadPort, VirtualLampReadPort,
};
use dali2rust_platform::slice_store::SliceStore;
use dali2rust_registry_runtime::{RegistryStore, RegistryWorkerCounters};

use super::http_bridges::{wire_registry_http_ports, RegistryHttpPorts};
use super::workers::RegistryDeps;

pub(crate) struct RegistrySlice {
    pub http: RegistryHttpPorts,
    pub deps: RegistryDeps,
}

pub(crate) fn init_registry_slice(
    adapter_count: u8,
    persistence_slices: Option<Arc<dyn SliceStore>>,
    publisher: &BusPublisher,
    controller_hardware_id: Option<[u8; 6]>,
    wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
) -> RegistrySlice {
    let store = new_registry_store(adapter_count);
    if let Some(mac) = controller_hardware_id {
        store.seed_home_assistant_controller_id(mac);
    }
    let reads = registry_read_handles(Arc::clone(&store));
    let counters = new_registry_counters();
    if let Some(fs) = persistence_slices.clone() {
        log_internal_free("before hydrate");
        join_hydrate_registry_from_fs(store.clone(), publisher.clone(), fs, adapter_count);
        log_internal_free("after hydrate");
    }
    let http = wire_registry_http_ports(
        reads.worker,
        Arc::clone(&store),
        reads.http,
        Arc::clone(&counters),
        crate::runtime::http_bridges::TransferSeams {
            slices: persistence_slices.clone(),
            adapter_count,
            wall_clock,
        },
    );
    let deps = RegistryDeps {
        read_port: http.read_port.clone(),
        store,
        counters,
        adapter_count,
        persistence_slices,
    };
    RegistrySlice { http, deps }
}

#[cfg(target_os = "espidf")]
pub(crate) fn log_internal_free(stage: &str) {
    let h = dali2rust_bsp::heap_stats::EspHeapStats.boot_snapshot();
    log::warn!(
        "boot heap [{stage}]: internal free={} largest={}",
        h.internal_free_bytes,
        h.internal_largest_block_bytes
    );
}

#[cfg(not(target_os = "espidf"))]
pub(crate) fn log_internal_free(_stage: &str) {}

pub(crate) struct RegistryReadHandles {
    pub worker: Arc<dyn RegistryReadPort>,
    pub http: RegistryHttpReadPorts,
}

pub(crate) struct RegistryHttpReadPorts {
    pub adapter: Arc<dyn dali2rust_domain::registry::AdapterReadPort>,
    pub group: Arc<dyn GroupReadPort>,
    pub scene: Arc<dyn SceneReadPort>,
    pub physical: Arc<dyn dali2rust_domain::registry::PhysicalDeviceReadPort>,
    pub vl: Arc<dyn VirtualLampReadPort>,
    pub hcl: Arc<dyn dali2rust_domain::registry::HclScheduleReadPort>,
    pub poller: Arc<dyn dali2rust_domain::registry::PollerSettingsReadPort>,
    pub dali_settings: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    pub redundancy: Arc<dyn dali2rust_domain::registry::RedundancySettingsReadPort>,
    pub dali_settings_role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    pub policies: Arc<dyn dali2rust_domain::registry::PoliciesReadPort>,
    pub home_assistant: Arc<dyn dali2rust_domain::registry::HomeAssistantSettingsReadPort>,
}

pub(crate) fn new_registry_store(adapter_count: u8) -> Arc<RegistryStore> {
    Arc::new(RegistryStore::with_adapter_count(adapter_count))
}

pub(crate) fn new_registry_counters() -> Arc<RegistryWorkerCounters> {
    Arc::new(RegistryWorkerCounters::default())
}

pub(crate) fn registry_read_handles(store: Arc<RegistryStore>) -> RegistryReadHandles {
    RegistryReadHandles {
        worker: store.clone(),
        http: RegistryHttpReadPorts {
            adapter: store.clone(),
            group: store.clone(),
            scene: store.clone(),
            physical: store.clone(),
            vl: store.clone(),
            hcl: store.clone(),
            poller: store.clone(),
            dali_settings: store.clone(),
            redundancy: store.clone(),
            dali_settings_role: store.clone(),
            policies: store.clone(),
            home_assistant: store,
        },
    }
}

pub(crate) fn hydrate_registry_from_fs(
    store: &RegistryStore,
    publisher: &BusPublisher,
    fs: &dyn SliceStore,
    adapter_count: u8,
) {
    let report = store.hydrate_from_store(fs, adapter_count);
    if !report.loaded_slices.is_empty() {
        log::info!(
            "persistence: hydrated {} slices, {} defaults, {} errors",
            report.loaded_slices.len(),
            report.default_slices.len(),
            report.errors.len()
        );
    }
    for slice in &report.loaded_slices {
        publish_slice_result(publisher, slice.clone(), PersistenceSliceOutcome::Loaded, None);
    }
    for slice in &report.default_slices {
        if report.errors.iter().any(|e| e.kind == *slice) {
            continue;
        }
        publish_slice_result(publisher, slice.clone(), PersistenceSliceOutcome::Defaults, None);
    }
    for e in &report.errors {
        let error = dali2rust_contracts::msg::CompactErrorPayload::new(
            dali2rust_contracts::msg::ErrorCode::ExecutionFailed,
            e.error.as_str(),
        );
        publish_slice_result(
            publisher,
            e.kind.clone(),
            PersistenceSliceOutcome::Failed,
            Some(error),
        );
    }
}

#[inline(never)]
fn publish_slice_result(
    publisher: &BusPublisher,
    slice: dali2rust_contracts::msg::PersistenceSliceKind,
    outcome: PersistenceSliceOutcome,
    error: Option<dali2rust_contracts::msg::CompactErrorPayload>,
) {
    let ev = dali2rust_contracts::bus::event_envelope(dali2rust_contracts::SOURCE_ID_UNSPECIFIED, 0, 0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::PersistenceLoadResultEvent { slice, outcome, error });
    publish_or_drop(
        publisher,
        BusChannel::Events,
        BusFrame::event(ev),
        "persistence-hydrate",
    );
}

pub(crate) fn join_hydrate_registry_from_fs(
    store: Arc<RegistryStore>,
    publisher: BusPublisher,
    fs: Arc<dyn SliceStore>,
    adapter_count: u8,
) {
    dali2rust_bsp::esp_thread::run_on_named_stack_in(
        c"registry-hydrate",
        dali2rust_bsp::std_thread_stack::HYDRATION_WORKER,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        || hydrate_registry_from_fs(&store, &publisher, fs.as_ref(), adapter_count),
    )
    .expect("registry hydrate thread");
}
