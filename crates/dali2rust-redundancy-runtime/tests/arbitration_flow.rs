use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{
    BusCommandPayload, BusEventPayload, Dali103ArbitrationProbedEvent, DaliSettingsUpdateCommand,
};
use dali2rust_domain::registry::{
    DaliSettingsReadPort, DaliSettingsView, RedundancySettingsReadPort, RedundancySettingsView,
};
use dali2rust_redundancy_runtime::{
    run_turn, ArbitrationAction, ArbitrationState, ArbitrationWorkerCounters,
    ArbitrationWorkerDeps, TransitionReason, WorkerStateHandle,
};

const ADAPTER: u8 = 0;

struct FakeSettings {
    enabled: AtomicBool,
    standby: AtomicBool,
    active: AtomicBool,
    mover: AtomicU8,
}

impl RedundancySettingsReadPort for FakeSettings {
    fn redundancy_settings_view(&self) -> RedundancySettingsView {
        RedundancySettingsView {
            enabled: self.enabled.load(Ordering::Relaxed),
            standby_role: self.standby.load(Ordering::Relaxed),
            probe_interval_ms: 250,
            takeover_after_missed: 2,
            boot_listen_ms: 0,
            peer_device_short_address: None,
            peer_url: String::new(),
        }
    }
}

impl DaliSettingsReadPort for FakeSettings {
    fn dali_settings_view(&self) -> DaliSettingsView {
        DaliSettingsView {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: self.active.load(Ordering::Relaxed),
            device_short_address: None,
        }
    }

    fn application_active_moved_by(&self) -> (bool, dali2rust_domain::registry::ApplicationActiveMover) {
        let mover = match self.mover.load(Ordering::Relaxed) {
            dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_WIRE => {
                dali2rust_domain::registry::ApplicationActiveMover::Wire
            }
            dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_COMMAND => {
                dali2rust_domain::registry::ApplicationActiveMover::Command
            }
            _ => dali2rust_domain::registry::ApplicationActiveMover::Unmoved,
        };
        (self.active.load(Ordering::Relaxed), mover)
    }
}

struct Rig {
    deps: ArbitrationWorkerDeps,
    settings: Arc<FakeSettings>,
    cmd_rx: dali2rust_bus::BusSubscriberRx,
    _host: BusHost,
}

const WATCHED: &[&str] = &["Dali103ArbitrationProbeCommand", "DaliSettingsUpdateCommand"];

fn rig(standby: bool) -> Rig {
    let (host, publisher, cmd_rx) =
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_commands(64, WATCHED));
    let settings = Arc::new(FakeSettings {
        enabled: AtomicBool::new(true),
        mover: AtomicU8::new(dali2rust_contracts::msg::APPLICATION_ACTIVE_UNMOVED),
        standby: AtomicBool::new(standby),
        active: AtomicBool::new(false),
    });
    Rig {
        deps: ArbitrationWorkerDeps {
            publisher,
            bus_id: BusId(1),
            registry_adapter_id: ADAPTER,
            redundancy: Arc::clone(&settings) as Arc<dyn RedundancySettingsReadPort>,
            dali: Arc::clone(&settings) as Arc<dyn DaliSettingsReadPort>,
            counters: Arc::new(ArbitrationWorkerCounters::default()),
            transitions: Arc::new(Mutex::new(Vec::new())),
        },
        settings,
        cmd_rx,
        _host: host,
    }
}

fn drain_at_least(rig: &Rig, count: usize) -> Vec<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut out = Vec::new();
    while out.len() < count && std::time::Instant::now() < deadline {
        while let Ok(BusFrame::Command(ce)) = rig.cmd_rx.try_recv() {
            out.push(payload_name(&ce.payload));
        }
    }
    out
}

fn drain_empty(rig: &Rig) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    let mut seen = Vec::new();
    while std::time::Instant::now() < deadline {
        while let Ok(BusFrame::Command(ce)) = rig.cmd_rx.try_recv() {
            seen.push(payload_name(&ce.payload));
        }
    }
    seen.is_empty()
}

fn payload_name(payload: &BusCommandPayload) -> String {
    match payload {
        BusCommandPayload::Dali103ArbitrationProbeCommand(_) => "probe".to_string(),
        BusCommandPayload::DaliSettingsUpdateCommand(c) => {
            format!("role:{}", c.application_active)
        }
        other => format!("{other:?}"),
    }
}

#[test]
fn a_standby_probes_and_does_not_touch_the_bus_while_the_peer_answers() {
    let rig = rig(true);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    assert_eq!(drain_at_least(&rig, 1), vec!["probe"]);

    for tick in 1..6 {
        state.note_verdict(true);
        run_turn(&mut state, &rig.deps, 1_000 + tick * 250);
        assert_eq!(drain_at_least(&rig, 1), vec!["probe"], "tick {tick}");
    }
    assert_eq!(state.state(), ArbitrationState::Passive { missed: 0 });
}

