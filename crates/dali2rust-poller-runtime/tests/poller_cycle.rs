use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx,
    CorrelationIdAllocator,
};
use dali2rust_contracts::bus::{build_confirmation_envelope_with_product_error, event_envelope};
use dali2rust_contracts::msg::{
    AttributeGroupReadOutcome, BusCommandPayload, BusHealthVerdict, DaliAttributeGroup,
    DaliAttributeReadOutcomesEvent, DaliBusHealthProbeCommand, DaliBusHealthProbedEvent,
    DaliReadAttributesCommand, DeliveryStatus, ErrorCode, MemoryBankReadPreset, Origin,
    PollerSettingsChangedEvent, RegistrySliceReloadedEvent,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{
    PollTargetReadPort, PollTargetSelection, PollTargetView, PollerReadPort,
    PollerSettingsReadPort, PollerSettingsView,
};
use dali2rust_poller_runtime::runtime::power_cycle::RESWEEP_AFTER_YES;
use dali2rust_poller_runtime::{spawn_poller_worker, PollerConfig, PollerCounters, PollerWiring};
use dali2rust_test_support::{remains_false_for, wait_until};

const WAIT: Duration = Duration::from_secs(3);
const BUS_ID: dali2rust_bus::BusId = dali2rust_bus::BusId(7);

#[derive(Default)]
struct StubRegistry {
    devices: Vec<PollTargetView>,
    excluded_unbound: u16,
    settings: PollerSettingsView,
}

impl PollTargetReadPort for StubRegistry {
    fn list_poll_targets(&self, _adapter_id: u8) -> PollTargetSelection {
        PollTargetSelection {
            targets: self.devices.clone(),
            excluded_unbound: self.excluded_unbound,
            adapter_enabled: true,
        }
    }
}

impl PollerSettingsReadPort for StubRegistry {
    fn poller_settings_view(&self) -> PollerSettingsView {
        self.settings
    }
}

fn device(short_address: u8) -> PollTargetView {
    PollTargetView { short_address, is_dt8: false, is_dt6: false, declares_energy: false, declares_diagnostics: false }
}

fn bound_device(short_address: u8, is_dt8: bool) -> PollTargetView {
    PollTargetView { short_address, is_dt8, is_dt6: false, declares_energy: false, declares_diagnostics: false }
}

fn bound_dt6_device(short_address: u8) -> PollTargetView {
    PollTargetView { short_address, is_dt8: false, is_dt6: true, declares_energy: false, declares_diagnostics: false }
}

fn outcomes_event(short_address: u8, outcome: AttributeGroupReadOutcome) -> DaliAttributeReadOutcomesEvent {
    DaliAttributeReadOutcomesEvent {
        registry_adapter_id: 0,
        short_address,
        identity: outcome,
        runtime_status: outcome,
        common_102: outcome,
        dt8_color: outcome,
        dt6_led: outcome,
        groups: outcome,
        scenes: outcome,
        extended: outcome,
        memory_banks: outcome,
        scene_colours: outcome,
    }
}

fn publish_outcomes(publisher: &BusPublisher, correlation_id: u64, target_adapter_id: u16, short_address: u8, outcome: AttributeGroupReadOutcome) {
    let ev = event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, target_adapter_id, Some(Origin::Internal), outcomes_event(short_address, outcome));
    let _ = publisher.try_publish(BusChannel::Events, BusFrame::event(ev));
}

fn publish_health_probed(
    publisher: &BusPublisher,
    control_answered: bool,
    lamp_failure: BusHealthVerdict,
) {
    publish_health_probed_for(publisher, 1, control_answered, lamp_failure);
}

fn publish_health_probed_for(
    publisher: &BusPublisher,
    correlation_id: u64,
    control_answered: bool,
    lamp_failure: BusHealthVerdict,
) {
    publish_probe_verdict(publisher, correlation_id, control_answered, lamp_failure, false);
}

fn publish_probe_verdict(
    publisher: &BusPublisher,
    correlation_id: u64,
    control_answered: bool,
    lamp_failure: BusHealthVerdict,
    power_cycle_seen: bool,
) {
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BusId::default().0,
        Some(Origin::Poller),
        DaliBusHealthProbedEvent {
            registry_adapter_id: 0,
            control_answered,
            lamp_failure,
            power_cycle_seen,
        },
    );
    let _ = publisher.try_publish(BusChannel::Events, BusFrame::event(ev));
}

#[derive(Clone)]
struct CapturedRead {
    correlation_id: u64,
    target_adapter_id: u16,
    command: DaliReadAttributesCommand,
}

struct Harness {
    publisher: BusPublisher,
    counters: Arc<PollerCounters>,
    seen: Arc<Mutex<Vec<CapturedRead>>>,
    held: Arc<Mutex<VecDeque<CapturedRead>>>,
    held_probes: Arc<Mutex<VecDeque<u64>>>,
    seen_probes: Arc<Mutex<Vec<DaliBusHealthProbeCommand>>>,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
    role: Arc<SwitchableRole>,
    _worker: std::thread::JoinHandle<()>,
    _pump: Option<std::thread::JoinHandle<()>>,
    _host: BusHost,
}

impl Harness {
    fn seen_len(&self) -> usize {
        self.seen.lock().expect("seen").len()
    }

    fn seen_all(&self) -> Vec<CapturedRead> {
        self.seen.lock().expect("seen").clone()
    }

    fn held_is_empty(&self) -> bool {
        self.held.lock().expect("held").is_empty()
    }

    fn held_probe_count(&self) -> usize {
        self.held_probes.lock().expect("held_probes").len()
    }

    fn probed_adapters(&self) -> Vec<u8> {
        let probes = self.seen_probes.lock().expect("seen_probes");
        probes.iter().map(|probe| probe.registry_adapter_id).collect()
    }

    fn probes_asking_power_failure(&self) -> Vec<bool> {
        let probes = self.seen_probes.lock().expect("seen_probes");
        probes.iter().map(|probe| probe.query_power_failure).collect()
    }

    fn release_one(&self) {
        let next = self.held.lock().expect("held").pop_front().expect("a held read");
        publish_outcomes(&self.publisher, next.correlation_id, next.target_adapter_id, next.command.short_address, AttributeGroupReadOutcome::Success);
    }

    fn release_probe(&self) {
        let corr = self
            .held_probes
            .lock()
            .expect("held_probes")
            .pop_front()
            .expect("a held probe");
        publish_health_probed_for(&self.publisher, corr, true, BusHealthVerdict::Clear);
    }
}

fn spawn_harness(registry: StubRegistry, owned: bool, auto_respond: Option<AttributeGroupReadOutcome>) -> Harness {
    spawn_harness_with_adapters(registry, owned, auto_respond, 1)
}

fn spawn_harness_with_adapters(
    registry: StubRegistry,
    owned: bool,
    auto_respond: Option<AttributeGroupReadOutcome>,
    adapter_count: u8,
) -> Harness {
    spawn_harness_on(Arc::new(registry), owned, auto_respond, adapter_count)
}

