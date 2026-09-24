mod adapter;
mod config_write;
mod confirm;
mod dispatch;
mod group;
mod matrix;
mod hcl;
mod home_assistant_settings;
mod physical_device;
mod dali_settings;
mod policies;
mod transfer;
mod redundancy_settings;
mod input_devices;
mod poller_settings;
mod runtime_apply;
mod scene;
mod virtual_lamp;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bus::{recv_then_drain, BusFrame, BusId, BusPublisher, BusSubscriberRx, WorkerTurn};
use dali2rust_platform::slice_store::SliceStore;

use crate::runtime::registry::RegistryStore;
use dali2rust_bsp::esp_thread;
use dali2rust_bsp::stack_probe::StackLowWater;
use dali2rust_bsp::std_thread_stack;

use dispatch::process_one;
pub use dispatch::REGISTRY_WORKER_HANDLED_COMMANDS;

const PERSISTENCE_DEBOUNCE: Duration = Duration::from_millis(500);
const MEMORY_BANK_STAGING_MAX_AGE_MS: u64 = 30_000;

use crate::runtime::registry::config_write_stage::CONFIG_WRITE_STAGE_MAX_AGE_MS;
use crate::runtime::registry::hcl_schedules::HCL_SCHEDULE_STAGE_MAX_AGE_MS;
const RECV_TIMEOUT: Duration = Duration::from_millis(250);


fn command_flush_interval(store: &RegistryStore) -> Duration {
    let deliberate_config_write = store.dirty.adapters.load(Ordering::Acquire)
        || store.dirty.hcl_schedules.load(Ordering::Acquire)
        || store.dirty.poller_settings.load(Ordering::Acquire)
        || store.dirty.home_assistant_settings.load(Ordering::Acquire);
    if deliberate_config_write {
        Duration::ZERO
    } else {
        PERSISTENCE_DEBOUNCE
    }
}

fn flush_persistence(
    persistence_slices: &Option<Arc<dyn SliceStore>>,
    store: &RegistryStore,
    last_flush: &mut Instant,
    min_interval: Duration,
) {
    let Some(fs) = persistence_slices.as_ref() else {
        return;
    };
    if !store.dirty.any_dirty() || last_flush.elapsed() < min_interval {
        return;
    }
    if dali2rust_platform::flash_gate::firmware_write_open() {
        return;
    }
    let started = Instant::now();
    store.flush_dirty_slices(fs.as_ref());
    let flush_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    if dali2rust_platform::dali::note_persist_flush(flush_ms) {
        log::warn!("registry flush: slow flush_dirty_slices took {flush_ms} ms");
    }
    *last_flush = Instant::now();
}

#[derive(Debug, Default)]
pub struct RegistryCommandCounters {
    pub input_device_metadata_applied: AtomicU32,
    pub input_device_metadata_rejected: AtomicU32,
    pub runtime_updates_applied: AtomicU32,
    pub runtime_updates_superseded: AtomicU32,
    pub config_updates_applied: AtomicU32,
    pub config_write_signal_publish_failed: AtomicU32,
    pub config_write_signal_publish_retried: AtomicU32,
    pub adapter_settings_applied: AtomicU32,
    pub group_metadata_patches_applied: AtomicU32,
    pub group_matrix_patches_applied: AtomicU32,
    pub scene_metadata_patches_applied: AtomicU32,
    pub scene_matrix_patches_applied: AtomicU32,
    pub physical_device_overrides_applied: AtomicU32,
    pub physical_device_runtime_applied: AtomicU32,
    pub virtual_lamp_runtime_applied: AtomicU32,
    pub virtual_lamp_metadata_patches_applied: AtomicU32,
    pub virtual_lamp_bindings_applied: AtomicU32,
    pub hcl_schedule_upserts_applied: AtomicU32,
    pub hcl_schedule_deletes_applied: AtomicU32,
    pub poller_settings_applied: AtomicU32,
    pub dali_settings_applied: AtomicU32,
    pub redundancy_settings_applied: AtomicU32,
    pub policies_applied: AtomicU32,
    pub slice_reloads: AtomicU32,
    pub home_assistant_settings_applied: AtomicU32,
    pub runtime_events_published: AtomicU32,
    pub ignored_commands: AtomicU32,
}