#[test]
fn two_silences_take_the_bus_and_the_transition_records_both_instants() {
    let rig = rig(true);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    drain_at_least(&rig, 1);

    state.note_verdict(false);
    assert_eq!(
        run_turn(&mut state, &rig.deps, 1_250),
        ArbitrationAction::Probe,
        "one silence must not move the bus (102 §3.13 Note 1)"
    );
    drain_at_least(&rig, 1);

    state.note_verdict(false);
    let action = run_turn(&mut state, &rig.deps, 1_500);
    assert_eq!(
        action,
        ArbitrationAction::Claim(TransitionReason::PeerSilent)
    );
    assert_eq!(drain_at_least(&rig, 1), vec!["role:true"]);

    let rows = rig.deps.transitions.lock().expect("transitions");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].reason, TransitionReason::Boot.code());
    assert!(!rows[0].now_active);
    let row = rows[1];
    assert!(row.now_active);
    assert_eq!(row.reason, TransitionReason::PeerSilent.code());
    assert_eq!(row.detected_at_ms, 1_500);
    assert_eq!(row.missed_probes, 2, "the record carries the silences that decided it");
}

#[test]
fn an_active_standby_stands_down_when_the_primary_answers_again() {
    let rig = rig(true);
    rig.settings.active.store(true, Ordering::Relaxed);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    drain_at_least(&rig, 1);

    state.note_verdict(true);
    let action = run_turn(&mut state, &rig.deps, 1_250);
    assert_eq!(
        action,
        ArbitrationAction::StandDown(TransitionReason::PeerAnswered)
    );
    assert_eq!(drain_at_least(&rig, 1), vec!["role:false"]);
    assert_eq!(state.state(), ArbitrationState::Passive { missed: 0 });
}

#[test]
fn a_verdict_arriving_early_does_not_bring_the_next_probe_forward() {
    let rig = rig(true);
    let mut state = WorkerStateHandle::new();
    assert_eq!(run_turn(&mut state, &rig.deps, 1_000), ArbitrationAction::Probe);
    assert_eq!(drain_at_least(&rig, 1), vec!["probe"]);
    state.note_verdict(true);
    assert_eq!(run_turn(&mut state, &rig.deps, 1_040), ArbitrationAction::Probe);
    assert!(drain_empty(&rig), "a probe went out 40 ms after the last one");
    assert_eq!(run_turn(&mut state, &rig.deps, 1_200), ArbitrationAction::Probe);
    assert!(drain_empty(&rig), "a probe went out 200 ms after the last one");
    assert_eq!(run_turn(&mut state, &rig.deps, 1_250), ArbitrationAction::Probe);
    assert_eq!(drain_at_least(&rig, 1), vec!["probe"]);
    assert!(drain_empty(&rig));
    assert_eq!(rig.deps.counters.probes_published.load(Ordering::Relaxed), 2);
}

fn last_reason(rig: &Rig) -> Option<(u8, bool)> {
    let rows = rig.deps.transitions.lock().expect("transitions");
    rows.last().map(|r| (r.reason, r.now_active))
}

#[test]
fn a_handover_we_sent_records_our_own_stand_down_as_a_handover() {
    let rig = rig(false);
    rig.settings.active.store(true, Ordering::Relaxed);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    assert_eq!(last_reason(&rig), Some((TransitionReason::Boot.code(), true)));
    state.note_event(
        &rig.deps,
        &BusEventPayload::Dali103HandoverSentEvent(
            dali2rust_contracts::msg::Dali103HandoverSentEvent {
                registry_adapter_id: ADAPTER,
                peer_short_address: 61,
            },
        ),
    );
    rig.settings.active.store(false, Ordering::Relaxed);
    assert_eq!(run_turn(&mut state, &rig.deps, 1_400), ArbitrationAction::Idle);
    assert_eq!(last_reason(&rig), Some((TransitionReason::Handover.code(), false)));
    assert!(drain_empty(&rig));
}

#[test]
fn a_flip_from_the_wire_is_a_handover_and_one_from_an_operator_is_manual() {
    let rig = rig(true);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    drain_at_least(&rig, 1);
    assert_eq!(last_reason(&rig), Some((TransitionReason::Boot.code(), false)));
    state.note_event(
        &rig.deps,
        &BusEventPayload::DaliSettingsChangedEvent(
            dali2rust_contracts::msg::DaliSettingsChangedEvent {
                dt8_auto_activation_repair: false,
                dt8_rgbwaf_control_assert: false,
                application_active: true,
                application_active_moved_by:
                    dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_WIRE,
            },
        ),
    );
    rig.settings.mover.store(dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_WIRE, Ordering::Relaxed);
    rig.settings.active.store(true, Ordering::Relaxed);
    run_turn(&mut state, &rig.deps, 1_250);
    assert_eq!(last_reason(&rig), Some((TransitionReason::Handover.code(), true)));
    state.note_event(
        &rig.deps,
        &BusEventPayload::DaliSettingsChangedEvent(
            dali2rust_contracts::msg::DaliSettingsChangedEvent {
                dt8_auto_activation_repair: false,
                dt8_rgbwaf_control_assert: false,
                application_active: false,
                application_active_moved_by:
                    dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_COMMAND,
            },
        ),
    );
    rig.settings.mover.store(dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_COMMAND, Ordering::Relaxed);
    rig.settings.active.store(false, Ordering::Relaxed);
    run_turn(&mut state, &rig.deps, 1_500);
    assert_eq!(last_reason(&rig), Some((TransitionReason::Manual.code(), false)));
}