fn spawn_harness_on(
    read_port: Arc<dyn PollerReadPort>,
    owned: bool,
    auto_respond: Option<AttributeGroupReadOutcome>,
    adapter_count: u8,
) -> Harness {
    spawn_harness_configured(read_port, owned, auto_respond, adapter_count, PollerConfig::default())
}

fn spawn_harness_configured(
    read_port: Arc<dyn PollerReadPort>,
    owned: bool,
    auto_respond: Option<AttributeGroupReadOutcome>,
    adapter_count: u8,
    config: PollerConfig,
) -> Harness {
    let (host, publisher, (ev_rx, conf_rx, cmd_rx)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(64, dali2rust_poller_runtime::POLLER_HANDLED_EVENTS),
            reg.subscribe_confirmations(32),
            reg.subscribe_commands(64, if owned { OWNED_COMMANDS } else { &[] }),
        )
    });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let held = Arc::new(Mutex::new(VecDeque::new()));
    let held_probes = Arc::new(Mutex::new(VecDeque::new()));
    let seen_probes = Arc::new(Mutex::new(Vec::new()));
    let pump = owned.then(|| {
        spawn_pump(
            cmd_rx,
            publisher.clone(),
            Arc::clone(&seen),
            Arc::clone(&held),
            Arc::clone(&held_probes),
            Arc::clone(&seen_probes),
            auto_respond,
        )
    });
    let counters = Arc::new(PollerCounters::default());
    let interactive = Arc::new(dali2rust_platform::dali::WireActivity::new());
    let role = Arc::new(SwitchableRole { active: std::sync::atomic::AtomicBool::new(true) });
    let worker = spawn_poller_worker(
        ev_rx,
        conf_rx,
        PollerWiring {
            publisher: publisher.clone(),
            read_port,
            correlation: Arc::new(CorrelationIdAllocator::new()),
            bus_id: BUS_ID,
            adapter_count,
            counters: Arc::clone(&counters),
            role: Arc::clone(&role) as Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
            interactive: Arc::clone(&interactive),
            hold: Arc::new(dali2rust_platform::firmware::MaintenanceHold::new()),
            config,
        },
    );
    Harness { publisher, counters, seen, held, held_probes, seen_probes, interactive, role, _worker: worker, _pump: pump, _host: host }
}

const OWNED_COMMANDS: &[&str] = &["DaliReadAttributesCommand", "DaliBusHealthProbeCommand"];

fn spawn_pump(
    cmd_rx: BusSubscriberRx,
    publisher: BusPublisher,
    seen: Arc<Mutex<Vec<CapturedRead>>>,
    held: Arc<Mutex<VecDeque<CapturedRead>>>,
    held_probes: Arc<Mutex<VecDeque<u64>>>,
    seen_probes: Arc<Mutex<Vec<DaliBusHealthProbeCommand>>>,
    auto_respond: Option<AttributeGroupReadOutcome>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(BusFrame::Command(envelope)) = cmd_rx.recv() {
            if let BusCommandPayload::DaliBusHealthProbeCommand(probe) = &envelope.payload {
                seen_probes.lock().expect("seen_probes").push(probe.clone());
                held_probes
                    .lock()
                    .expect("held_probes")
                    .push_back(envelope.meta.correlation_id);
                continue;
            }
            let BusCommandPayload::DaliReadAttributesCommand(body) = &envelope.payload else {
                continue;
            };
            assert_eq!(
                envelope.meta.target_adapter_id, BUS_ID.0,
                "a read addressed to any other adapter instance is dropped by the DALI worker"
            );
            let captured = CapturedRead {
                correlation_id: envelope.meta.correlation_id,
                target_adapter_id: envelope.meta.target_adapter_id,
                command: body.clone(),
            };
            seen.lock().expect("seen").push(captured.clone());
            match auto_respond {
                Some(outcome) => publish_outcomes(&publisher, captured.correlation_id, captured.target_adapter_id, body.short_address, outcome),
                None => held.lock().expect("held").push_back(captured),
            }
        }
    })
}

#[test]
fn pol_001_only_one_read_is_ever_in_flight() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: (1..=6).map(device).collect(),
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    wait_until(
        || h.counters.reads_published.load(Ordering::Relaxed) == 1 && h.seen_len() == 1,
        WAIT,
    );
    assert_eq!(h.seen_len(), 1, "one read at a time, six due devices notwithstanding");

    for expected in 2..=4 {
        h.release_one();
        wait_until(
            || {
                h.counters.reads_published.load(Ordering::Relaxed) == expected
                    && h.seen_len() == expected as usize
            },
            WAIT,
        );
        assert_eq!(
            h.counters.reads_published.load(Ordering::Relaxed),
            expected,
            "closing one read admits exactly one more"
        );
        assert_eq!(
            h.seen_len(),
            expected as usize,
            "and exactly one more reaches the bus"
        );
    }
}

#[test]
fn pol_003_unroutable_read_retries_next_cycle() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![device(9)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 150,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let generous = Duration::from_secs(5);
    let h = spawn_harness(registry, false, None);

    wait_until(|| h.counters.skipped_inbox_full.load(Ordering::Relaxed) >= 1, generous);
    let rejected_once = h.counters.skipped_inbox_full.load(Ordering::Relaxed);
    let published = h.counters.reads_published.load(Ordering::Relaxed)
        + h.counters.health_probes_published.load(Ordering::Relaxed);
    assert!(
        published >= rejected_once,
        "every rejection was first counted as an accepted publish"
    );
    assert_eq!(h.seen_len(), 0, "nothing ever reached a real owner");

    wait_until(
        || h.counters.skipped_inbox_full.load(Ordering::Relaxed) > rejected_once,
        generous,
    );
}

#[test]
fn pol_004_reads_carry_the_configured_default_groups() {
    let configured =
        DaliAttributeGroup::RuntimeStatus.mask_bit() | DaliAttributeGroup::Common102.mask_bit();
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![bound_device(11, false)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: configured,
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::Success));

    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) >= 1, WAIT);
    let read = h.seen_all().into_iter().next().expect("a published read");
    assert_eq!(read.command.short_address, 11);
    assert_eq!(
        read.command.attribute_groups_mask, configured,
        "the read should carry exactly runtime_status and common_102"
    );
}

#[test]
fn pol_005_dt8_colour_is_added_only_for_dt8_gear() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![bound_device(11, false), bound_device(12, true)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: true,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::Success));

    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) >= 2, WAIT);
    let masks: std::collections::HashMap<u8, u8> = h
        .seen_all()
        .into_iter()
        .map(|read| (read.command.short_address, read.command.attribute_groups_mask))
        .collect();
    assert_eq!(
        masks.get(&11),
        Some(&DaliAttributeGroup::RuntimeStatus.mask_bit()),
        "dt6 gear has no colour to read, so the group must not be added"
    );
    assert_eq!(
        masks.get(&12),
        Some(&(DaliAttributeGroup::RuntimeStatus.mask_bit() | DaliAttributeGroup::Dt8Color.mask_bit())),
        "dt8 gear gets the colour group on top of the configured default"
    );
}

