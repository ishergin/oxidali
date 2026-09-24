use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, PublishResult};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{DaliSettingsChangedEvent, APPLICATION_ACTIVE_MOVED_BY_COMMAND};
use dali2rust_domain::registry::{DaliSettingsReadPort, DaliSettingsView};
use dali2rust_platform::arbitration::ArbitrationReflex;
use dali2rust_platform::liveness::{monotonic_ms, LivenessBeat, LivenessWatch};
use dali2rust_test_support::try_wait_until;
use dali2rust_redundancy_runtime::{
    spawn_arbitration_supervisor, ArbitrationSupervisorConfig, ArbitrationSupervisorCounters,
    SupervisorInputs, LEASE_TTL_MS, SUPERVISOR_HANDLED_EVENTS, WORKER_STALE_AFTER_MS,
};

struct FakeSettings {
    active: AtomicBool,
}

impl DaliSettingsReadPort for FakeSettings {
    fn dali_settings_view(&self) -> DaliSettingsView {
        DaliSettingsView {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: self.active.load(Ordering::Relaxed),
            device_short_address: Some(60),
        }
    }
}

const LONG_PERIOD_MS: u64 = 10_000;
const WAKE_BUDGET: Duration = Duration::from_millis(1_500);

#[test]
fn a_stand_down_clears_the_answer_table_on_the_settings_event_not_the_period() {
    let (host, publisher, ev_rx) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_events(8, SUPERVISOR_HANDLED_EVENTS)
    });
    let settings = Arc::new(FakeSettings {
        active: AtomicBool::new(true),
    });
    let registry = Arc::new(LivenessBeat::new("registry", WORKER_STALE_AFTER_MS));
    registry.beat(monotonic_ms());
    let mut watch = LivenessWatch::new();
    watch.register(Arc::clone(&registry));
    let reflex = Arc::new(ArbitrationReflex::new());
    let counters = Arc::new(ArbitrationSupervisorCounters::default());
    let _supervisor = spawn_arbitration_supervisor(
        ev_rx,
        SupervisorInputs {
            reflex: Arc::clone(&reflex),
            watch,
            settings: Arc::clone(&settings) as Arc<dyn DaliSettingsReadPort>,
            counters: Arc::clone(&counters),
            config: ArbitrationSupervisorConfig {
                period_ms: LONG_PERIOD_MS,
                lease_ttl_ms: LEASE_TTL_MS,
            },
        },
    );
    let armed = try_wait_until(|| reflex.lease_alive(monotonic_ms()), WAKE_BUDGET);
    assert!(armed, "the supervisor never defended an active, healthy controller");

    settings.active.store(false, Ordering::Relaxed);
    let ev = event_envelope(
        dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
        dali2rust_contracts::CORRELATION_NONE,
        1,
        None,
        DaliSettingsChangedEvent {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: false,
            application_active_moved_by: APPLICATION_ACTIVE_MOVED_BY_COMMAND,
        },
    );
    let sent = Instant::now();
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
    let cleared = try_wait_until(|| !reflex.lease_alive(monotonic_ms()), WAKE_BUDGET);
    let took = sent.elapsed();
    assert!(
        cleared,
        "the lease was still alive {took:?} after the stand-down event — the supervisor waited for its period"
    );
    assert!(took < Duration::from_secs(5), "cleared, but only after {took:?}");
    drop(host);
}