#[test]
fn a_turn_that_outruns_the_event_still_names_the_handover() {
    let rig = rig(true);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    drain_at_least(&rig, 1);
    assert_eq!(last_reason(&rig), Some((TransitionReason::Boot.code(), false)));
    rig.settings.mover.store(dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_WIRE, Ordering::Relaxed);
    rig.settings.active.store(true, Ordering::Relaxed);
    run_turn(&mut state, &rig.deps, 1_250);
    assert_eq!(last_reason(&rig), Some((TransitionReason::Handover.code(), true)));
    state.note_event(
        &rig.deps,
        &BusEventPayload::DaliSettingsChangedEvent(
            dali2rust_contracts::msg::DaliSettingsChangedEvent {
                dt8_auto_activation_repair: false,
                dt8_rgbwaf_control_assert: false,
                application_active: true,
                application_active_moved_by:
                    dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_WIRE,
            },
        ),
    );
    run_turn(&mut state, &rig.deps, 1_500);
    assert_eq!(
        rig.deps.transitions.lock().expect("transitions").len(),
        2,
        "one boot, one handover — the late event records nothing twice"
    );
}

#[test]
fn a_primary_puts_nothing_on_the_bus_at_all() {
    let rig = rig(false);
    rig.settings.active.store(true, Ordering::Relaxed);
    let mut state = WorkerStateHandle::new();
    for tick in 0..5 {
        run_turn(&mut state, &rig.deps, 1_000 + tick * 250);
    }
    assert!(drain_empty(&rig));
    assert_eq!(state.state(), ArbitrationState::Active);
}

#[test]
fn switched_off_it_publishes_nothing_however_silent_the_bus_is() {
    let rig = rig(true);
    rig.settings.enabled.store(false, Ordering::Relaxed);
    let mut state = WorkerStateHandle::new();
    for tick in 0..5 {
        state.note_verdict(false);
        run_turn(&mut state, &rig.deps, 1_000 + tick * 250);
    }
    assert!(drain_empty(&rig));
    assert!(rig.deps.transitions.lock().expect("transitions").is_empty());
}

#[test]
fn the_transition_log_keeps_the_most_recent_entries_rather_than_the_first() {
    let rig = rig(true);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    for round in 0..6 {
        rig.settings.active.store(false, Ordering::Relaxed);
        state.note_verdict(false);
        run_turn(&mut state, &rig.deps, 2_000 + round * 1_000);
        state.note_verdict(false);
        run_turn(&mut state, &rig.deps, 2_100 + round * 1_000);
        rig.settings.active.store(true, Ordering::Relaxed);
        state.note_verdict(true);
        run_turn(&mut state, &rig.deps, 2_200 + round * 1_000);
    }
    let rows = rig.deps.transitions.lock().expect("transitions");
    assert!(rows.len() <= dali2rust_redundancy_runtime::TRANSITION_LOG_DEPTH);
    assert!(
        rows.first().expect("rows").detected_at_ms > 2_000,
        "the oldest entry should have aged out"
    );
}

#[test]
fn a_probe_that_never_reached_the_bus_is_not_evidence_against_the_peer() {
    let rig = rig(true);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    drain_at_least(&rig, 1);
    for tick in 1..5 {
        run_turn(&mut state, &rig.deps, 1_000 + tick * 250);
    }
    assert_eq!(state.state(), ArbitrationState::Passive { missed: 0 });
    assert!(!drain_at_least(&rig, 0).contains(&"role:true".to_string()));
}

#[test]
fn a_verdict_arriving_as_an_event_is_what_the_worker_acts_on() {
    let (_host, publisher, _rx) =
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_commands(4, WATCHED));
    let ev = dali2rust_contracts::bus::event_envelope(
        dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
        dali2rust_contracts::CORRELATION_NONE,
        1,
        None,
        Dali103ArbitrationProbedEvent {
            registry_adapter_id: ADAPTER,
            owned: false,
        },
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
}

#[test]
fn the_role_command_names_only_the_flag_it_moves() {
    let rig = rig(true);
    let mut state = WorkerStateHandle::new();
    run_turn(&mut state, &rig.deps, 1_000);
    drain_at_least(&rig, 1);
    state.note_verdict(false);
    run_turn(&mut state, &rig.deps, 1_250);
    state.note_verdict(false);
    run_turn(&mut state, &rig.deps, 1_500);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        while let Ok(BusFrame::Command(ce)) = rig.cmd_rx.try_recv() {
            if let BusCommandPayload::DaliSettingsUpdateCommand(c) = &ce.payload {
                assert_eq!(
                    c.patch_mask,
                    DaliSettingsUpdateCommand::PATCH_APPLICATION_ACTIVE
                );
                return;
            }
        }
    }
    panic!("no role command published");
}
