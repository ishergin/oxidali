mod adapter;
mod command_ctx;
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

use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bus::{recv_then_drain, BusFrame, BusId, BusPublisher, BusSubscriberRx, WorkerTurn};
use dali2rust_contracts::msg::BusCommandPayload;
use dali2rust_platform::slice_store::SliceStore;

use crate::runtime::registry::replaced_slices::AnnouncementPacing;
use crate::runtime::registry::transfer::{ForeignSliceOwner, RegistryOwnedOnly};
use crate::runtime::registry::RegistryStore;
use dali2rust_bsp::esp_thread;
use dali2rust_bsp::stack_probe::StackLowWater;
use dali2rust_bsp::std_thread_stack;

use dispatch::process_one;
pub use dispatch::REGISTRY_WORKER_HANDLED_COMMANDS;

const PERSISTENCE_DEBOUNCE: Duration = Duration::from_millis(500);
const HCL_SWITCH_WRITE_INTERVAL: Duration = Duration::from_secs(30 * 60);
const MEMORY_BANK_STAGING_MAX_AGE_MS: u64 = 30_000;

use crate::runtime::registry::config_write_stage::CONFIG_WRITE_STAGE_MAX_AGE_MS;
use crate::runtime::registry::hcl_schedules::HCL_SCHEDULE_STAGE_MAX_AGE_MS;
const RECV_TIMEOUT: Duration = Duration::from_millis(250);
pub const HELD_SLICE_REREAD_INTERVAL: Duration = Duration::from_secs(2);
pub const ANNOUNCEMENT_INBOX_FLOOR: usize = 64;
pub const ANNOUNCEMENT_PARK_WINDOW: Duration = Duration::from_secs(1);
const ANNOUNCEMENT_SHARE_OF_INBOX: usize = 2;
const WINDOWS_PER_PARK: u128 = ANNOUNCEMENT_PARK_WINDOW.as_millis() / RECV_TIMEOUT.as_millis();
pub(crate) const ANNOUNCEMENT_PACING: AnnouncementPacing = AnnouncementPacing {
    per_window: ANNOUNCEMENT_INBOX_FLOOR / ANNOUNCEMENT_SHARE_OF_INBOX / WINDOWS_PER_PARK as usize,
    window: RECV_TIMEOUT,
};


fn command_flush_interval(store: &RegistryStore) -> Duration {
    if store.dirty.deliberate_config_write() {
        Duration::ZERO
    } else {
        PERSISTENCE_DEBOUNCE
    }
}

struct FlushClock {
    last_flush: Instant,
    switch_interval: Duration,
    last_reread: Instant,
    reread_interval: Duration,
    reread_failures: transfer::RereadFailures,
}

impl FlushClock {
    fn new(switch_interval: Duration) -> Self {
        Self {
            last_flush: Instant::now(),
            switch_interval,
            last_reread: Instant::now(),
            reread_interval: HELD_SLICE_REREAD_INTERVAL,
            reread_failures: transfer::RereadFailures::default(),
        }
    }
}

fn reread_held_when_due(deps: &RegistryWorkerDeps, clock: &mut FlushClock) {
    if clock.last_reread.elapsed() < clock.reread_interval
        || dali2rust_platform::flash_gate::firmware_write_open()
    {
        return;
    }
    clock.last_reread = Instant::now();
    let outcome = transfer::reread_held_slices(deps);
    if let Some(e) = clock.reread_failures.first(&outcome) {
        log::error!("registry: held slices are not read again while the reload stack fails: {e}");
    }
}

fn flush_paced(
    deps: &RegistryWorkerDeps,
    clock: &mut FlushClock,
    min_interval: fn(&RegistryStore) -> Duration,
) {
    reread_held_when_due(deps, clock);
    if deps.store.dirty.hcl_switches_due(clock.switch_interval) {
        deps.store.dirty.mark_hcl_schedules_dirty();
    }
    let min_interval = min_interval(&deps.store);
    let slices = deps.persistence.as_ref().map(|p| &p.slices);
    flush_persistence(slices, &deps.store, &mut clock.last_flush, min_interval);
}