#[test]
fn dt6_led_is_stripped_for_gear_that_is_not_dt6() {
    let configured =
        DaliAttributeGroup::RuntimeStatus.mask_bit() | DaliAttributeGroup::Dt6Led.mask_bit();
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![
            bound_dt6_device(11),
            bound_device(12, true),
            bound_device(13, false),
        ],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: configured,
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::Success));

    wait_until(
        || h.counters.reads_completed.load(Ordering::Relaxed) >= 3,
        Duration::from_secs(8),
    );
    let masks: std::collections::HashMap<u8, u8> = h
        .seen_all()
        .into_iter()
        .map(|read| (read.command.short_address, read.command.attribute_groups_mask))
        .collect();
    assert_eq!(
        masks.get(&11),
        Some(&configured),
        "dt6 gear keeps the configured dt6_led group"
    );
    let stripped = DaliAttributeGroup::RuntimeStatus.mask_bit();
    assert_eq!(
        masks.get(&12),
        Some(&stripped),
        "dt8 gear must not be swept with 19 unanswerable DT6 queries"
    );
    assert_eq!(
        masks.get(&13),
        Some(&stripped),
        "unknown type counts as not-DT6, exactly like the dt8 gate"
    );
}

#[test]
fn an_absent_verdict_counts_reads_absent_not_reads_failed() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![bound_device(21, false)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::DeviceAbsent));

    wait_until(|| h.counters.reads_absent.load(Ordering::Relaxed) >= 1, WAIT);
    assert_eq!(
        h.counters.reads_failed.load(Ordering::Relaxed),
        0,
        "an answered question — nothing is here — is not a wire fault"
    );
    assert!(h.counters.device_cooldowns.load(Ordering::Relaxed) >= 1);
}

#[test]
fn targets_excluded_gauge_mirrors_the_ports_exclusion_count() {
    let registry = StubRegistry {
        excluded_unbound: 3,
        devices: vec![bound_device(11, false)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::Success));
    wait_until(
        || h.counters.targets_excluded.load(Ordering::Relaxed) == 3,
        WAIT,
    );
}

#[test]
fn window_deferral_is_one_per_cycle_not_one_per_leftover_device() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![
            bound_device(11, false),
            bound_device(12, false),
            bound_device(13, false),
            bound_device(14, false),
        ],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 200,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);
    wait_until(
        || h.counters.window_deferred.load(Ordering::Relaxed) >= 2,
        WAIT,
    );
    let cycles = h.counters.cycles_total.load(Ordering::Relaxed);
    let deferred = h.counters.window_deferred.load(Ordering::Relaxed);
    assert!(
        deferred <= cycles,
        "episode semantics: at most one deferral per cycle boundary \
         (deferred={deferred}, cycles={cycles})"
    );
}

#[test]
fn pol_007_never_requests_identity_or_profile_banks() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![bound_device(4, false)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit() | DaliAttributeGroup::Extended.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::Success));

    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) >= 1, WAIT);
    let reads = h.seen_all();
    assert!(!reads.is_empty());
    for read in reads {
        assert_eq!(read.command.memory_banks, MemoryBankReadPreset::None);
    }
}

#[test]
fn pol_009_failed_read_skips_the_next_two_cycles() {
    let interval_ms = 150;
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![bound_device(11, false)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::TransportAbort));

    wait_until(|| h.counters.device_cooldowns.load(Ordering::Relaxed) == 1, WAIT);
    let published_at_failure = h.counters.reads_published.load(Ordering::Relaxed);
    assert_eq!(published_at_failure, 1);

    wait_for_cycles(&h, 2);
    assert_eq!(
        h.counters.reads_published.load(Ordering::Relaxed),
        published_at_failure,
        "the device stays skipped for the next 2 cycles"
    );

    wait_for_cycles(&h, 1);
    wait_until(
        || h.counters.reads_published.load(Ordering::Relaxed) == published_at_failure + 1,
        WAIT,
    );
}

fn wait_for_cycles(h: &Harness, extra: u32) {
    let target = h.counters.cycles_total.load(Ordering::Relaxed) + extra;
    wait_until(|| h.counters.cycles_total.load(Ordering::Relaxed) >= target, WAIT);
}

#[test]
fn pol_010_a_preempted_read_is_retried_without_a_cooldown() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![device(4)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 150,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::Preempted));

    wait_until(|| h.counters.reads_preempted.load(Ordering::Relaxed) >= 1, WAIT);
    let preempted = h.counters.reads_preempted.load(Ordering::Relaxed);
    assert_eq!(
        h.counters.reads_failed.load(Ordering::Relaxed),
        0,
        "standing down is not a read failure"
    );
    assert_eq!(
        h.counters.device_cooldowns.load(Ordering::Relaxed),
        0,
        "standing down must not cool the device down"
    );
    assert_eq!(
        h.counters.reads_completed.load(Ordering::Relaxed),
        0,
        "and it is not a success either"
    );

    wait_until(
        || h.counters.reads_preempted.load(Ordering::Relaxed) > preempted,
        WAIT,
    );
    assert!(h
        .seen_all()
        .iter()
        .all(|r| r.command.short_address == 4));
}

#[test]
fn pol_011_an_expensive_read_buys_itself_silence() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: (1..=4).map(device).collect(),
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 200,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    wait_until(|| h.counters.reads_published.load(Ordering::Relaxed) == 1, WAIT);
    let published_two = || h.counters.reads_published.load(Ordering::Relaxed) >= 2;
    let cost = Duration::from_millis(300);
    assert!(remains_false_for(published_two, cost));
    h.release_one();

    wait_until(|| h.counters.duty_deferred.load(Ordering::Relaxed) >= 1, WAIT);
    assert!(
        remains_false_for(published_two, Duration::from_millis(300)),
        "a read that cost {cost:?} owes ~3x that in silence; duty_deferred={}",
        h.counters.duty_deferred.load(Ordering::Relaxed)
    );

    wait_until(published_two, WAIT);
}


#[test]
fn a_rest_window_with_nothing_due_is_not_a_deferral() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![device(1)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    wait_until(|| h.counters.reads_published.load(Ordering::Relaxed) == 1, WAIT);
    let cost = Duration::from_millis(300);
    assert!(remains_false_for(|| h.seen_len() > 1, cost));
    h.release_one();
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) >= 1, WAIT);

    for _ in 0..10 {
        publish_outcomes(&h.publisher, u64::MAX, BUS_ID.0, 1, AttributeGroupReadOutcome::Success);
    }
    assert!(
        remains_false_for(
            || h.counters.duty_deferred.load(Ordering::Relaxed) > 0,
            Duration::from_millis(300)
        ),
        "nothing was due, so the budget held nothing back"
    );
}

#[test]
fn a_read_the_budget_holds_back_counts_once_not_once_per_wake() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![device(1), device(2)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    wait_until(|| h.counters.reads_published.load(Ordering::Relaxed) == 1, WAIT);
    let cost = Duration::from_millis(300);
    assert!(remains_false_for(|| h.seen_len() > 1, cost));
    h.release_one();

    wait_until(|| h.counters.duty_deferred.load(Ordering::Relaxed) >= 1, WAIT);
    for _ in 0..10 {
        publish_outcomes(&h.publisher, u64::MAX, BUS_ID.0, 2, AttributeGroupReadOutcome::Success);
    }
    assert!(
        remains_false_for(
            || h.counters.duty_deferred.load(Ordering::Relaxed) > 1,
            Duration::from_millis(300)
        ),
        "one held-back read is one deferral, not one per wake-up"
    );

    wait_until(|| h.seen_len() >= 2, WAIT);
    assert_eq!(
        h.seen_all()[1].command.short_address,
        2,
        "the read the budget deferred is the one that goes out next"
    );
}

