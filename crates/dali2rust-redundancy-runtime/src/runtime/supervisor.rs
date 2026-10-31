use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;

use dali2rust_platform::arbitration::AnswerInputs;
use dali2rust_platform::liveness::LivenessWatch;

pub use dali2rust_platform::arbitration::LEASE_TTL_MS;

pub const SUPERVISOR_PERIOD_MS: u64 = 1_000;

pub const WORKER_STALE_AFTER_MS: u32 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorTurn {
    Defend,
    WorkerStale(&'static str),
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
            SupervisorTurn::WorkerStale(_) => &self.stood_down_worker_stale,
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
    pub answer_inputs: Arc<AnswerInputs>,
    pub watch: LivenessWatch,
    pub counters: Arc<ArbitrationSupervisorCounters>,
    pub config: ArbitrationSupervisorConfig,
}

// IEC 62386-103 §9.15
pub fn run_supervisor_tick(inputs: &SupervisorInputs, now_ms: u32) -> SupervisorTurn {
    let turn = match inputs.watch.stale(now_ms) {
        Some(name) => SupervisorTurn::WorkerStale(name),
        None => SupervisorTurn::Defend,
    };
    match turn {
        SupervisorTurn::Defend => {
            inputs.answer_inputs.extend_lease(now_ms, inputs.config.lease_ttl_ms);
        }
        SupervisorTurn::WorkerStale(name) => log_stale_worker(inputs, name, now_ms),
    }
    inputs.counters.record(turn);
    turn
}

fn log_stale_worker(inputs: &SupervisorInputs, name: &str, now_ms: u32) {
    match inputs.watch.stale_age(now_ms) {
        Some((_, None)) => log::info!("arbitration: {name} has not turned yet — the lease waits for it"),
        Some((_, Some(age))) => log::warn!(
            "arbitration: {name} last turned {age} ms ago (stale after {} ms) — the lease is not renewed and lapses in at most {} ms, which reports an application controller error",
            WORKER_STALE_AFTER_MS,
            inputs.config.lease_ttl_ms.min(LEASE_TTL_MS)
        ),
        None => {}
    }
}

pub const SUPERVISOR_HANDLED_EVENTS: &[&str] = &[];

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
    use dali2rust_platform::liveness::LivenessBeat;

    struct Rig {
        inputs: SupervisorInputs,
        registry: Arc<LivenessBeat>,
    }

    fn rig() -> Rig {
        let registry = Arc::new(LivenessBeat::new("registry", WORKER_STALE_AFTER_MS));
        registry.beat(1_000);
        let mut watch = LivenessWatch::new();
        watch.register(Arc::clone(&registry));
        Rig {
            inputs: SupervisorInputs {
                answer_inputs: Arc::new(AnswerInputs::new()),
                watch,
                counters: Arc::new(ArbitrationSupervisorCounters::default()),
                config: ArbitrationSupervisorConfig::default(),
            },
            registry,
        }
    }

    const NOW: u32 = 1_000;

    #[test]
    fn healthy_workers_renew_the_lease() {
        let r = rig();
        assert_eq!(run_supervisor_tick(&r.inputs, NOW), SupervisorTurn::Defend);
        assert!(r.inputs.answer_inputs.lease_alive(NOW));
        assert_eq!(r.inputs.counters.defended.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_wedged_worker_stops_the_renewal_and_the_lease_runs_out() {
        let r = rig();
        run_supervisor_tick(&r.inputs, NOW);
        assert!(r.inputs.answer_inputs.lease_alive(1_000));

        assert_eq!(
            run_supervisor_tick(&r.inputs, 5_000),
            SupervisorTurn::WorkerStale("registry")
        );
        assert!(!r.inputs.answer_inputs.lease_alive(5_000));
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
        let r = rig();
        run_supervisor_tick(&r.inputs, 5_000);
        assert!(!r.inputs.answer_inputs.lease_alive(5_000));

        r.registry.beat(5_100);
        assert_eq!(run_supervisor_tick(&r.inputs, 5_100), SupervisorTurn::Defend);
        assert!(r.inputs.answer_inputs.lease_alive(5_100));
    }

    #[test]
    fn a_worker_that_never_started_is_never_defended_for() {
        let mut watch = LivenessWatch::new();
        watch.register(Arc::new(LivenessBeat::new("ghost", WORKER_STALE_AFTER_MS)));
        let r = rig();
        let inputs = SupervisorInputs { watch, ..r.inputs };
        assert_eq!(
            run_supervisor_tick(&inputs, NOW),
            SupervisorTurn::WorkerStale("ghost")
        );
        assert_eq!(
            inputs.counters.stood_down_worker_stale.load(Ordering::Relaxed),
            1,
            "a boot tick before the worker first turns is counted as a stand-down"
        );
        assert_eq!(inputs.counters.defended.load(Ordering::Relaxed), 0);
    }
}