#[derive(Debug, Default)]
pub struct RegistryEventsCounters {
    pub dali_attributes_committed: AtomicU32,
    pub scene_recalls_noted: AtomicU32,
    pub input_instance_readbacks_applied: AtomicU32,
    pub input_presence_cleared: AtomicU32,
    pub input_scan_progress_applied: AtomicU32,
    pub input_scan_progress_rejected: AtomicU32,
    pub input_events_applied: AtomicU32,
    pub input_events_unattributed: AtomicU32,
    pub input_power_cycles_seen: AtomicU32,
    pub app_control_applied: AtomicU32,
    pub discovery_progress_applied: AtomicU32,
    pub discovery_progress_rejected: AtomicU32,
    pub discovery_scan_reconciled_applied: AtomicU32,
    pub attribute_read_evidence_applied: AtomicU32,
    pub memory_bank_read_committed: AtomicU32,
    pub group_membership_programmed_applied: AtomicU32,
    pub scene_programmed_applied: AtomicU32,
    pub dali_address_changes_committed: AtomicU32,
    pub dali_device_replacements_committed: AtomicU32,
    pub ignored_events: AtomicU32,
}

#[derive(Debug, Default)]
pub struct RegistryWorkerCounters {
    pub command: RegistryCommandCounters,
    pub events: RegistryEventsCounters,
}

struct RegistryWorkerDeps {
    publisher: BusPublisher,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: Arc<RegistryStore>,
    counters: Arc<RegistryWorkerCounters>,
    persistence_slices: Option<Arc<dyn SliceStore>>,
}

static WORKER_STACK: StackLowWater =
    StackLowWater::new("registry_worker", std_thread_stack::STATE_WORKER_STACK);

fn frame_tag(frame: &BusFrame) -> &'static str {
    match frame {
        BusFrame::Command(ce) => ce.payload.variant_name(),
        BusFrame::Event(ee) => ee.payload.variant_name(),
        BusFrame::Confirmation(_) => "confirmation",
    }
}

fn apply_frame_and_flush(frame: BusFrame, deps: &RegistryWorkerDeps, last_flush: &mut Instant) {
    let tag = frame_tag(&frame);
    match &frame {
        BusFrame::Command(_) => process_one(
            frame,
            &deps.publisher,
            deps.primary_adapter_id,
            deps.adapter_count,
            &deps.store,
            &deps.counters.command,
            deps.persistence_slices.as_ref(),
        ),
        BusFrame::Event(_) => crate::runtime::registry_events_worker::apply_event_frame(
            frame,
            &deps.publisher,
            deps.primary_adapter_id,
            &deps.store,
            &deps.counters.events,
        ),
        BusFrame::Confirmation(_) => {}
    }
    flush_persistence(
        &deps.persistence_slices,
        &deps.store,
        last_flush,
        command_flush_interval(&deps.store),
    );
    WORKER_STACK.note(tag);
}

fn evict_and_flush_on_idle(deps: &RegistryWorkerDeps, last_flush: &mut Instant) {
    deps.store
        .evict_stale_memory_bank_staging(MEMORY_BANK_STAGING_MAX_AGE_MS);
    deps.store
        .evict_stale_config_write_stages(CONFIG_WRITE_STAGE_MAX_AGE_MS);
    deps.store
        .evict_stale_hcl_schedule_stages(HCL_SCHEDULE_STAGE_MAX_AGE_MS);
    flush_persistence(
        &deps.persistence_slices,
        &deps.store,
        last_flush,
        PERSISTENCE_DEBOUNCE,
    );
    WORKER_STACK.note("idle");
}

fn run_registry_worker_loop(
    rx: BusSubscriberRx,
    deps: &RegistryWorkerDeps,
    liveness: &dali2rust_platform::liveness::LivenessBeat,
) {
    let mut last_flush = Instant::now();
    loop {
        liveness.beat(dali2rust_platform::liveness::monotonic_ms());
        let turn = liveness.while_turning(|| {
            recv_then_drain(&rx, RECV_TIMEOUT, |frame| {
                apply_frame_and_flush(frame, deps, &mut last_flush)
            })
        });
        match turn {
            WorkerTurn::Handled => {}
            WorkerTurn::Idle => evict_and_flush_on_idle(deps, &mut last_flush),
            WorkerTurn::Disconnected => {
                flush_persistence(&deps.persistence_slices, &deps.store, &mut last_flush, Duration::ZERO);
                break;
            }
        }
    }
}

pub fn spawn_registry_worker(
    rx: BusSubscriberRx,
    publisher: BusPublisher,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: Arc<RegistryStore>,
    counters: Arc<RegistryWorkerCounters>,
    persistence_slices: Option<Arc<dyn SliceStore>>,
    liveness: Arc<dali2rust_platform::liveness::LivenessBeat>,
) -> std::thread::JoinHandle<()> {
    let deps = RegistryWorkerDeps {
        publisher,
        primary_adapter_id,
        adapter_count,
        store,
        counters,
        persistence_slices,
    };
    esp_thread::spawn_named_stack_in(
        c"registry_worker",
        std_thread_stack::STATE_WORKER_STACK,
        esp_thread::StackHome::ExternalOnXip,
        move || run_registry_worker_loop(rx, &deps, liveness.as_ref()),
    )
}