#[test]
fn pol_012_a_queue_that_cannot_drain_still_reaches_every_device() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: (1..=4).map(device).collect(),
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 200,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    for _ in 0..4 {
        wait_until(|| !h.held_is_empty(), WAIT);
        h.release_one();
    }
    wait_until(|| h.seen_len() >= 4, WAIT);

    let addressed: std::collections::BTreeSet<u8> =
        h.seen_all().iter().map(|r| r.command.short_address).collect();
    assert_eq!(
        addressed,
        (1..=4).collect::<std::collections::BTreeSet<u8>>(),
        "every device must get a turn: {:?}",
        h.seen_all().iter().map(|r| r.command.short_address).collect::<Vec<_>>()
    );
}

#[test]
fn pol_032_each_cycle_publishes_a_broadcast_health_probe() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: (1..=3).map(device).collect(),
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    wait_until(
        || h.counters.health_probes_published.load(Ordering::Relaxed) >= 1,
        WAIT,
    );
    wait_until(|| h.counters.reads_published.load(Ordering::Relaxed) >= 1, WAIT);
    assert!(h.counters.reads_published.load(Ordering::Relaxed) >= 1);
}

const SHORT_PROBE_PERIOD: Duration = Duration::from_millis(100);
const NO_PROBE_PERIOD: Duration = Duration::from_secs(3_600);

fn poller_off_registry(devices: Vec<PollTargetView>, power_cycle_watch: bool) -> StubRegistry {
    StubRegistry {
        excluded_unbound: 0,
        devices,
        settings: PollerSettingsView {
            enabled: false,
            interval_ms: 50,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch,
        },
    }
}

fn spawn_watch_harness(
    devices: Vec<PollTargetView>,
    auto_respond: Option<AttributeGroupReadOutcome>,
    probe_period: Duration,
) -> Harness {
    let port = Arc::new(poller_off_registry(devices, true));
    spawn_harness_configured(port, true, auto_respond, 1, PollerConfig { probe_period })
}

#[test]
fn pol_033_a_disabled_poller_publishes_no_probe() {
    let port = Arc::new(poller_off_registry(vec![device(1)], false));
    let config = PollerConfig { probe_period: SHORT_PROBE_PERIOD };
    let h = spawn_harness_configured(port, true, None, 1, config);

    wait_until(|| h.counters.cycles_total.load(Ordering::Relaxed) >= 2, WAIT);
    assert_eq!(h.counters.health_probes_published.load(Ordering::Relaxed), 0);
}

#[test]
fn a_watch_probes_on_its_own_period_with_the_poller_off_and_reads_nothing() {
    let h = spawn_watch_harness(vec![device(1)], None, SHORT_PROBE_PERIOD);

    for expected in 1..=2 {
        wait_until(|| h.held_probe_count() >= 1, WAIT);
        h.release_probe();
        wait_until(
            || h.counters.health_probes_clear.load(Ordering::Relaxed) == expected,
            WAIT,
        );
    }
    assert_eq!(
        h.counters.reads_published.load(Ordering::Relaxed),
        0,
        "the watch probes with the poller off; the cycle reads stay off"
    );
    assert!(h.probes_asking_power_failure().iter().all(|asks| *asks));
}

fn publish_settings_changed(publisher: &BusPublisher) {
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        0,
        BusId::default().0,
        Some(Origin::Registry),
        PollerSettingsChangedEvent {
            enabled: false,
            interval_ms: 0,
            attribute_groups_mask: 0,
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
        },
    );
    let _ = publisher.try_publish(BusChannel::Events, BusFrame::event(ev));
}

fn switchable_watch_harness(enabled: bool, power_cycle_watch: bool) -> (Arc<ReplicatedSettings>, Harness) {
    let mut settings = disabled_fast_settings();
    settings.enabled = enabled;
    settings.power_cycle_watch = power_cycle_watch;
    let port = Arc::new(ReplicatedSettings {
        devices: Vec::new(),
        settings: Mutex::new(settings),
    });
    let config = PollerConfig { probe_period: SHORT_PROBE_PERIOD };
    let h = spawn_harness_configured(Arc::clone(&port) as Arc<dyn PollerReadPort>, true, None, 1, config);
    (port, h)
}

#[test]
fn the_probe_asks_for_power_failures_only_while_the_watch_is_on() {
    let (port, h) = switchable_watch_harness(true, false);
    wait_until(|| h.held_probe_count() >= 1, WAIT);
    port.settings.lock().expect("settings").power_cycle_watch = true;
    publish_settings_changed(&h.publisher);
    while !h.probes_asking_power_failure().last().is_some_and(|asks| *asks) {
        wait_until(|| h.held_probe_count() >= 1, WAIT);
        h.release_probe();
        wait_until(|| h.held_probe_count() >= 1, WAIT);
    }
    let asked = h.probes_asking_power_failure();
    assert!(!asked[0], "with the watch off the enabled poller's probe is two frames: {asked:?}");
}

#[test]
fn a_watch_switched_off_stops_probing_with_the_poller_off() {
    let (port, h) = switchable_watch_harness(false, true);
    wait_until(|| h.held_probe_count() >= 1, WAIT);
    port.settings.lock().expect("settings").power_cycle_watch = false;
    publish_settings_changed(&h.publisher);
    h.release_probe();
    wait_until(|| h.counters.health_probes_clear.load(Ordering::Relaxed) == 1, WAIT);
    assert!(
        remains_false_for(|| h.held_probe_count() > 0, SHORT_PROBE_PERIOD * 5),
        "the changed event wakes the poller, which reads the switch from the registry"
    );
}

fn yes_until(h: &Harness, power_failures: u32) {
    while h.counters.health_probes_power_failure.load(Ordering::Relaxed) < power_failures {
        let before = h.counters.health_probes_power_failure.load(Ordering::Relaxed);
        publish_probe_verdict(&h.publisher, 0, true, BusHealthVerdict::Clear, true);
        wait_until(
            || h.counters.health_probes_power_failure.load(Ordering::Relaxed) > before,
            WAIT,
        );
    }
}

fn sweeps(h: &Harness) -> u32 {
    h.counters.power_cycle_sweeps.load(Ordering::Relaxed)
}

fn sweep_reads(h: &Harness) -> u32 {
    h.counters.power_cycle_sweep_reads.load(Ordering::Relaxed)
}