fn flush_persistence(
    persistence_slices: Option<&Arc<dyn SliceStore>>,
    store: &RegistryStore,
    last_flush: &mut Instant,
    min_interval: Duration,
) {
    let Some(fs) = persistence_slices else {
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
    pub hcl_switches_coalesced: AtomicU32,
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

#[derive(Clone)]
pub struct SlicePersistence {
    pub slices: Arc<dyn SliceStore>,
    pub foreign: Arc<dyn ForeignSliceOwner>,
}

impl SlicePersistence {
    pub fn registry_owned_only(slices: Arc<dyn SliceStore>) -> Self {
        Self {
            slices,
            foreign: Arc::new(RegistryOwnedOnly),
        }
    }
}

struct RegistryWorkerDeps {
    publisher: BusPublisher,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: Arc<RegistryStore>,
    counters: Arc<RegistryWorkerCounters>,
    persistence: Option<SlicePersistence>,
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

fn flush_before_import(frame: &BusFrame, deps: &RegistryWorkerDeps, clock: &mut FlushClock) {
    let BusFrame::Command(command) = frame else {
        return;
    };
    if !matches!(command.payload, BusCommandPayload::RegistrySliceReloadCommand(_))
        || !deps.store.import_staged_for(command.meta.correlation_id)
    {
        return;
    }
    if deps.store.dirty.take_hcl_switches_waiting() {
        deps.store.dirty.mark_hcl_schedules_dirty();
    }
    let slices = deps.persistence.as_ref().map(|p| &p.slices);
    flush_persistence(slices, &deps.store, &mut clock.last_flush, Duration::ZERO);
}

fn apply_frame_and_flush(frame: BusFrame, deps: &RegistryWorkerDeps, clock: &mut FlushClock) {
    let tag = frame_tag(&frame);
    flush_before_import(&frame, deps, clock);
    match &frame {
        BusFrame::Command(_) => process_one(
            frame,
            &deps.publisher,
            deps.primary_adapter_id,
            deps.adapter_count,
            &deps.store,
            &deps.counters.command,
            deps.persistence.as_ref(),
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
    flush_paced(deps, clock, command_flush_interval);
    WORKER_STACK.note(tag);
}

fn evict_and_flush_on_idle(deps: &RegistryWorkerDeps, clock: &mut FlushClock) {
    deps.store
        .evict_stale_memory_bank_staging(MEMORY_BANK_STAGING_MAX_AGE_MS);
    deps.store
        .evict_stale_config_write_stages(CONFIG_WRITE_STAGE_MAX_AGE_MS);
    deps.store
        .evict_stale_hcl_schedule_stages(HCL_SCHEDULE_STAGE_MAX_AGE_MS);
    deps.store.evict_stale_import();
    flush_paced(deps, clock, |_| PERSISTENCE_DEBOUNCE);
    announce_queued(deps);
    WORKER_STACK.note("idle");
}

#[inline(never)]
fn announce_queued(deps: &RegistryWorkerDeps) {
    let target = deps.primary_adapter_id.0;
    deps.store.announce_queued(&deps.publisher, target, ANNOUNCEMENT_PACING);
}

fn run_registry_worker_loop(
    rx: BusSubscriberRx,
    deps: &RegistryWorkerDeps,
    liveness: &dali2rust_platform::liveness::LivenessBeat,
) {
    let mut clock = FlushClock::new(HCL_SWITCH_WRITE_INTERVAL);
    loop {
        liveness.beat(dali2rust_platform::liveness::monotonic_ms());
        let turn = liveness.while_turning(|| {
            recv_then_drain(&rx, RECV_TIMEOUT, |frame| {
                apply_frame_and_flush(frame, deps, &mut clock)
            })
        });
        match turn {
            WorkerTurn::Handled => announce_queued(deps),
            WorkerTurn::Idle => evict_and_flush_on_idle(deps, &mut clock),
            WorkerTurn::Disconnected => {
                clock.switch_interval = Duration::ZERO;
                flush_paced(deps, &mut clock, |_| Duration::ZERO);
                break;
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the public entry point of the composition root and the integration tests; each argument is a distinct wired handle"
)]
pub fn spawn_registry_worker(
    rx: BusSubscriberRx,
    publisher: BusPublisher,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: Arc<RegistryStore>,
    counters: Arc<RegistryWorkerCounters>,
    persistence: Option<SlicePersistence>,
    liveness: Arc<dali2rust_platform::liveness::LivenessBeat>,
) -> std::thread::JoinHandle<()> {
    let deps = RegistryWorkerDeps {
        publisher,
        primary_adapter_id,
        adapter_count,
        store,
        counters,
        persistence,
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
    use super::{
        announce_queued, apply_frame_and_flush, command_flush_interval, evict_and_flush_on_idle,
        flush_persistence, ANNOUNCEMENT_PACING,
        FlushClock, RegistryStore, RegistryWorkerCounters, RegistryWorkerDeps, SlicePersistence,
        HCL_SWITCH_WRITE_INTERVAL, HELD_SLICE_REREAD_INTERVAL, PERSISTENCE_DEBOUNCE,
    };
    use crate::test_support::{CountingStore, ProbeStore};
    use dali2rust_bus::{BusConfig, BusFrame, BusHost, BusId, BusPublisher};
    use dali2rust_contracts::bus::command_envelope;
    use dali2rust_contracts::msg::{
        fixed_text_32, fixed_text_64, AdapterSettingsUpdateCommand, BusCommandPayload,
        DaliAttributeReadChunk, HclAlgorithm, HclLevelMode,
        HclPointList, HclScheduleEnableCommand, HclSchedulePointRow, HclScheduleUpsertCommand,
        HclTargetList, HclTargetRow, HclTargetScope, HclTimeRef, RegistrySliceReloadCommand,
    };
    use crate::runtime::registry::import_stage::StagedSlice;
    use dali2rust_contracts::msg::BusEventPayload;
    use dali2rust_test_support::{recv_event_matching, wait_until};
    use dali2rust_domain::registry::{AdapterReadPort, HclScheduleReadPort};
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
            persistence_slices.as_ref(),
            &store,
            &mut last_flush,
            command_flush_interval(&store),
        );
        assert_eq!(slices.write_count(), 0);

        last_flush = Instant::now() - PERSISTENCE_DEBOUNCE;
        flush_persistence(
            persistence_slices.as_ref(),
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
            persistence_slices.as_ref(),
            &store,
            &mut last_flush,
            command_flush_interval(&store),
        );
        assert_eq!(slices.write_count(), 1);
    }

    #[test]
    fn unchanged_attribute_read_flushes_nothing() {
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
                persistence_slices.as_ref(),
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

    const ALL_DAYS: u8 = 0b0111_1111;
    const SWITCH_BURST: [bool; 5] = [false, true, false, true, false];

    fn deps_on<S: SliceStore + 'static>(
        publisher: BusPublisher,
        slices: &Arc<S>,
    ) -> RegistryWorkerDeps {
        RegistryWorkerDeps {
            publisher,
            primary_adapter_id: BusId::default(),
            adapter_count: 1,
            store: Arc::new(RegistryStore::with_adapter_count(1)),
            counters: Arc::new(RegistryWorkerCounters::default()),
            persistence: Some(SlicePersistence::registry_owned_only(slices.clone())),
        }
    }

    fn worker_on<S: SliceStore + 'static>(slices: &Arc<S>) -> (RegistryWorkerDeps, BusHost) {
        let (host, publisher, ()) = BusHost::spawn(BusConfig::default(), |_| ());
        (deps_on(publisher, slices), host)
    }

    const OBSERVED_EVENTS: usize = 64;
    const EVENT_WAIT: Duration = Duration::from_secs(2);

    fn worker_observed<S: SliceStore + 'static>(
        slices: &Arc<S>,
    ) -> (RegistryWorkerDeps, BusHost, Receiver<BusFrame>) {
        let (host, publisher, events) = BusHost::spawn(BusConfig::default(), |reg| {
            reg.subscribe_events(OBSERVED_EVENTS, dali2rust_contracts::msg::EVENT_VARIANT_NAMES)
        });
        (deps_on(publisher, slices), host, events)
    }

    fn command(correlation_id: u64, payload: impl Into<BusCommandPayload>) -> BusFrame {
        let target = BusId::default().0;
        BusFrame::command(command_envelope(0, correlation_id, target, None, payload))
    }

    fn schedule(schedule_id: &str) -> HclScheduleUpsertCommand {
        let target = HclTargetRow { adapter_id: 0, scope: HclTargetScope::Group, group_mask: 0b1 };
        let point = HclSchedulePointRow {
            time_ref: HclTimeRef::Absolute,
            offset_minutes: 1080,
            level_mode: HclLevelMode::Absolute,
            level: Some(60),
            color_temperature_kelvin: Some(2400),
        };
        HclScheduleUpsertCommand {
            schedule_id: fixed_text_32(schedule_id),
            enabled: true,
            algorithm: HclAlgorithm::Interpolated,
            active_days_mask: ALL_DAYS,
            latitude_microdeg: None,
            longitude_microdeg: None,
            first_target_index: 0,
            targets: HclTargetList::from_slice(&[target]).expect("one target"),
            first_point_index: 0,
            points: HclPointList::from_slice(&[point]).expect("one point"),
            last_chunk: true,
        }
    }

    fn switch(schedule_id: &str, enabled: bool) -> HclScheduleEnableCommand {
        HclScheduleEnableCommand { schedule_id: fixed_text_32(schedule_id), enabled }
    }

    fn bit_after_restart(slices: &CountingStore, schedule_id: &str) -> Option<bool> {
        let restarted = RegistryStore::with_adapter_count(1);
        restarted.hydrate_from_store(slices, 1);
        restarted.hcl_schedule_view(schedule_id).map(|view| view.enabled)
    }

    fn idle_turn(deps: &RegistryWorkerDeps, clock: &mut FlushClock) {
        clock.last_flush = Instant::now() - PERSISTENCE_DEBOUNCE;
        evict_and_flush_on_idle(deps, clock);
    }

    #[test]
    fn a_burst_of_rule_switches_is_written_once_with_the_last_bit() {
        let slices = Arc::new(CountingStore::default());
        let (deps, _host) = worker_on(&slices);
        let mut clock = FlushClock::new(HCL_SWITCH_WRITE_INTERVAL);
        apply_frame_and_flush(command(1, schedule("porch")), &deps, &mut clock);
        assert_eq!(slices.write_count(), 1, "a schedule edit is written at once");

        for (correlation_id, enabled) in (2..).zip(SWITCH_BURST) {
            apply_frame_and_flush(command(correlation_id, switch("porch", enabled)), &deps, &mut clock);
        }
        idle_turn(&deps, &mut clock);
        assert_eq!(slices.write_count(), 1, "the burst waits for its interval");
        assert_eq!(
            bit_after_restart(&slices, "porch"),
            Some(true),
            "a restart inside the interval keeps the bit last written"
        );
        assert_eq!(
            deps.counters.command.hcl_switches_coalesced.load(Ordering::Relaxed),
            SWITCH_BURST.len() as u32 - 1,
            "every switch after the first joined the write already waiting"
        );

        clock.switch_interval = Duration::ZERO;
        idle_turn(&deps, &mut clock);
        idle_turn(&deps, &mut clock);
        assert_eq!(slices.write_count(), 2, "the whole burst costs one write");
        assert_eq!(bit_after_restart(&slices, "porch"), Some(false), "and it carries the last bit");
    }

    #[test]
    fn a_schedule_edit_writes_a_waiting_switch_at_once() {
        let slices = Arc::new(CountingStore::default());
        let (deps, _host) = worker_on(&slices);
        let mut clock = FlushClock::new(HCL_SWITCH_WRITE_INTERVAL);
        apply_frame_and_flush(command(1, schedule("porch")), &deps, &mut clock);
        apply_frame_and_flush(command(2, switch("porch", false)), &deps, &mut clock);
        assert_eq!(slices.write_count(), 1, "the switch waits");

        apply_frame_and_flush(command(3, schedule("dusk")), &deps, &mut clock);
        assert_eq!(slices.write_count(), 2, "the edit is written at once");
        assert_eq!(bit_after_restart(&slices, "porch"), Some(false), "and carries the switch");

        clock.switch_interval = Duration::ZERO;
        idle_turn(&deps, &mut clock);
        assert_eq!(slices.write_count(), 2, "nothing is left waiting");
    }

    #[test]
    fn an_import_keeps_a_switch_still_waiting_for_its_interval() {
        let slices = Arc::new(CountingStore::default());
        let (deps, _host) = worker_on(&slices);
        let mut clock = FlushClock::new(HCL_SWITCH_WRITE_INTERVAL);
        apply_frame_and_flush(command(1, schedule("porch")), &deps, &mut clock);
        apply_frame_and_flush(command(2, switch("porch", false)), &deps, &mut clock);

        import_poller_settings(&deps, &mut clock);

        assert_eq!(deps.store.hcl_schedule_view("porch").map(|view| view.enabled), Some(false));
        assert_eq!(bit_after_restart(&slices, "porch"), Some(false));
    }

    #[test]
    fn a_reload_with_no_stage_leaves_a_waiting_switch_waiting() {
        let slices = Arc::new(CountingStore::default());
        let (deps, _host) = worker_on(&slices);
        let mut clock = FlushClock::new(HCL_SWITCH_WRITE_INTERVAL);
        apply_frame_and_flush(command(1, schedule("porch")), &deps, &mut clock);
        apply_frame_and_flush(command(2, switch("porch", false)), &deps, &mut clock);
        let writes = slices.write_count();

        let reload = RegistrySliceReloadCommand { slice_name: fixed_text_32("poller_settings") };
        apply_frame_and_flush(command(IMPORT_WORKFLOW, reload), &deps, &mut clock);
        assert_eq!(slices.write_count(), writes, "nothing was staged, so nothing is flushed");
        assert!(deps.store.dirty.hcl_switches_due(Duration::ZERO), "the switch keeps its interval");
    }

    const IMPORT_WORKFLOW: u64 = 9;
    const IMPORTED_POLLER_INTERVAL_MS: u32 = 9_000;

    fn import_poller_settings(deps: &RegistryWorkerDeps, clock: &mut FlushClock) {
        let poller = crate::runtime::registry::persistence_slices::PersistablePollerSettingsSlice {
            enabled: true,
            interval_ms: IMPORTED_POLLER_INTERVAL_MS,
            attribute_groups_mask: 1,
            include_dt8_color: false,
            skip_unbound_virtual_lamps: true,
            include_energy: false,
            include_diagnostics: false,
        };
        let bytes = crate::runtime::registry::persistence_slices::encode_persistence_blob(
            &crate::runtime::registry::persistence_slices::PersistenceEnvelope::new(
                crate::runtime::registry::persistence_slices::POLLER_SETTINGS_SLICE_VERSION,
                poller,
            ),
        )
        .expect("encode poller slice");
        let staged = vec![StagedSlice { key: SliceKey::PollerSettings, bytes }];
        deps.store.stage_import(IMPORT_WORKFLOW, staged).expect("stage");
        let reload = RegistrySliceReloadCommand { slice_name: fixed_text_32("poller_settings") };
        apply_frame_and_flush(command(IMPORT_WORKFLOW, reload), deps, clock);
    }

    #[test]
    fn an_import_announces_the_settings_it_replaced() {
        let slices = Arc::new(CountingStore::default());
        let (deps, _host, events) = worker_observed(&slices);
        let mut clock = FlushClock::new(HCL_SWITCH_WRITE_INTERVAL);
        import_poller_settings(&deps, &mut clock);
        idle_turn(&deps, &mut clock);
        let changed = recv_event_matching(&events, EVENT_WAIT, |payload| {
            matches!(payload, BusEventPayload::PollerSettingsChangedEvent(_))
        });
        let BusEventPayload::PollerSettingsChangedEvent(poller) = &changed.payload else {
            panic!("not a poller settings event: {changed:?}");
        };
        assert_eq!(poller.interval_ms, IMPORTED_POLLER_INTERVAL_MS);
    }

    fn held_dali_settings_of_a_stood_down_controller(
    ) -> (RegistryWorkerDeps, BusHost, Receiver<BusFrame>) {
        let slices = Arc::new(ProbeStore::default());
        let installer = RegistryStore::with_adapter_count(1);
        installer.write_inner().dali_settings.application_active = false;
        installer.dirty.mark_dali_settings_dirty();
        installer.flush_dirty_slices(slices.as_ref());
        slices.fail_reads_of(Some(SliceKey::DaliSettings));
        let (deps, host, events) = worker_observed(&slices);
        deps.store.hydrate_from_store(slices.as_ref(), 1);
        slices.fail_reads_of(None);
        (deps, host, events)
    }

    #[test]
    fn a_settled_dali_settings_slice_announces_the_stored_role() {
        let (deps, _host, events) = held_dali_settings_of_a_stood_down_controller();
        let held = deps.store.dali_settings_row();
        assert!(held.application_active, "held: memory keeps the default");
        let mut clock = FlushClock::new(HCL_SWITCH_WRITE_INTERVAL);
        clock.reread_interval = Duration::ZERO;
        idle_turn(&deps, &mut clock);
        let reloaded =
            |p: &BusEventPayload| matches!(p, BusEventPayload::RegistrySliceReloadedEvent(_));
        let dali_of = |p: &BusEventPayload| match p {
            BusEventPayload::DaliSettingsChangedEvent(ev) => Some(ev.application_active),
            _ => None,
        };
        let heard = events_heard(&events, |heard| {
            heard.iter().any(reloaded) && heard.iter().any(|p| dali_of(p).is_some())
        });
        assert!(heard.iter().any(reloaded), "no RegistrySliceReloadedEvent after the settle");
        let active = heard.iter().find_map(dali_of).expect("no DaliSettingsChangedEvent");
        assert!(!active, "the event carried the default, not the stored role");
    }

    const EVENT_POLL: Duration = Duration::from_millis(20);

    fn events_heard(
        events: &Receiver<BusFrame>,
        enough: impl Fn(&[BusEventPayload]) -> bool,
    ) -> Vec<BusEventPayload> {
        let deadline = Instant::now() + EVENT_WAIT;
        let mut heard = Vec::new();
        while !enough(&heard) && Instant::now() < deadline {
            if let Ok(BusFrame::Event(event)) = events.recv_timeout(EVENT_POLL) {
                heard.push(event.payload.clone());
            }
        }
        heard
    }

    const FIRST_ADAPTER: u16 = 0;
    const QUEUED_LAMPS: u8 = 40;
    const TURNS_IN_ONE_WINDOW: usize = 5;

    fn lamps_queued(store: &RegistryStore, count: u8) {
        let mut lamps = store.write_inner();
        for virtual_lamp_id in 0..count {
            lamps.lamps.insert((0, virtual_lamp_id), Default::default());
        }
        drop(lamps);
        let watch = store.watch_reload(&[SliceKey::VirtualLamps { adapter_id: 0 }], 1);
        store.queue_announcements(store.replaced_entities(watch));
    }

    #[test]
    fn turns_inside_one_receive_window_share_its_announcement_budget() {
        let slices = Arc::new(CountingStore::default());
        let (deps, _host) = worker_on(&slices);
        lamps_queued(&deps.store, QUEUED_LAMPS);
        let queued = || deps.store.announcements_queued();
        assert_eq!(queued(), usize::from(QUEUED_LAMPS));
        (0..TURNS_IN_ONE_WINDOW).for_each(|_| announce_queued(&deps));
        let window = ANNOUNCEMENT_PACING.per_window;
        assert_eq!(queued(), usize::from(QUEUED_LAMPS) - window, "every turn spent a budget");
        let first_window = usize::from(QUEUED_LAMPS) - window;
        wait_until(|| { announce_queued(&deps); queued() < first_window }, EVENT_WAIT);
        assert_eq!(queued(), usize::from(QUEUED_LAMPS) - 2 * window, "the next window spent none");
    }

    fn rename_adapter(name: &str) -> BusFrame {
        let patch_mask = AdapterSettingsUpdateCommand::PATCH_NAME;
        let name = fixed_text_64(name);
        let rename = AdapterSettingsUpdateCommand { patch_mask, name, enabled: true };
        BusFrame::command(command_envelope(0, 1, FIRST_ADAPTER, None, rename))
    }

    fn adapter_name(store: &RegistryStore) -> Option<String> {
        store.adapter_view(0).map(|view| view.name)
    }

    fn unread(deps: &RegistryWorkerDeps) -> u32 {
        deps.store.persist_counters.unread_slices.load(Ordering::Acquire)
    }

    fn booted_with_adapters_unread() -> (Arc<ProbeStore>, RegistryWorkerDeps, BusHost) {
        let slices = Arc::new(ProbeStore::default());
        let installer = RegistryStore::with_adapter_count(1);
        let patch_mask = AdapterSettingsUpdateCommand::PATCH_NAME;
        let stored = Some("Stored Name");
        assert!(installer.apply_adapter_settings_from_command(0, patch_mask, stored, true));
        installer.flush_dirty_slices(slices.as_ref());
        slices.fail_reads_of(Some(SliceKey::Adapters));
        let (deps, host) = worker_on(&slices);
        deps.store.hydrate_from_store(slices.as_ref(), 1);
        slices.fail_reads_of(None);
        (slices, deps, host)
    }

    #[test]
    fn a_turn_reads_a_held_slice_again_once_its_interval_passes_and_the_next_flush_writes_it() {
        let (slices, deps, _host) = booted_with_adapters_unread();
        let mut clock = FlushClock::new(HCL_SWITCH_WRITE_INTERVAL);
        slices.take_loaded();
        idle_turn(&deps, &mut clock);
        let early = (unread(&deps), slices.take_loaded());
        assert_eq!(early, (1, vec![]), "the retry did not wait for its interval");

        clock.last_reread = Instant::now() - HELD_SLICE_REREAD_INTERVAL;
        idle_turn(&deps, &mut clock);
        assert_eq!(slices.take_loaded(), vec![SliceKey::Adapters], "only the held slice is read");
        assert_eq!(unread(&deps), 0);
        assert_eq!(adapter_name(&deps.store).as_deref(), Some("Stored Name"));
        assert_eq!(deps.counters.command.slice_reloads.load(Ordering::Relaxed), 1);

        apply_frame_and_flush(rename_adapter("Saved Name"), &deps, &mut clock);
        let written = slices.written();
        assert!(written.contains(&SliceKey::Adapters), "the slice is still held from the flush");
        let rebooted = RegistryStore::with_adapter_count(1);
        rebooted.hydrate_from_store(slices.as_ref(), 1);
        assert_eq!(adapter_name(&rebooted).as_deref(), Some("Saved Name"));

        slices.take_loaded();
        clock.last_reread = Instant::now() - HELD_SLICE_REREAD_INTERVAL;
        idle_turn(&deps, &mut clock);
        assert_eq!(slices.take_loaded(), vec![], "a turn with nothing held read the store");
    }
}