#[cfg(test)]
mod tests {
    use super::{command_flush_interval, flush_persistence, RegistryStore, PERSISTENCE_DEBOUNCE};
    use crate::test_support::CountingStore;
    use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct BlockingStore {
        entered: Sender<()>,
        release: Mutex<Receiver<()>>,
    }

    struct BlockingSession<'a> {
        store: &'a BlockingStore,
    }

    impl SliceWriteSession for BlockingSession<'_> {
        fn append(&mut self, _chunk: &[u8]) -> Result<(), StoreError> {
            Ok(())
        }

        fn commit(self: Box<Self>) -> Result<(), StoreError> {
            self.store.entered.send(()).expect("probe listening");
            let release = self.store.release.lock().expect("release rx");
            release.recv().expect("release signal");
            Ok(())
        }

        fn abort(self: Box<Self>) {}
    }

    impl SliceStore for BlockingStore {
        fn load(&self, _key: SliceKey) -> Result<Vec<u8>, StoreError> {
            Err(StoreError::Missing)
        }

        fn begin_write(&self, _key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
            Ok(Box::new(BlockingSession { store: self }))
        }
    }

    #[test]
    fn readers_stay_available_while_flush_writes_flash() {
        use dali2rust_domain::registry::AdapterReadPort;
        let store = Arc::new(RegistryStore::with_adapter_count(1));
        store.dirty.adapters.store(true, Ordering::Release);
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let slices = BlockingStore {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        };
        let flusher = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.flush_dirty_slices(&slices))
        };
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("flush reaches the flash write");

        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let reader = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                let _ = read_tx.send(store.adapter_view(0).expect("adapter").name);
            })
        };
        let name = read_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reader must not park behind the flushing writer thread");
        assert_eq!(name, "Main DALI");

        release_tx.send(()).expect("release flush");
        reader.join().expect("reader thread");
        flusher.join().expect("flusher thread");
    }

    #[test]
    fn physical_device_flush_is_debounced() {
        let slices = Arc::new(CountingStore::default());
        let persistence_slices: Option<Arc<dyn SliceStore>> = Some(slices.clone());
        let store = RegistryStore::with_adapter_count(1);
        let mut last_flush = Instant::now();

        store.dirty.mark_physical_device_dirty(0, 0);
        flush_persistence(
            &persistence_slices,
            &store,
            &mut last_flush,
            command_flush_interval(&store),
        );
        assert_eq!(slices.write_count(), 0);

        last_flush = Instant::now() - PERSISTENCE_DEBOUNCE;
        flush_persistence(
            &persistence_slices,
            &store,
            &mut last_flush,
            command_flush_interval(&store),
        );
        assert_eq!(slices.write_count(), 1);
    }

    #[test]
    fn adapter_flush_stays_eager() {
        let slices = Arc::new(CountingStore::default());
        let persistence_slices: Option<Arc<dyn SliceStore>> = Some(slices.clone());
        let store = RegistryStore::with_adapter_count(1);
        let mut last_flush = Instant::now();

        store.dirty.adapters.store(true, Ordering::Release);
        flush_persistence(
            &persistence_slices,
            &store,
            &mut last_flush,
            command_flush_interval(&store),
        );
        assert_eq!(slices.write_count(), 1);
    }

    #[test]
    fn unchanged_attribute_read_flushes_nothing() {
        use dali2rust_contracts::msg::DaliAttributeReadChunk;

        let slices = Arc::new(CountingStore::default());
        let persistence_slices: Option<Arc<dyn SliceStore>> = Some(slices.clone());
        let store = RegistryStore::with_adapter_count(1);
        assert!(store.seed_discovered_dt8(4, Some(0x3333)));
        let fade = |ms: u32| DaliAttributeReadChunk::Common102 {
            version: None,
            device_type: None,
            physical_minimum: None,
            min_level: None,
            max_level: None,
            power_on_level: None,
            system_failure_level: None,
            fade_time_ms: Some(ms),
            fade_rate: None,
            supported_device_types: None,
            light_source_type: None,
            light_source_types: None,
        };
        let flush_now = |store: &RegistryStore| {
            let mut last_flush = Instant::now() - PERSISTENCE_DEBOUNCE;
            flush_persistence(
                &persistence_slices,
                store,
                &mut last_flush,
                command_flush_interval(store),
            );
        };

        assert!(store.apply_physical_device_attribute_chunk(0, 4, &fade(700)));
        flush_now(&store);
        assert_eq!(slices.write_count(), 1);

        assert!(store.apply_physical_device_attribute_chunk(0, 4, &fade(700)));
        flush_now(&store);
        assert_eq!(
            slices.write_count(),
            1,
            "an identical re-read must not reach flash"
        );

        assert!(store.apply_physical_device_attribute_chunk(0, 4, &fade(1000)));
        flush_now(&store);
        assert_eq!(slices.write_count(), 2);
    }
}