#[test]
fn a_yes_behind_its_control_reads_every_target_once_with_the_poller_off() {
    let h = spawn_watch_harness(vec![device(1), device(2)], Some(AttributeGroupReadOutcome::Success), NO_PROBE_PERIOD);

    yes_until(&h, 1);
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) == 2, WAIT);
    assert_eq!(sweeps(&h), 1);
    assert_eq!(sweep_reads(&h), 2);
    assert_eq!(
        h.counters.reads_published.load(Ordering::Relaxed),
        sweep_reads(&h),
        "every read a disabled poller publishes is a sweep read"
    );
    let reads = h.seen_all();
    let shorts: Vec<u8> = reads.iter().map(|r| r.command.short_address).collect();
    assert_eq!(shorts, vec![1, 2], "one read per poll target");
    for read in &reads {
        assert_eq!(
            read.command.attribute_groups_mask,
            DaliAttributeGroup::RuntimeStatus.mask_bit(),
            "a sweep asks for the status byte and nothing else"
        );
        assert_eq!(read.command.memory_banks, MemoryBankReadPreset::None);
    }
}

#[test]
fn a_sweep_read_is_counted_apart_from_the_cycle_reads() {
    let h = slow_cycle_harness(DaliAttributeGroup::Groups.mask_bit());
    yes_until(&h, 1);
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) == 2, WAIT);
    assert_eq!(sweep_reads(&h), 1);
    assert_eq!(h.counters.reads_published.load(Ordering::Relaxed), 2, "a sweep read is still a published read");
}

fn slow_cycle_harness(attribute_groups_mask: u8) -> Harness {
    let mut registry = poller_off_registry(vec![device(1)], true);
    registry.settings.enabled = true;
    registry.settings.interval_ms = 100_000;
    registry.settings.attribute_groups_mask = attribute_groups_mask;
    let config = PollerConfig { probe_period: NO_PROBE_PERIOD };
    let h = spawn_harness_configured(Arc::new(registry), true, Some(AttributeGroupReadOutcome::Success), 1, config);
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) == 1, WAIT);
    assert_eq!(sweep_reads(&h), 0, "a cycle read is no sweep read");
    h
}

#[test]
fn a_cycle_that_reads_the_status_leaves_a_yes_nothing_to_sweep() {
    let h = slow_cycle_harness(DaliAttributeGroup::RuntimeStatus.mask_bit());
    yes_until(&h, 1);
    assert_eq!(sweeps(&h), 0, "every cycle reads each target's status already; a sweep would read it twice");
}

#[test]
fn a_cycle_switched_on_to_read_the_status_takes_over_a_sweep_in_progress() {
    let port = Arc::new(ReplicatedSettings {
        devices: vec![device(1), device(2)],
        settings: Mutex::new(PollerSettingsView { power_cycle_watch: true, ..disabled_fast_settings() }),
    });
    let config = PollerConfig { probe_period: NO_PROBE_PERIOD };
    let h = spawn_harness_configured(Arc::clone(&port) as Arc<dyn PollerReadPort>, true, None, 1, config);
    yes_until(&h, 1);
    wait_until(|| h.seen_len() == 1, WAIT);
    port.settings.lock().expect("settings").enabled = true;
    publish_settings_changed(&h.publisher);
    wait_until(|| h.counters.health_probes_published.load(Ordering::Relaxed) >= 1, WAIT);
    release_with(&h, AttributeGroupReadOutcome::Preempted);
    wait_until(|| h.seen_len() == 2, HELD_READ_REST);
    assert_eq!(
        sweep_reads(&h),
        1,
        "the cycle now reads every status, so neither the queued sweep target nor the preempted one is read as a sweep"
    );
}

#[test]
fn a_yes_without_its_control_reads_nothing_and_leaves_the_hold_off_alone() {
    let h = spawn_watch_harness(vec![device(1)], Some(AttributeGroupReadOutcome::Success), NO_PROBE_PERIOD);

    publish_probe_verdict(&h.publisher, 0, false, BusHealthVerdict::Clear, true);
    wait_until(|| h.counters.health_probes_invalid.load(Ordering::Relaxed) == 1, WAIT);
    assert_eq!(sweeps(&h), 0, "a YES whose control went unanswered proves nothing");

    yes_until(&h, 1);
    assert_eq!(sweeps(&h), 1);
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) == 1, WAIT);
    publish_probe_verdict(&h.publisher, 0, false, BusHealthVerdict::Clear, false);
    wait_until(|| h.counters.health_probes_invalid.load(Ordering::Relaxed) == 2, WAIT);
    yes_until(&h, 2);
    assert_eq!(sweeps(&h), 1, "a silent probe without its control is no NO, so the hold-off stands");

    publish_probe_verdict(&h.publisher, 0, true, BusHealthVerdict::Clear, false);
    yes_until(&h, 3);
    assert_eq!(sweeps(&h), 2, "a NO behind an answered control re-arms the watch");
}

#[test]
fn a_standing_yes_sweeps_again_only_after_its_hold_off() {
    let h = spawn_watch_harness(vec![device(1)], Some(AttributeGroupReadOutcome::Success), NO_PROBE_PERIOD);

    yes_until(&h, 1);
    assert_eq!(sweeps(&h), 1);
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) == 1, WAIT);
    yes_until(&h, RESWEEP_AFTER_YES);
    assert_eq!(
        sweeps(&h),
        1,
        "a gear that keeps its bit keeps the answer YES; that is not a new power cycle"
    );
    yes_until(&h, RESWEEP_AFTER_YES + 1);
    assert_eq!(
        sweeps(&h),
        2,
        "after {RESWEEP_AFTER_YES} further YES answers the sweep runs again, so a gear driven \
         and cycled again behind a standing YES is still found"
    );
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) == 2, WAIT);
}

#[test]
fn a_preempted_sweep_read_is_retried_with_the_poller_off() {
    let h = spawn_watch_harness(vec![device(4)], None, NO_PROBE_PERIOD);

    yes_until(&h, 1);
    wait_until(|| h.seen_len() == 1, WAIT);
    let first = h.held.lock().expect("held").pop_front().expect("the sweep read");
    publish_outcomes(&h.publisher, first.correlation_id, first.target_adapter_id, 4, AttributeGroupReadOutcome::Preempted);
    wait_until(|| h.seen_len() == 2, WAIT);
    assert_eq!(
        h.seen_all()[1].command.short_address,
        4,
        "a preempted sweep read goes back to the sweep, not to the switched-off cycle queue"
    );
}

fn release_with(h: &Harness, outcome: AttributeGroupReadOutcome) -> u8 {
    let read = h.held.lock().expect("held").pop_front().expect("a held read");
    let short = read.command.short_address;
    publish_outcomes(&h.publisher, read.correlation_id, read.target_adapter_id, short, outcome);
    short
}

fn read_shorts(h: &Harness) -> Vec<u8> {
    h.seen_all().iter().map(|read| read.command.short_address).collect()
}

fn no(h: &Harness) {
    let before = h.counters.health_probes_clear.load(Ordering::Relaxed);
    publish_probe_verdict(&h.publisher, 0, true, BusHealthVerdict::Clear, false);
    wait_until(|| h.counters.health_probes_clear.load(Ordering::Relaxed) > before, WAIT);
}

const QUIET_SPELL: Duration = Duration::from_millis(300);

