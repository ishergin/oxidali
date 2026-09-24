use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;

use dali2rust_domain::dali::dev103::slave::arbitration_slots;
use dali2rust_domain::registry::DaliSettingsReadPort;
use dali2rust_platform::arbitration::ArbitrationReflex;
use dali2rust_platform::liveness::LivenessWatch;

pub const SUPERVISOR_PERIOD_MS: u64 = 1_000;

pub const LEASE_TTL_MS: u32 = 3_000;

pub const WORKER_STALE_AFTER_MS: u32 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandDownReason {
    Passive,
    WorkerStale(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorTurn {
    Defend,
    StandDown(StandDownReason),
}

#[must_use]
pub fn supervisor_turn(
    application_active: bool,
    stale_worker: Option<&'static str>,
) -> SupervisorTurn {
    if !application_active {
        return SupervisorTurn::StandDown(StandDownReason::Passive);
    }
    match stale_worker {
        Some(name) => SupervisorTurn::StandDown(StandDownReason::WorkerStale(name)),
        None => SupervisorTurn::Defend,
    }
}

#[derive(Debug, Default)]
pub struct ArbitrationSupervisorCounters {
    pub defended: AtomicU32,
    pub stood_down_worker_stale: AtomicU32,
}

impl ArbitrationSupervisorCounters {
    fn record(&self, turn: SupervisorTurn) {
        let counter = match turn {
            SupervisorTurn::Defend => &self.defended,
            SupervisorTurn::StandDown(StandDownReason::Passive) => return,
            SupervisorTurn::StandDown(StandDownReason::WorkerStale(_)) => {
                &self.stood_down_worker_stale
            }
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ArbitrationSupervisorConfig {
    pub period_ms: u64,
    pub lease_ttl_ms: u32,
}

impl Default for ArbitrationSupervisorConfig {
    fn default() -> Self {
        Self {
            period_ms: SUPERVISOR_PERIOD_MS,
            lease_ttl_ms: LEASE_TTL_MS,
        }
    }
}

pub struct SupervisorInputs {
    pub reflex: Arc<ArbitrationReflex>,
    pub watch: LivenessWatch,
    pub settings: Arc<dyn DaliSettingsReadPort>,
    pub counters: Arc<ArbitrationSupervisorCounters>,
    pub config: ArbitrationSupervisorConfig,
}

pub fn run_supervisor_tick(inputs: &SupervisorInputs, now_ms: u32) -> SupervisorTurn {
    let settings = inputs.settings.dali_settings_view();
    let turn = supervisor_turn(settings.application_active, inputs.watch.stale(now_ms));
    match turn {
        SupervisorTurn::Defend => {
            publish_slots(
                &inputs.reflex,
                settings.application_active,
                settings.device_short_address,
            );
            inputs.reflex.extend_lease(now_ms, inputs.config.lease_ttl_ms);
        }
        SupervisorTurn::StandDown(StandDownReason::Passive) => {
            inputs.reflex.clear_all();
            inputs.reflex.revoke_lease();
        }
        SupervisorTurn::StandDown(StandDownReason::WorkerStale(name)) => {
            match inputs.watch.stale_age(now_ms) {
                Some((_, None)) => log::info!(
                    "arbitration: {name} has not turned yet — the lease waits for it"
                ),
                Some((_, Some(age))) => log::warn!(
                    "arbitration: {name} last turned {age} ms ago (stale after {} ms) — the answer table is not renewed, the lease lapses in at most {} ms",
                    WORKER_STALE_AFTER_MS,
                    inputs.config.lease_ttl_ms
                ),
                None => {}
            }
        }
    }
    inputs.counters.record(turn);
    turn
}

fn publish_slots(reflex: &ArbitrationReflex, active: bool, short_address: Option<u8>) {
    let slots = arbitration_slots(active, short_address);
    let mut index = 0usize;
    for slot in slots.iter() {
        reflex.set_slot(index, slot.frame, slot.answer);
        index = index.saturating_add(1);
    }
    while index < dali2rust_platform::arbitration::ARBITRATION_REFLEX_SLOTS {
        reflex.clear_slot(index);
        index = index.saturating_add(1);
    }
}

pub const SUPERVISOR_HANDLED_EVENTS: &[&str] = &["DaliSettingsChangedEvent"];

pub fn spawn_arbitration_supervisor(
    ev_rx: dali2rust_bus::BusSubscriberRx,
    inputs: SupervisorInputs,
) -> std::thread::JoinHandle<()> {
    dali2rust_bsp::esp_thread::spawn_named_stack_in(
        c"arb-supervisor",
        dali2rust_bsp::std_thread_stack::ARBITRATION_SUPERVISOR_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        move || supervisor_loop(&ev_rx, &inputs),
    )
}

fn supervisor_loop(ev_rx: &dali2rust_bus::BusSubscriberRx, inputs: &SupervisorInputs) {
    loop {
        run_supervisor_tick(inputs, dali2rust_platform::liveness::monotonic_ms());
        let wait = std::time::Duration::from_millis(inputs.config.period_ms.max(1));
        if let Err(RecvTimeoutError::Disconnected) = ev_rx.recv_timeout(wait) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_domain::registry::DaliSettingsView;
    use dali2rust_platform::liveness::LivenessBeat;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    struct FakeSettings {
        active: AtomicBool,
        short: Mutex<Option<u8>>,
    }

    impl DaliSettingsReadPort for FakeSettings {
        fn dali_settings_view(&self) -> DaliSettingsView {
            DaliSettingsView {
                dt8_auto_activation_repair: false,
                dt8_rgbwaf_control_assert: false,
                application_active: self.active.load(Ordering::Relaxed),
                device_short_address: *self.short.lock().expect("short address"),
            }
        }
    }

    struct Rig {
        inputs: SupervisorInputs,
        settings: Arc<FakeSettings>,
        registry: Arc<LivenessBeat>,
    }

    fn rig(active: bool) -> Rig {
        let settings = Arc::new(FakeSettings {
            active: AtomicBool::new(active),
            short: Mutex::new(Some(7)),
        });
        let registry = Arc::new(LivenessBeat::new("registry", WORKER_STALE_AFTER_MS));
        registry.beat(1_000);
        let mut watch = LivenessWatch::new();
        watch.register(Arc::clone(&registry));
        Rig {
            inputs: SupervisorInputs {
                reflex: Arc::new(ArbitrationReflex::new()),
                watch,
                settings: Arc::clone(&settings) as Arc<dyn DaliSettingsReadPort>,
                counters: Arc::new(ArbitrationSupervisorCounters::default()),
                config: ArbitrationSupervisorConfig::default(),
            },
            settings,
            registry,
        }
    }

    const QUERY_BROADCAST: [u8; 3] = [0xFF, 0xFE, 0x3D];
    const NOW: u32 = 1_000;

    #[test]
    fn an_active_controller_with_healthy_workers_publishes_and_renews() {
        let r = rig(true);
        assert_eq!(run_supervisor_tick(&r.inputs, NOW), SupervisorTurn::Defend);
        assert_eq!(
            r.inputs.reflex.answer_for(QUERY_BROADCAST, 1_000),
            Some(0xFF)
        );
        assert_eq!(r.inputs.counters.defended.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_wedged_worker_stops_the_renewal_and_the_lease_runs_out() {
        let r = rig(true);
        run_supervisor_tick(&r.inputs, NOW);
        assert!(r.inputs.reflex.lease_alive(1_000));

        assert_eq!(
            run_supervisor_tick(&r.inputs, 5_000),
            SupervisorTurn::StandDown(StandDownReason::WorkerStale("registry"))
        );
        assert!(!r.inputs.reflex.lease_alive(5_000));
        assert_eq!(r.inputs.reflex.answer_for(QUERY_BROADCAST, 5_000), None);
        assert_eq!(
            r.inputs
                .counters
                .stood_down_worker_stale
                .load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn a_worker_that_recovers_takes_the_bus_back_with_no_reset() {
        let r = rig(true);
        run_supervisor_tick(&r.inputs, 5_000);
        assert!(!r.inputs.reflex.lease_alive(5_000));

        r.registry.beat(5_100);
        assert_eq!(run_supervisor_tick(&r.inputs, 5_100), SupervisorTurn::Defend);
        assert_eq!(
            r.inputs.reflex.answer_for(QUERY_BROADCAST, 5_100),
            Some(0xFF)
        );
    }

    #[test]
    fn a_passive_controller_publishes_nothing_and_says_passive_not_stale() {
        let r = rig(false);
        assert_eq!(
            run_supervisor_tick(&r.inputs, NOW),
            SupervisorTurn::StandDown(StandDownReason::Passive)
        );
        assert_eq!(r.inputs.reflex.answer_for(QUERY_BROADCAST, 1_000), None);
        assert_eq!(
            r.inputs
                .counters
                .stood_down_worker_stale
                .load(Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn passivity_is_immediate_rather_than_waiting_out_the_lease() {
        let r = rig(true);
        run_supervisor_tick(&r.inputs, NOW);
        r.settings.active.store(false, Ordering::Relaxed);
        run_supervisor_tick(&r.inputs, NOW);
        assert!(!r.inputs.reflex.lease_alive(1_000));
        assert_eq!(r.inputs.reflex.answer_for(QUERY_BROADCAST, 1_000), None);
    }

    #[test]
    fn a_changed_short_address_stops_being_answered_on() {
        let r = rig(true);
        run_supervisor_tick(&r.inputs, NOW);
        let old_addressed = [(7 << 1) | 1, 0xFE, 0x3D];
        assert_eq!(
            r.inputs.reflex.answer_for(old_addressed, 1_000),
            Some(0xFF)
        );

        *r.settings.short.lock().expect("short address") = Some(9);
        run_supervisor_tick(&r.inputs, NOW);
        assert_eq!(r.inputs.reflex.answer_for(old_addressed, 1_000), None);
        assert_eq!(
            r.inputs.reflex.answer_for([(9 << 1) | 1, 0xFE, 0x3D], 1_000),
            Some(0xFF)
        );
    }

    #[test]
    fn a_worker_that_never_started_is_never_defended_for() {
        let mut watch = LivenessWatch::new();
        watch.register(Arc::new(LivenessBeat::new("ghost", WORKER_STALE_AFTER_MS)));
        let r = rig(true);
        let inputs = SupervisorInputs { watch, ..r.inputs };
        assert_eq!(
            run_supervisor_tick(&inputs, NOW),
            SupervisorTurn::StandDown(StandDownReason::WorkerStale("ghost"))
        );
    }

    #[test]
    fn the_pure_turn_checks_passivity_before_health() {
        assert_eq!(
            supervisor_turn(false, Some("registry")),
            SupervisorTurn::StandDown(StandDownReason::Passive)
        );
        assert_eq!(supervisor_turn(true, None), SupervisorTurn::Defend);
    }
}
