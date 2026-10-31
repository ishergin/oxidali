use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusConfig, BusHost};
use dali2rust_platform::arbitration::AnswerInputs;
use dali2rust_platform::liveness::{monotonic_ms, LivenessBeat, LivenessWatch};
use dali2rust_redundancy_runtime::{
    spawn_arbitration_supervisor, ArbitrationSupervisorConfig, ArbitrationSupervisorCounters,
    SupervisorInputs, LEASE_TTL_MS, SUPERVISOR_HANDLED_EVENTS, WORKER_STALE_AFTER_MS,
};
use dali2rust_test_support::try_wait_until;

const PERIOD_MS: u64 = 50;
const BUDGET: Duration = Duration::from_millis(1_500);

#[test]
fn the_supervisor_renews_on_its_period_and_stops_with_its_host() {
    let (host, _publisher, ev_rx) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_events(8, SUPERVISOR_HANDLED_EVENTS)
    });
    let registry = Arc::new(LivenessBeat::new("registry", WORKER_STALE_AFTER_MS));
    registry.beat(monotonic_ms());
    let mut watch = LivenessWatch::new();
    watch.register(Arc::clone(&registry));
    let inputs = Arc::new(AnswerInputs::new());
    let counters = Arc::new(ArbitrationSupervisorCounters::default());
    let supervisor = spawn_arbitration_supervisor(
        ev_rx,
        SupervisorInputs {
            answer_inputs: Arc::clone(&inputs),
            watch,
            counters: Arc::clone(&counters),
            config: ArbitrationSupervisorConfig {
                period_ms: PERIOD_MS,
                lease_ttl_ms: LEASE_TTL_MS,
            },
        },
    );
    let defended = || counters.defended.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        try_wait_until(|| defended() >= 3, BUDGET),
        "the supervisor defended {} times, not once per period",
        defended()
    );
    assert!(inputs.lease_alive(monotonic_ms()));

    drop(host);
    assert!(
        try_wait_until(|| supervisor.is_finished(), BUDGET),
        "the supervisor outlived its bus host"
    );
}