#[test]
fn a_failed_sweep_read_is_read_again_on_the_next_yes() {
    let h = spawn_watch_harness(vec![device(1)], None, NO_PROBE_PERIOD);

    yes_until(&h, 1);
    wait_until(|| h.seen_len() == 1, WAIT);
    release_with(&h, AttributeGroupReadOutcome::ContendedAbort);
    wait_until(|| h.counters.reads_failed.load(Ordering::Relaxed) == 1, WAIT);
    assert!(
        remains_false_for(|| h.seen_len() > 1, QUIET_SPELL),
        "a failed sweep read is retried by the next YES, not at once"
    );
    yes_until(&h, 2);
    wait_until(|| h.seen_len() == 2, WAIT);
    assert_eq!(read_shorts(&h), vec![1, 1], "the target the failed read still owes is read again");
    assert_eq!(sweeps(&h), 1, "reading what a sweep still owes is the same sweep");
}

#[test]
fn a_stand_down_mid_sweep_leaves_the_watch_able_to_sweep_on_the_next_yes() {
    let h = spawn_watch_harness(vec![device(1), device(2)], None, NO_PROBE_PERIOD);

    yes_until(&h, 1);
    wait_until(|| h.seen_len() == 1, WAIT);
    h.role.active.store(false, Ordering::Relaxed);
    let cycles = h.counters.cycles_total.load(Ordering::Relaxed);
    wait_until(|| h.counters.cycles_total.load(Ordering::Relaxed) >= cycles + 2, WAIT);
    release_with(&h, AttributeGroupReadOutcome::Success);
    h.role.active.store(true, Ordering::Relaxed);
    yes_until(&h, 2);
    wait_until(|| h.seen_len() == 2, WAIT);
    release_with(&h, AttributeGroupReadOutcome::Success);
    wait_until(|| h.seen_len() == 3, WAIT);
    assert_eq!(
        read_shorts(&h),
        vec![1, 1, 2],
        "the stand-down dropped the sweep, so the next YES sweeps every target afresh"
    );
}

#[test]
fn yes_no_yes_while_a_sweep_is_queued_reads_each_target_once() {
    let h = spawn_watch_harness(vec![device(1), device(2)], None, NO_PROBE_PERIOD);

    yes_until(&h, 1);
    wait_until(|| h.seen_len() == 1, WAIT);
    no(&h);
    yes_until(&h, 2);
    release_with(&h, AttributeGroupReadOutcome::Success);
    wait_until(|| h.seen_len() == 2, WAIT);
    release_with(&h, AttributeGroupReadOutcome::Success);
    assert!(
        remains_false_for(|| h.seen_len() > 2, QUIET_SPELL),
        "a target already queued or in flight is not queued again: {:?}",
        read_shorts(&h)
    );
    assert_eq!(read_shorts(&h), vec![1, 2]);
}

fn end_held_probe_without_a_verdict(h: &Harness, cause: (ErrorCode, &str)) {
    let correlation_id = h.held_probes.lock().expect("held_probes").pop_front().expect("a held probe");
    let ended = build_confirmation_envelope_with_product_error(
        correlation_id,
        DeliveryStatus::ExecutionFailed,
        0,
        SOURCE_ID_UNSPECIFIED,
        Some(cause),
    );
    let _ = h.publisher.try_publish(BusChannel::Confirmations, BusFrame::confirmation(ended));
}

#[test]
fn a_probe_that_ends_without_a_verdict_frees_the_probe_slot_and_leaves_the_watch_alone() {
    let h = spawn_watch_harness(vec![device(1)], Some(AttributeGroupReadOutcome::Success), SHORT_PROBE_PERIOD);

    for cause in [
        (ErrorCode::Preempted, "preempted"),
        (ErrorCode::OperationFailed, "dali_transport_error"),
        (ErrorCode::Conflict, "controller_passive"),
    ] {
        wait_until(|| h.held_probe_count() >= 1, WAIT);
        end_held_probe_without_a_verdict(&h, cause);
    }
    wait_until(|| h.held_probe_count() >= 1, WAIT);
    let verdicts = [
        &h.counters.health_probes_clear,
        &h.counters.health_probes_invalid,
        &h.counters.health_probes_one_failure,
        &h.counters.health_probes_several_failures,
        &h.counters.health_probes_power_failure,
        &h.counters.power_cycle_sweeps,
    ];
    assert!(
        verdicts.iter().all(|counter| counter.load(Ordering::Relaxed) == 0),
        "a probe that ended without a verdict counts as none"
    );
    assert_eq!(h.counters.health_probes_expired.load(Ordering::Relaxed), 0);
}

struct ReplicatedSettings {
    devices: Vec<PollTargetView>,
    settings: Mutex<PollerSettingsView>,
}

impl PollTargetReadPort for ReplicatedSettings {
    fn list_poll_targets(&self, _adapter_id: u8) -> PollTargetSelection {
        PollTargetSelection { targets: self.devices.clone(), excluded_unbound: 0, adapter_enabled: true }
    }
}

impl PollerSettingsReadPort for ReplicatedSettings {
    fn poller_settings_view(&self) -> PollerSettingsView {
        *self.settings.lock().expect("settings")
    }
}

fn disabled_fast_settings() -> PollerSettingsView {
    PollerSettingsView {
        enabled: false,
        interval_ms: 50,
        attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
        include_dt8_color: false,
        include_energy: false,
        include_diagnostics: false,
        skip_unbound_virtual_lamps: false,
        power_cycle_watch: false,
    }
}

fn replicated_harness() -> (Arc<ReplicatedSettings>, Harness) {
    let port = Arc::new(ReplicatedSettings {
        devices: vec![device(1)],
        settings: Mutex::new(disabled_fast_settings()),
    });
    let h = spawn_harness_on(Arc::clone(&port) as Arc<dyn PollerReadPort>, true, None, 1);
    wait_until(|| h.counters.cycles_total.load(Ordering::Relaxed) >= 2, WAIT);
    port.settings.lock().expect("settings").enabled = true;
    (port, h)
}

fn publish_slice_reloaded(publisher: &BusPublisher, slice_name: &str) {
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        0,
        BusId::default().0,
        Some(Origin::Registry),
        RegistrySliceReloadedEvent {
            slice_name: dali2rust_contracts::msg::fixed_text_32(slice_name),
        },
    );
    let _ = publisher.try_publish(BusChannel::Events, BusFrame::event(ev));
}

#[test]
fn replicated_settings_apply_on_the_slice_reload() {
    let (_port, h) = replicated_harness();
    publish_slice_reloaded(&h.publisher, "physical_devices_b0+5");
    wait_until(
        || h.counters.health_probes_published.load(Ordering::Relaxed) >= 1,
        WAIT,
    );
}

#[test]
fn replicated_settings_wait_for_the_reload_that_announces_them() {
    let (_port, h) = replicated_harness();
    assert!(remains_false_for(
        || h.counters.health_probes_published.load(Ordering::Relaxed) > 0,
        Duration::from_millis(400),
    ));
}

struct AdapterGate {
    enabled: bool,
}

impl PollTargetReadPort for AdapterGate {
    fn list_poll_targets(&self, _adapter_id: u8) -> PollTargetSelection {
        let targets = if self.enabled { vec![device(1)] } else { Vec::new() };
        PollTargetSelection { targets, excluded_unbound: 0, adapter_enabled: self.enabled }
    }
}

impl PollerSettingsReadPort for AdapterGate {
    fn poller_settings_view(&self) -> PollerSettingsView {
        PollerSettingsView { enabled: true, ..disabled_fast_settings() }
    }
}

fn adapter_gate_harness(enabled: bool) -> Harness {
    let h = spawn_harness_on(Arc::new(AdapterGate { enabled }), true, None, 1);
    wait_until(|| h.counters.cycles_total.load(Ordering::Relaxed) >= 3, WAIT);
    h
}

#[test]
fn a_disabled_adapter_is_neither_read_nor_probed() {
    let control = adapter_gate_harness(true);
    wait_until(
        || {
            control.counters.health_probes_published.load(Ordering::Relaxed) >= 1
                && control.counters.reads_published.load(Ordering::Relaxed) >= 1
        },
        WAIT,
    );
    let disabled = adapter_gate_harness(false);
    assert_eq!(disabled.counters.health_probes_published.load(Ordering::Relaxed), 0);
    assert_eq!(disabled.counters.reads_published.load(Ordering::Relaxed), 0);
}

struct AdapterZeroSwitch {
    enabled: AtomicBool,
}

impl PollTargetReadPort for AdapterZeroSwitch {
    fn list_poll_targets(&self, adapter_id: u8) -> PollTargetSelection {
        let enabled = adapter_id != 0 || self.enabled.load(Ordering::Relaxed);
        let targets = if enabled { vec![device(1), device(2)] } else { Vec::new() };
        PollTargetSelection { targets, excluded_unbound: 0, adapter_enabled: enabled }
    }
}

impl PollerSettingsReadPort for AdapterZeroSwitch {
    fn poller_settings_view(&self) -> PollerSettingsView {
        PollerSettingsView {
            enabled: true,
            attribute_groups_mask: DaliAttributeGroup::Groups.mask_bit(),
            power_cycle_watch: true,
            ..disabled_fast_settings()
        }
    }
}

fn adapter_zero_switch_harness(auto_respond: Option<AttributeGroupReadOutcome>) -> (Arc<AdapterZeroSwitch>, Harness) {
    let gate = Arc::new(AdapterZeroSwitch { enabled: AtomicBool::new(true) });
    let config = PollerConfig { probe_period: NO_PROBE_PERIOD };
    let h = spawn_harness_configured(Arc::clone(&gate) as Arc<dyn PollerReadPort>, true, auto_respond, 2, config);
    (gate, h)
}

fn disable_adapter_zero_for_a_cycle(gate: &AdapterZeroSwitch, h: &Harness) {
    gate.enabled.store(false, Ordering::Relaxed);
    let cycles = h.counters.cycles_total.load(Ordering::Relaxed);
    wait_until(|| h.counters.cycles_total.load(Ordering::Relaxed) >= cycles + 2, WAIT);
}

const HELD_READ_REST: Duration = Duration::from_secs(10);

#[test]
fn a_sweep_owed_by_an_adapter_disabled_mid_sweep_gives_the_read_slot_up() {
    let (gate, h) = adapter_zero_switch_harness(None);
    wait_until(|| h.seen_len() == 1, WAIT);
    yes_until(&h, 1);
    assert_eq!(sweeps(&h), 1, "adapter 0 owes a sweep of gear 1 and 2 behind its cycle read");
    disable_adapter_zero_for_a_cycle(&gate, &h);
    release_with(&h, AttributeGroupReadOutcome::Success);
    for expected in [2, 3] {
        wait_until(|| h.seen_len() == expected, HELD_READ_REST);
        let read = h.seen_all()[expected - 1].command.clone();
        assert_eq!(
            (read.registry_adapter_id, read.short_address, read.attribute_groups_mask),
            (1, (expected - 1) as u8, DaliAttributeGroup::Groups.mask_bit()),
            "the slot goes to adapter 1's cycle reads, not to a sweep a disabled adapter refuses unanswered"
        );
        release_with(&h, AttributeGroupReadOutcome::Success);
    }
    assert_eq!(sweep_reads(&h), 0);
}

#[test]
fn an_adapter_disabled_and_enabled_again_sweeps_on_its_first_yes() {
    let (gate, h) = adapter_zero_switch_harness(Some(AttributeGroupReadOutcome::Success));
    yes_until(&h, 1);
    wait_until(|| sweep_reads(&h) == 2, WAIT);
    let published = h.counters.reads_published.load(Ordering::Relaxed);
    wait_until(|| h.counters.reads_published.load(Ordering::Relaxed) > published, WAIT);
    yes_until(&h, 2);
    assert_eq!(sweeps(&h), 1, "control: a settled sweep holds a standing YES off");
    disable_adapter_zero_for_a_cycle(&gate, &h);
    gate.enabled.store(true, Ordering::Relaxed);
    yes_until(&h, 3);
    assert_eq!(
        sweeps(&h),
        2,
        "nothing watched the adapter while it was disabled, so its first YES afterwards sweeps afresh"
    );
}

#[test]
fn pol_034_a_probe_without_its_control_is_void_not_clear() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![device(1)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    publish_health_probed(&h.publisher, false, BusHealthVerdict::Clear);
    wait_until(
        || h.counters.health_probes_invalid.load(Ordering::Relaxed) == 1,
        WAIT,
    );
    assert_eq!(h.counters.health_probes_clear.load(Ordering::Relaxed), 0);

    publish_health_probed(&h.publisher, true, BusHealthVerdict::Clear);
    wait_until(
        || h.counters.health_probes_clear.load(Ordering::Relaxed) == 1,
        WAIT,
    );
}

#[test]
fn pol_035_each_verdict_lands_in_its_own_counter() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![device(1)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    publish_health_probed(&h.publisher, true, BusHealthVerdict::One);
    publish_health_probed(&h.publisher, true, BusHealthVerdict::Several);
    wait_until(
        || {
            h.counters.health_probes_one_failure.load(Ordering::Relaxed) == 1
                && h.counters.health_probes_several_failures.load(Ordering::Relaxed) == 1
        },
        WAIT,
    );
    assert_eq!(h.counters.health_probes_clear.load(Ordering::Relaxed), 0);
    assert_eq!(h.counters.health_probes_invalid.load(Ordering::Relaxed), 0);
}

fn metered_device(short_address: u8, energy: bool, diagnostics: bool) -> PollTargetView {
    PollTargetView {
        short_address,
        is_dt8: false,
        is_dt6: false,
        declares_energy: energy,
        declares_diagnostics: diagnostics,
    }
}

#[test]
fn pol_040_bank_reads_need_the_switch_and_the_declared_type() {
    let settings = |include_energy: bool| PollerSettingsView {
        enabled: true,
        interval_ms: 100_000,
        attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
        include_dt8_color: false,
        include_energy,
        include_diagnostics: false,
        skip_unbound_virtual_lamps: false,
        power_cycle_watch: false,
    };

    let h = spawn_harness(
        StubRegistry {
            excluded_unbound: 0,
            devices: vec![metered_device(4, true, true)],
            settings: settings(false),
        },
        true,
        Some(AttributeGroupReadOutcome::Success),
    );
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) >= 1, WAIT);
    assert_eq!(h.seen_all()[0].command.memory_banks, MemoryBankReadPreset::None);
    drop(h);

    let h = spawn_harness(
        StubRegistry {
            excluded_unbound: 0,
            devices: vec![metered_device(4, false, false)],
            settings: settings(true),
        },
        true,
        Some(AttributeGroupReadOutcome::Success),
    );
    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) >= 1, WAIT);
    assert_eq!(
        h.seen_all()[0].command.memory_banks,
        MemoryBankReadPreset::None,
        "a gear that never declared type 51 is never asked for bank 202"
    );
}

#[test]
fn pol_041_each_bank_series_is_read_once_before_any_is_read_twice() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![metered_device(4, true, true)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: true,
            include_diagnostics: true,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, Some(AttributeGroupReadOutcome::Success));

    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) >= 4, WAIT);
    let asked: Vec<MemoryBankReadPreset> =
        h.seen_all().iter().take(4).map(|r| r.command.memory_banks).collect();
    let mut sorted = asked.clone();
    sorted.sort_by_key(|p| *p as u8);
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        4,
        "each series is read before any repeats: {asked:?}"
    );
    assert!(
        !asked.contains(&MemoryBankReadPreset::None),
        "with both switches on and both types declared, every read carries a series: {asked:?}"
    );

    wait_until(|| h.counters.reads_completed.load(Ordering::Relaxed) >= 6, WAIT);
    assert_eq!(
        h.seen_all()[5].command.memory_banks,
        MemoryBankReadPreset::None,
        "the shortest spacing is 30 s; nothing may be re-read within one cycle"
    );
}

fn probe_only_registry(interval_ms: u32) -> StubRegistry {
    StubRegistry {
        excluded_unbound: 0,
        devices: Vec::new(),
        settings: PollerSettingsView {
            enabled: true,
            interval_ms,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    }
}

#[test]
fn pol_036_a_probe_waits_for_the_interactive_quiet_window() {
    let h = spawn_harness(probe_only_registry(100), true, None);

    wait_until(|| h.held_probe_count() >= 1, WAIT);
    h.release_probe();
    wait_until(
        || h.counters.health_probes_clear.load(Ordering::Relaxed) == 1,
        WAIT,
    );

    let baseline = h.counters.health_probes_published.load(Ordering::Relaxed);
    assert!(
        remains_false_for(
            || {
                h.interactive.note_activity();
                h.counters.health_probes_published.load(Ordering::Relaxed) > baseline
            },
            Duration::from_millis(600)
        ),
        "the wire never went quiet for 400 ms, so no probe may be published"
    );

    wait_until(
        || h.counters.health_probes_published.load(Ordering::Relaxed) > baseline,
        WAIT,
    );
}

#[test]
fn pol_037_a_probe_pays_for_its_own_wire_time() {
    let h = spawn_harness(probe_only_registry(100), true, None);

    wait_until(|| h.held_probe_count() >= 1, WAIT);
    let cost = Duration::from_millis(300);
    assert!(remains_false_for(
        || h.counters.health_probes_published.load(Ordering::Relaxed) > 1,
        cost
    ));
    h.release_probe();

    assert!(
        remains_false_for(
            || h.counters.health_probes_published.load(Ordering::Relaxed) > 1,
            Duration::from_millis(400)
        ),
        "a probe that cost {cost:?} owes ~3x that in silence, whatever interval_ms says"
    );
    wait_until(
        || h.counters.health_probes_published.load(Ordering::Relaxed) > 1,
        WAIT,
    );
}

#[test]
fn pol_038_an_unanswered_probe_neither_charges_nor_repeats() {
    let h = spawn_harness(probe_only_registry(100), true, None);

    wait_until(|| h.held_probe_count() >= 1, WAIT);
    assert!(
        remains_false_for(
            || h.counters.health_probes_published.load(Ordering::Relaxed) > 1,
            Duration::from_millis(600)
        ),
        "one probe in flight: an unanswered probe must not be re-issued every cycle"
    );
    assert_eq!(
        h.counters.duty_deferred.load(Ordering::Relaxed),
        0,
        "an unanswered probe is a timeout, not a measured cost, so it charges nothing"
    );
}

#[test]
fn pol_039_an_interactive_episode_counts_once_not_once_per_wake() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: vec![device(1), device(2)],
        settings: PollerSettingsView {
            enabled: true,
            interval_ms: 100,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    wait_until(|| h.counters.reads_published.load(Ordering::Relaxed) >= 1, WAIT);
    wait_until(|| h.held_probe_count() >= 1, WAIT);
    h.release_probe();
    h.interactive.note_activity();
    h.release_one();

    wait_until(
        || {
            h.interactive.note_activity();
            h.counters.interactive_deferred.load(Ordering::Relaxed) >= 1
        },
        WAIT,
    );
    for _ in 0..10 {
        publish_outcomes(&h.publisher, u64::MAX, BUS_ID.0, 2, AttributeGroupReadOutcome::Success);
    }
    assert!(
        remains_false_for(
            || {
                h.interactive.note_activity();
                h.counters.interactive_deferred.load(Ordering::Relaxed) > 1
            },
            Duration::from_millis(300)
        ),
        "one blocked stretch is one episode, not one per wake-up"
    );

    wait_until(|| h.counters.reads_published.load(Ordering::Relaxed) >= 2, WAIT);
}

#[test]
fn m13_a_second_adapter_gets_its_probe_turn() {
    let h = spawn_harness_with_adapters(probe_only_registry(300), true, None, 2);

    for _ in 0..3 {
        wait_until(|| h.held_probe_count() >= 1, WAIT);
        wait_for_cycles(&h, 1);
        h.release_probe();
    }
    wait_until(|| h.probed_adapters().len() >= 3, WAIT);
    let probed = h.probed_adapters();
    assert!(
        probed.contains(&1),
        "one probe per window must still reach every adapter in turn: {probed:?}"
    );
}

#[test]
fn m13_an_interval_of_zero_is_paced_at_the_floor_not_spun_hot() {
    let registry = StubRegistry {
        excluded_unbound: 0,
        devices: Vec::new(),
        settings: PollerSettingsView {
            enabled: false,
            interval_ms: 0,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        },
    };
    let h = spawn_harness(registry, true, None);

    wait_until(|| h.counters.cycles_total.load(Ordering::Relaxed) >= 1, WAIT);
    let seen = h.counters.cycles_total.load(Ordering::Relaxed);
    assert!(
        remains_false_for(
            || h.counters.cycles_total.load(Ordering::Relaxed) >= seen + 4,
            Duration::from_millis(400),
        ),
        "a 200 ms floor admits at most two more cycle boundaries in 400 ms"
    );
    wait_until(|| h.counters.cycles_total.load(Ordering::Relaxed) > seen, WAIT);
}

struct SwitchableRole {
    active: std::sync::atomic::AtomicBool,
}

impl dali2rust_domain::registry::DaliSettingsReadPort for SwitchableRole {
    fn dali_settings_view(&self) -> dali2rust_domain::registry::DaliSettingsView {
        dali2rust_domain::registry::DaliSettingsView {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: self.active.load(Ordering::Relaxed),
            device_short_address: None,
        }
    }
}
