use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    BusEventPayload, Origin, RuleCommitCommand, RuleEnableCommand, RuleStageCommand,
    RULE_SOURCE_CHUNK_BYTES,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_rules_model::testing::StubResolver;
use dali2rust_rules_runtime::runtime::persistence::fnv1a32;
use dali2rust_rules_runtime::{spawn_rules_worker, RulesStore, RulesWorkerCounters};

struct EmptyWorld {
    started: std::time::Instant,
    lamps: Vec<dali2rust_rules_runtime::runtime::engine::LampState>,
    active: bool,
    lit_group: Arc<std::sync::atomic::AtomicBool>,
    overridden: Arc<std::sync::atomic::AtomicBool>,
}

const LIT_GROUP_ID: u16 = 2;

impl dali2rust_rules_runtime::RulesWorldPort for EmptyWorld {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn wall(&self) -> Option<dali2rust_rules_runtime::runtime::engine::WallTime> {
        None
    }
    fn sun(&self) -> Option<dali2rust_rules_runtime::runtime::engine::SunTimes> {
        None
    }
    fn controller_active(&self) -> bool {
        self.active
    }
    fn lamps(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::LampState> {
        self.lamps.clone()
    }
    fn groups(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::GroupState> {
        let any_on = self.lit_group.load(std::sync::atomic::Ordering::Relaxed);
        vec![dali2rust_rules_runtime::runtime::engine::GroupState {
            adapter_id: 0,
            id: LIT_GROUP_ID,
            any_on,
            all_off: !any_on,
            member_count: 1,
        }]
    }
    fn devices(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::DeviceState> {
        Vec::new()
    }
    fn inputs(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::InputState> {
        Vec::new()
    }
    fn hcl(&self) -> Vec<dali2rust_rules_runtime::runtime::engine::HclTargetState> {
        vec![dali2rust_rules_runtime::runtime::engine::HclTargetState {
            target: dali2rust_rules_model::LightTarget::Group(dali2rust_rules_model::GroupRef {
                adapter_id: 0,
                id: LIT_GROUP_ID,
            }),
            enabled: true,
            overridden: self.overridden.load(std::sync::atomic::Ordering::Relaxed),
        }]
    }
    fn input_instance_groups(&self, _a: u8, _s: u8, _i: u8) -> [Option<u8>; 3] {
        [None; 3]
    }
    fn hcl_schedules_for(&self, _t: &dali2rust_rules_model::LightTarget) -> Vec<String> {
        vec!["вечер".to_string()]
    }
}

const DOC: &str = "rule \"ночь\" {\n  when at 23:00\n  do broadcast.off()\n}\n";

const EXECUTOR_OUTPUT: &[&str] = &[
    "DaliSetTargetStateCommand",
    "DaliRecallSceneCommand",
    "DaliStopFadeCommand",
    "SceneApplyExecuteCommand",
    "HclOverrideClearCommand",
    "Dali103FeedbackDriveCommand",
    "MqttPublishCommand",
];

struct Harness {
    publisher: dali2rust_bus::BusPublisher,
    ev_rx: std::sync::mpsc::Receiver<BusFrame>,
    out_rx: std::sync::mpsc::Receiver<BusFrame>,
    store: Arc<RulesStore>,
    slices: Arc<dali2rust_bsp::slice_store_files::FileSliceStore>,
    cells: Arc<dali2rust_rules_runtime::RulesEngineCells>,
    counters: Arc<RulesWorkerCounters>,
    _host: BusHost,
    _worker: std::thread::JoinHandle<()>,
}

fn harness(label: &str) -> Harness {
    let slices = Arc::new(dali2rust_test_support::fs::temp_slice_store(label));
    harness_on(slices)
}

fn harness_on(slices: Arc<dali2rust_bsp::slice_store_files::FileSliceStore>) -> Harness {
    harness_with_lamps(slices, Vec::new())
}

fn harness_with_lamps(
    slices: Arc<dali2rust_bsp::slice_store_files::FileSliceStore>,
    lamps: Vec<dali2rust_rules_runtime::runtime::engine::LampState>,
) -> Harness {
    harness_with(slices, lamps, true)
}

fn harness_with(
    slices: Arc<dali2rust_bsp::slice_store_files::FileSliceStore>,
    lamps: Vec<dali2rust_rules_runtime::runtime::engine::LampState>,
    active: bool,
) -> Harness {
    harness_full(slices, lamps, active, Arc::new(std::sync::atomic::AtomicBool::new(false)))
}

fn harness_full(
    slices: Arc<dali2rust_bsp::slice_store_files::FileSliceStore>,
    lamps: Vec<dali2rust_rules_runtime::runtime::engine::LampState>,
    active: bool,
    lit_group: Arc<std::sync::atomic::AtomicBool>,
) -> Harness {
    harness_hcl(slices, lamps, active, lit_group, Arc::new(std::sync::atomic::AtomicBool::new(false)))
}

fn harness_hcl(
    slices: Arc<dali2rust_bsp::slice_store_files::FileSliceStore>,
    lamps: Vec<dali2rust_rules_runtime::runtime::engine::LampState>,
    active: bool,
    lit_group: Arc<std::sync::atomic::AtomicBool>,
    overridden: Arc<std::sync::atomic::AtomicBool>,
) -> Harness {
    let (host, publisher, (worker_rx, ev_rx, out_rx)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands_and_events(
                128,
                dali2rust_rules_runtime::RULES_WORKER_HANDLED_COMMANDS,
                dali2rust_rules_runtime::RULES_WORKER_HANDLED_EVENTS,
            ),
            reg.subscribe_events(64, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            reg.subscribe_commands(32, EXECUTOR_OUTPUT),
        )
    });
    let store = Arc::new(RulesStore::new());
    let cells = Arc::new(dali2rust_rules_runtime::RulesEngineCells::default());
    let counters = Arc::new(RulesWorkerCounters::default());
    let worker = spawn_rules_worker(
        worker_rx,
        publisher.clone(),
        BusId::default(),
        dali2rust_rules_runtime::RulesWorkerSeams {
            store: Arc::clone(&store),
            compiler: Arc::new(dali2rust_rules_lang::RulesLangV1),
            resolver: Arc::new(StubResolver::permissive()),
            slices: Some(slices.clone() as Arc<dyn dali2rust_platform::slice_store::SliceStore>),
            world: Arc::new(EmptyWorld { started: std::time::Instant::now(), lamps, active, lit_group, overridden }),
        },
        Arc::clone(&counters),
        Arc::clone(&cells),
        std::sync::Arc::new(dali2rust_platform::liveness::LivenessBeat::new("test", 60_000)),
    );
    Harness {
        publisher,
        ev_rx,
        out_rx,
        store,
        slices,
        cells,
        counters,
        _host: host,
        _worker: worker,
    }
}

fn publish_document(h: &Harness, corr: u64, source: &str, base_revision: u32) {
    let bytes = source.as_bytes();
    let chunks: Vec<&[u8]> = bytes.chunks(RULE_SOURCE_CHUNK_BYTES).collect();
    for (index, chunk) in chunks.iter().enumerate() {
        let mut fixed = dali2rust_contracts::msg::FixedItems::new();
        for b in *chunk {
            fixed.push(*b).expect("chunk fits");
        }
        publish(
            h,
            corr,
            RuleStageCommand {
                chunk_index: u8::try_from(index).unwrap(),
                chunk_count: u8::try_from(chunks.len()).unwrap(),
                bytes: fixed,
            },
        );
    }
    publish(
        h,
        corr,
        RuleCommitCommand {
            chunk_count: u8::try_from(chunks.len()).unwrap(),
            total_len: u16::try_from(bytes.len()).unwrap(),
            source_hash: fnv1a32(bytes),
            base_revision,
            lang_id: 1,
        },
    );
}

fn publish<P>(h: &Harness, corr: u64, payload: P)
where
    dali2rust_contracts::msg::BusCommandPayload: From<P>,
{
    let env = command_envelope(
        SOURCE_ID_UNSPECIFIED,
        corr,
        BusId::default().0,
        Some(Origin::Api),
        payload,
    );
    assert_eq!(
        h.publisher
            .try_publish(BusChannel::Commands, BusFrame::command(env)),
        PublishResult::Queued
    );
}

fn recv_signal(h: &Harness, corr: u64) -> dali2rust_contracts::msg::OperationWorkerSignalEvent {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let Ok(frame) = h.ev_rx.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        if let BusFrame::Event(ev) = frame {
            if ev.meta.correlation_id == corr {
                if let BusEventPayload::OperationWorkerSignalEvent(body) = &ev.payload {
                    return body.clone();
                }
            }
        }
    }
    panic!("no worker signal for {corr}");
}

fn recv_changed(h: &Harness, revision: u32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let Ok(frame) = h.ev_rx.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        if let BusFrame::Event(ev) = frame {
            if let BusEventPayload::RulesChangedEvent(body) = &ev.payload {
                if body.revision == revision {
                    return;
                }
            }
        }
    }
    panic!("no RulesChangedEvent at revision {revision}");
}

fn wait_revision(store: &RulesStore, at_least: u32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while store.revision() < at_least && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
}

#[test]
fn a_staged_commit_lands_and_hydrates_byte_for_byte() {
    let h = harness("rules-commit");
    publish_document(&h, 11, DOC, 0);
    let signal = recv_signal(&h, 11);
    assert!(signal.error.is_none(), "commit must succeed: {signal:?}");
    let doc = h.store.document();
    assert_eq!(doc.source, DOC, "the operator's bytes are the canon");
    assert_eq!(doc.revision, 1);
    assert_eq!(doc.compiled.as_ref().map(|s| s.rules.len()), Some(1));

    let h2 = harness_on(h.slices.clone());
    wait_revision(&h2.store, 1);
    let doc2 = h2.store.document();
    assert_eq!(doc2.source, DOC);
    assert_eq!(doc2.revision, 1);
}

#[test]
fn a_document_that_shrinks_past_a_bank_boundary_still_hydrates() {
    let h = harness("rules-shrink");
    let long = format!("{DOC}# {}\n", "0123456789".repeat(450));
    assert!(long.len() > 4_080, "the first document must span two banks");
    publish_document(&h, 41, &long, 0);
    assert!(recv_signal(&h, 41).error.is_none());
    publish_document(&h, 42, DOC, 1);
    assert!(recv_signal(&h, 42).error.is_none());
    assert_eq!(h.store.document().source, DOC);

    let h2 = harness_on(h.slices.clone());
    wait_revision(&h2.store, 2);
    let hydrated = h2.store.document();
    assert_eq!(
        hydrated.source, DOC,
        "the shorter document must come back whole, not as a torn one"
    );
    assert_eq!(hydrated.revision, 2);
    assert_eq!(
        hydrated.compiled.as_ref().map(|s| s.rules.len()),
        Some(1),
        "and it must still compile: a rule set nobody can hydrate is lost"
    );
}

#[test]
fn a_lost_revision_race_fails_the_operation_by_name() {
    let h = harness("rules-race");
    publish_document(&h, 21, DOC, 0);
    assert!(recv_signal(&h, 21).error.is_none());
    publish_document(&h, 22, "rule \"x\" {\n  when every 5m\n  do log(\"y\")\n}\n", 0);
    let signal = recv_signal(&h, 22);
    let error = signal.error.expect("a lost race is refused, not merged");
    assert_eq!(error.message.as_str(), "rule_set_conflict");
    assert_eq!(h.store.document().source, DOC, "the standing document survives");
}

#[test]
fn an_enable_toggle_bumps_the_revision_and_survives_hydration() {
    let h = harness("rules-enable");
    publish_document(&h, 31, DOC, 0);
    assert!(recv_signal(&h, 31).error.is_none());
    publish(
        &h,
        32,
        RuleEnableCommand {
            name: dali2rust_contracts::msg::fixed_text_64("ночь"),
            enabled: false,
        },
    );
    recv_changed(&h, 2);
    let doc = h.store.document();
    assert_eq!(doc.revision, 2);
    assert_eq!(
        doc.compiled.as_ref().and_then(|s| s.rule("ночь")).map(|r| r.enabled),
        Some(false)
    );

    let h2 = harness_on(h.slices.clone());
    wait_revision(&h2.store, 1);
    assert_eq!(
        h2.store
            .document()
            .compiled
            .as_ref()
            .and_then(|s| s.rule("ночь"))
            .map(|r| r.enabled),
        Some(false),
        "the manifest bit outlives the reboot"
    );
}

#[test]
fn a_rule_publishes_its_effects_in_source_order_and_merges_per_target() {
    let h = harness("rules-order");
    publish_document(
        &h,
        41,
        "rule \"смесь\" {\n  when every 5m\n  do broadcast.level(50)\n     broadcast.cct(2700)\n     scene(3).recall(broadcast)\n}\n",
        0,
    );
    let sig = recv_signal(&h, 41);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish(
        &h,
        42,
        dali2rust_contracts::msg::RuleRunCommand {
            name: dali2rust_contracts::msg::fixed_text_64("смесь"),
            dry: false,
        },
    );

    let published: Vec<_> = std::iter::repeat_with(|| {
        dali2rust_test_support::try_recv_command_matching(&h.out_rx, COMMAND_WAIT, |_| true)
    })
    .take(2)
    .flatten()
    .collect();

    let kinds: Vec<&str> = published
        .iter()
        .map(|ce| match &ce.payload {
            dali2rust_contracts::msg::BusCommandPayload::DaliRecallSceneCommand(_) => "recall",
            dali2rust_contracts::msg::BusCommandPayload::DaliSetTargetStateCommand(_) => "setpoint",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["setpoint", "recall"],
        "the light verbs are written first and must go out first; hoisting \
         every light effect behind every other kind inverts what the lamp \
         ends at, because `DaliWorker` will not reorder two lamp-driving \
         frames and neither family supersedes the other"
    );

    let dali2rust_contracts::msg::BusCommandPayload::DaliSetTargetStateCommand(ts) =
        &published[0].payload
    else {
        panic!("first command must be the merged setpoint");
    };
    assert_eq!(ts.setpoint.level, 50, "the level verb must survive the merge");
    assert!(
        ts.setpoint.color.is_some(),
        "the colour verb must survive it too — two verbs on one lamp are ONE \
         command, or the later one displaces the earlier at the worker"
    );
}

#[test]
fn on_without_a_level_asks_the_gear_for_its_last_active_level() {
    let h = harness_with_lamps(
        Arc::new(dali2rust_test_support::fs::temp_slice_store("rules-on-last-active")),
        vec![dali2rust_rules_runtime::runtime::engine::LampState {
            adapter_id: 0,
            id: 6,
            is_on: false,
            level: 0,
            cct_kelvin: None,
            last_level: 1,
        }],
    );
    publish_document(
        &h,
        51,
        "rule \"подсветка\" {\n  when http trigger\n  do lamp(6).on()\n}\n",
        0,
    );
    let sig = recv_signal(&h, 51);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish(
        &h,
        52,
        dali2rust_contracts::msg::RuleRunCommand {
            name: dali2rust_contracts::msg::fixed_text_64("подсветка"),
            dry: false,
        },
    );

    let (power, level) = recv_setpoint(&h).expect("the rule must publish a setpoint");
    assert_eq!(power, dali2rust_contracts::msg::PowerState::On);
    assert_eq!(
        level, 0,
        "no level means GO TO LAST ACTIVE LEVEL; a synthesized {} would be a \
         DAPC the operator never asked for",
        level
    );
}

fn publish_bus_event<P>(h: &Harness, corr: u64, payload: P)
where
    dali2rust_contracts::msg::BusEventPayload: From<P>,
{
    let env = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        corr,
        BusId::default().0,
        Some(Origin::Internal),
        payload,
    );
    assert_eq!(
        h.publisher
            .try_publish(BusChannel::Events, BusFrame::event(env)),
        PublishResult::Queued
    );
}

const COMMAND_WAIT: Duration = Duration::from_secs(2);

fn recv_setpoint(h: &Harness) -> Option<(dali2rust_contracts::msg::PowerState, u8)> {
    dali2rust_test_support::try_recv_command_matching(&h.out_rx, COMMAND_WAIT, |payload| {
        matches!(
            payload,
            dali2rust_contracts::msg::BusCommandPayload::DaliSetTargetStateCommand(_)
        )
    })
    .and_then(|ce| match ce.payload {
        dali2rust_contracts::msg::BusCommandPayload::DaliSetTargetStateCommand(ts) => {
            Some((ts.setpoint.power, ts.setpoint.level))
        }
        _ => None,
    })
}

#[test]
fn a_takeover_fires_the_controller_role_triggers() {
    let h = harness_with(
        Arc::new(dali2rust_test_support::fs::temp_slice_store("rules-takeover")),
        Vec::new(),
        false,
    );
    publish_document(
        &h,
        61,
        "rule \"роль\" {\n  when controller becomes active\n  do lamp(6).level(77)\n}\n",
        0,
    );
    let sig = recv_signal(&h, 61);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish_bus_event(
        &h,
        62,
        dali2rust_contracts::msg::RedundancyTransitionEvent {
            now_active: true,
            reason: 0,
            detected_at_ms: 1,
            completed_at_ms: 2,
            last_peer_answer_ms: 0,
            missed_probes: 3,
        },
    );

    let (power, level) = recv_setpoint(&h).expect("the takeover must fire the rule");
    assert_eq!(power, dali2rust_contracts::msg::PowerState::On);
    assert_eq!(level, 77);
}

#[test]
fn the_role_that_did_not_change_fires_nothing() {
    let h = harness_with(
        Arc::new(dali2rust_test_support::fs::temp_slice_store("rules-role-repeat")),
        Vec::new(),
        true,
    );
    publish_document(
        &h,
        71,
        "rule \"роль\" {\n  when controller becomes active\n  do lamp(6).level(77)\n}\n",
        0,
    );
    let sig = recv_signal(&h, 71);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish_bus_event(
        &h,
        72,
        dali2rust_contracts::msg::RedundancyTransitionEvent {
            now_active: true,
            reason: 4,
            detected_at_ms: 1,
            completed_at_ms: 2,
            last_peer_answer_ms: 0,
            missed_probes: 0,
        },
    );

    assert!(
        recv_setpoint(&h).is_none(),
        "an active controller told it is active has not transitioned"
    );
}


#[test]
fn a_group_edge_arrives_with_the_event_that_moved_it() {
    let lit = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let h = harness_full(
        Arc::new(dali2rust_test_support::fs::temp_slice_store("rules-group-edge")),
        Vec::new(),
        true,
        Arc::clone(&lit),
    );
    publish_document(
        &h,
        81,
        "rule \"подсветка\" {\n  when group(2) becomes any_on\n  do lamp(6).level(90)\n}\n",
        0,
    );
    let sig = recv_signal(&h, 81);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish_bus_event(&h, 82, runtime_state_changed(1));
    assert!(recv_setpoint(&h).is_none(), "no transition yet");

    lit.store(true, std::sync::atomic::Ordering::Relaxed);
    let started = std::time::Instant::now();
    publish_bus_event(&h, 83, runtime_state_changed(2));

    let (_, level) = recv_setpoint(&h).expect("the group edge must fire the rule");
    assert_eq!(level, 90);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "the edge came from the event, not from the next tick: {:?}",
        started.elapsed()
    );
}

fn runtime_state_changed(lamp_id: u8) -> dali2rust_contracts::msg::RuntimeStateChangedEvent {
    let state_setpoint = dali2rust_contracts::msg::LightSetpoint {
        power: dali2rust_contracts::msg::PowerState::On,
        level: 200,
        color: None,
    };
    dali2rust_contracts::msg::RuntimeStateChangedEvent {
        adapter_id: 0,
        virtual_lamp_id: Some(lamp_id),
        short_address: Some(lamp_id),
        state_setpoint: state_setpoint.clone(),
        state_observation: dali2rust_contracts::msg::RuntimeObservation::default(),
        commit_source: dali2rust_contracts::msg::RuntimeSource::Api,
        commit_dimensions: state_setpoint.dimensions(),
    }
}

#[test]
fn a_failed_activation_fires_the_rule_that_watches_it() {
    let h = harness("rules-failure");
    publish_document(
        &h,
        91,
        "rule \"падает\" {\n  when http trigger\n  do hcl.hold(group(2))\n}\n\
         \nrule \"ловит\" {\n  when rule(\"падает\") fails\n  do lamp(6).level(55)\n}\n",
        0,
    );
    let sig = recv_signal(&h, 91);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish(
        &h,
        92,
        dali2rust_contracts::msg::RuleRunCommand {
            name: dali2rust_contracts::msg::fixed_text_64("падает"),
            dry: false,
        },
    );

    let (_, level) = recv_setpoint(&h).expect("the failure must fire the watching rule");
    assert_eq!(level, 55);
}

#[test]
fn a_rule_that_watches_its_own_failure_does_not_recurse() {
    let h = harness("rules-failure-self");
    publish_document(
        &h,
        101,
        "rule \"сам\" {\n  when http trigger\n  when rule(\"сам\") fails\n  do hcl.hold(group(2))\n}\n",
        0,
    );
    let sig = recv_signal(&h, 101);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish(
        &h,
        102,
        dali2rust_contracts::msg::RuleRunCommand {
            name: dali2rust_contracts::msg::fixed_text_64("сам"),
            dry: false,
        },
    );

    assert!(recv_setpoint(&h).is_none(), "hcl.hold reaches no command");
    publish(
        &h,
        103,
        RuleEnableCommand {
            name: dali2rust_contracts::msg::fixed_text_64("сам"),
            enabled: false,
        },
    );
    recv_changed(&h, 2);
}

#[test]
fn a_dry_run_reports_no_failure() {
    let h = harness("rules-failure-dry");
    publish_document(
        &h,
        111,
        "rule \"падает\" {\n  when http trigger\n  do hcl.hold(group(2))\n}\n\
         \nrule \"ловит\" {\n  when rule(\"падает\") fails\n  do lamp(6).level(55)\n}\n",
        0,
    );
    let sig = recv_signal(&h, 111);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish(
        &h,
        112,
        dali2rust_contracts::msg::RuleRunCommand {
            name: dali2rust_contracts::msg::fixed_text_64("падает"),
            dry: true,
        },
    );

    assert!(
        recv_setpoint(&h).is_none(),
        "a preview of a failing rule must not run the rule that watches it"
    );
}

#[test]
fn an_override_edge_reaches_the_rule_that_watches_it() {
    let overridden = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let h = harness_hcl(
        Arc::new(dali2rust_test_support::fs::temp_slice_store("rules-hcl-override")),
        Vec::new(),
        true,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Arc::clone(&overridden),
    );
    publish_document(
        &h,
        121,
        "rule \"перехват\" {\n  when hcl override starts for group(2)\n  do lamp(6).level(44)\n}\n\
         \nrule \"часы\" {\n  when at 23:00\n  do log(\"тик\")\n}\n",
        0,
    );
    let sig = recv_signal(&h, 121);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    dali2rust_test_support::await_counter_u32(
        &h.cells.ticks_time_unsynced,
        |ticks| ticks >= 1,
        Duration::from_secs(3),
    );
    overridden.store(true, std::sync::atomic::Ordering::Relaxed);

    let (_, level) = recv_setpoint(&h).expect("the override edge must fire the rule");
    assert_eq!(level, 44);
}

#[test]
fn a_part_333_flag_moving_fires_the_rule_that_watches_the_device() {
    let h = harness("rules-manual-config");
    publish_document(
        &h,
        131,
        "rule \"локально\" {\n  when input device(dev=3) manual config changed\n  do lamp(6).level(33)\n}\n",
        0,
    );
    let sig = recv_signal(&h, 131);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish_bus_event(&h, 132, instance_configured(false));
    assert!(
        recv_setpoint(&h).is_none(),
        "the first reading of an instance is a seed, not a change"
    );

    publish_bus_event(&h, 133, instance_configured(true));
    let (_, level) = recv_setpoint(&h).expect("the flag moving must fire the rule");
    assert_eq!(level, 33);
}

fn instance_configured(manual: bool) -> dali2rust_contracts::msg::Dali103InstanceConfiguredEvent {
    dali2rust_contracts::msg::Dali103InstanceConfiguredEvent {
        registry_adapter_id: 0,
        short_address: 3,
        instance_number: 0,
        event_scheme: None,
        event_filter: None,
        event_priority: None,
        instance_groups: [None; 3],
        timers: [None; 4],
        manual_config_active: Some(manual),
        feedback_opcode_map: None,
        feedback_capability: None,
        feedback_colour_capability: None,
        feedback_timing: None,
        feedback_active_brightness: None,
        feedback_active_colour: None,
        feedback_inactive_brightness: None,
        feedback_inactive_colour: None,
        instance_status: None,
        resolution: None,
        instance_status_written: false,
    }
}


#[test]
fn stop_fade_publishes_its_own_command_for_a_group() {
    let h = harness("rules-stop-fade-group");
    publish_document(
        &h,
        1,
        "rule \"стоп\" {\n  when http trigger\n  do group(2).stop_fade()\n}\n",
        0,
    );
    let sig = recv_signal(&h, 1);
    assert!(sig.error.is_none(), "the document must compile: {sig:?}");
    wait_revision(&h.store, 1);

    publish(
        &h,
        2,
        dali2rust_contracts::msg::RuleRunCommand {
            name: dali2rust_contracts::msg::fixed_text_64("стоп"),
            dry: false,
        },
    );

    let published = dali2rust_test_support::try_recv_command_matching(
        &h.out_rx,
        COMMAND_WAIT,
        |payload| {
            matches!(
                payload,
                dali2rust_contracts::msg::BusCommandPayload::DaliStopFadeCommand(_)
            )
        },
    )
    .expect("a group stop_fade must reach the bus");
    let dali2rust_contracts::msg::BusCommandPayload::DaliStopFadeCommand(cmd) = published.payload
    else {
        unreachable!("filtered above")
    };
    assert_eq!(cmd.scope, dali2rust_contracts::msg::DaliTargetScope::Group);
    assert_eq!(cmd.group_id, 2);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landing {
    Bus(&'static str),
    BusVia(&'static str, &'static str),
    Unmapped(&'static str),
    Counted(&'static str),
    NoEffect,
}

const LANDINGS: &[(&str, &str, Landing)] = &[
    ("light_on", "broadcast.on()", Landing::Bus("DaliSetTargetStateCommand")),
    ("light_off", "broadcast.off()", Landing::Bus("DaliSetTargetStateCommand")),
    ("light_toggle", "broadcast.toggle()", Landing::Bus("DaliSetTargetStateCommand")),
    ("light_level", "broadcast.level(100)", Landing::Bus("DaliSetTargetStateCommand")),
    ("light_dim", "lamp(0).dim(+10)", Landing::Bus("DaliSetTargetStateCommand")),
    (
        "light_dim_hold",
        "lamp(0).dim_hold(+10)",
        Landing::BusVia(
            "DaliSetTargetStateCommand",
            "engine_core::dim_hold_steps_by_elapsed_time_not_by_event_count",
        ),
    ),
    ("light_cct", "broadcast.cct(3000)", Landing::Bus("DaliSetTargetStateCommand")),
    ("light_xy", "broadcast.xy(0.3128, 0.3290)", Landing::Bus("DaliSetTargetStateCommand")),
    ("light_rgb", "broadcast.rgb(254, 0, 0)", Landing::Bus("DaliSetTargetStateCommand")),
    ("light_last_active", "broadcast.last_active()", Landing::Bus("DaliSetTargetStateCommand")),
    ("light_stop_fade", "broadcast.stop_fade()", Landing::Bus("DaliStopFadeCommand")),
    ("scene_recall", "scene(3).recall(broadcast)", Landing::Bus("DaliRecallSceneCommand")),
    ("scene_apply", "scene(3).apply()", Landing::Bus("SceneApplyExecuteCommand")),
    ("scene_cycle", "scene.cycle(1, 3, 7)", Landing::Bus("DaliRecallSceneCommand")),
    ("hcl_resume", "hcl.resume(broadcast)", Landing::Bus("HclOverrideClearCommand")),
    ("hcl_hold", "hcl.hold(broadcast)", Landing::Unmapped("hcl_hold_unmapped")),
    ("hcl_enable", "hcl.enable(\"вечер\")", Landing::Unmapped("hcl_schedule_unmapped")),
    ("hcl_disable", "hcl.disable(\"вечер\")", Landing::Unmapped("hcl_schedule_unmapped")),
    ("input_feedback_on", "input(3,0).feedback.on()", Landing::Bus("Dali103FeedbackDriveCommand")),
    ("input_feedback_off", "input(3,0).feedback.off()", Landing::Bus("Dali103FeedbackDriveCommand")),
    ("panel_select", "panel_select(group=4, selected=1)", Landing::Bus("Dali103FeedbackDriveCommand")),
    ("input_cancel_hold", "input(3,0).cancel_hold()", Landing::Unmapped("input_action_unmapped")),
    ("input_catch_movement", "input(3,0).catch_movement()", Landing::Unmapped("input_action_unmapped")),
    ("wait", "wait 1s", Landing::NoEffect),
    ("after", "after 1m do { log(\"x\") }", Landing::NoEffect),
    ("timer_start", "timer(\"t\").start(1m)", Landing::NoEffect),
    ("timer_restart", "timer(\"t\").restart(1m)", Landing::NoEffect),
    ("timer_cancel", "timer(\"t\").cancel()", Landing::NoEffect),
    ("call", "call(\"блок\")", Landing::NoEffect),
    ("repeat", "repeat 2 { log(\"x\") }", Landing::NoEffect),
    ("conditional", "if var(\"режим\") == \"ночь\" { log(\"x\") }", Landing::NoEffect),
    ("var_set", "var(\"режим\").set(\"ночь\")", Landing::NoEffect),
    ("var_add", "var(\"счётчик\").add(1)", Landing::NoEffect),
    ("rule_enable", "rule(\"другое\").enable()", Landing::NoEffect),
    ("rule_disable", "rule(\"другое\").disable()", Landing::NoEffect),
    ("mqtt_publish", "mqtt.publish(\"t\", \"p\")", Landing::Bus("MqttPublishCommand")),
    ("log", "log(\"x\")", Landing::Counted("log_lines")),
    ("stat_count", "stat(\"имя\").count()", Landing::Counted("stat_counts")),
];

#[test]
fn every_action_kind_declares_where_it_lands() {
    use dali2rust_rules_model::action::ActionKind;
    let declared: std::collections::BTreeSet<&str> =
        LANDINGS.iter().map(|(kind, _, _)| *kind).collect();
    assert_eq!(
        declared.len(),
        LANDINGS.len(),
        "a kind is declared twice in LANDINGS"
    );
    let known: std::collections::BTreeSet<&str> =
        ActionKind::ALL.into_iter().map(ActionKind::name).collect();
    assert_eq!(
        declared, known,
        "LANDINGS and ActionKind::ALL disagree — a verb with no landing, or a \
         landing for a verb that no longer exists"
    );
}

fn landing_document(snippet: &str) -> String {
    format!(
        "def \"блок\" {{\n  log(\"b\")\n}}\n\
         rule \"другое\" {{\n  when http trigger\n  do log(\"o\")\n}}\n\
         rule \"под тестом\" {{\n  when http trigger\n  do {snippet}\n}}\n"
    )
}

fn run_landing(kind: &str, snippet: &str) -> (Vec<String>, Arc<RulesWorkerCounters>) {
    let lamps = vec![dali2rust_rules_runtime::runtime::engine::LampState {
        adapter_id: 0,
        id: 0,
        is_on: true,
        level: 120,
        cct_kelvin: Some(3000),
        last_level: 120,
    }];
    let h = harness_with_lamps(
        Arc::new(dali2rust_test_support::fs::temp_slice_store(&format!(
            "landing-{kind}"
        ))),
        lamps,
    );
    publish_document(&h, 1, &landing_document(snippet), 0);
    let sig = recv_signal(&h, 1);
    assert!(
        sig.error.is_none(),
        "{kind}: the document must compile — {snippet:?} — {sig:?}"
    );
    wait_revision(&h.store, 1);

    publish(
        &h,
        2,
        dali2rust_contracts::msg::RuleRunCommand {
            name: dali2rust_contracts::msg::fixed_text_64("под тестом"),
            dry: false,
        },
    );

    let mut seen = Vec::new();
    while let Some(ce) =
        dali2rust_test_support::try_recv_command_matching(&h.out_rx, COMMAND_WAIT, |_| true)
    {
        seen.push(ce.payload.variant_name().to_string());
    }
    (seen, Arc::clone(&h.counters))
}

#[test]
fn every_bus_via_landing_still_compiles_and_activates() {
    for (kind, snippet, landing) in LANDINGS {
        let Landing::BusVia(command, proof) = landing else {
            continue;
        };
        assert!(
            !proof.is_empty(),
            "{kind}: lands on {command} via an unnamed test"
        );
        let (seen, _) = run_landing(kind, snippet);
        assert!(
            seen.is_empty(),
            "{kind}: it DOES reach the bus from a manual run ({seen:?}) — make \
             it a plain `Bus` row instead of pointing at {proof}"
        );
    }
}

#[test]
fn every_bus_landing_reaches_the_bus_as_declared() {
    for (kind, snippet, landing) in LANDINGS {
        let Landing::Bus(expected) = landing else {
            continue;
        };
        let (seen, _) = run_landing(kind, snippet);
        assert!(
            seen.iter().any(|name| name == expected),
            "{kind}: `{snippet}` was declared to land on {expected} and put \
             {seen:?} on the bus"
        );
    }
}

#[test]
fn every_unmapped_landing_moves_its_counter_and_nothing_else() {
    for (kind, snippet, landing) in LANDINGS {
        let Landing::Unmapped(counter) = landing else {
            continue;
        };
        let (seen, counters) = run_landing(kind, snippet);
        assert!(
            seen.is_empty(),
            "{kind}: declared unmapped, yet it published {seen:?}"
        );
        assert!(
            unmapped_counter(&counters, counter) > 0,
            "{kind}: declared unmapped via `{counter}`, and that counter did \
             not move — the no-op is invisible to an operator"
        );
    }
}

fn unmapped_counter(counters: &RulesWorkerCounters, name: &str) -> u32 {
    use std::sync::atomic::Ordering;
    match name {
        "hcl_hold_unmapped" => counters.hcl_hold_unmapped.load(Ordering::Relaxed),
        "hcl_schedule_unmapped" => counters.hcl_schedule_unmapped.load(Ordering::Relaxed),
        "input_action_unmapped" => counters.input_action_unmapped.load(Ordering::Relaxed),
        "log_lines" => counters.log_lines.load(Ordering::Relaxed),
        "stat_counts" => counters.stat_counts.load(Ordering::Relaxed),
        other => panic!("LANDINGS names a counter this test cannot read: {other}"),
    }
}

#[test]
fn every_counted_landing_moves_its_counter() {
    for (kind, snippet, landing) in LANDINGS {
        let Landing::Counted(counter) = landing else {
            continue;
        };
        let (seen, counters) = run_landing(kind, snippet);
        assert!(seen.is_empty(), "{kind}: published {seen:?}");
        assert!(
            unmapped_counter(&counters, counter) > 0,
            "{kind}: `{counter}` did not move"
        );
    }
}

fn landing_setpoint(kind: &str, snippet: &str) -> dali2rust_contracts::msg::LightSetpoint {
    let lamps = vec![dali2rust_rules_runtime::runtime::engine::LampState {
        adapter_id: 0,
        id: 0,
        is_on: true,
        level: 120,
        cct_kelvin: Some(3000),
        last_level: 120,
    }];
    let h = harness_with_lamps(
        Arc::new(dali2rust_test_support::fs::temp_slice_store(&format!(
            "argument-{kind}"
        ))),
        lamps,
    );
    publish_document(&h, 1, &landing_document(snippet), 0);
    let sig = recv_signal(&h, 1);
    assert!(sig.error.is_none(), "{kind}: {snippet:?} — {sig:?}");
    wait_revision(&h.store, 1);
    publish(
        &h,
        2,
        dali2rust_contracts::msg::RuleRunCommand {
            name: dali2rust_contracts::msg::fixed_text_64("под тестом"),
            dry: false,
        },
    );
    let ce = dali2rust_test_support::try_recv_command_matching(&h.out_rx, COMMAND_WAIT, |payload| {
        matches!(
            payload,
            dali2rust_contracts::msg::BusCommandPayload::DaliSetTargetStateCommand(_)
        )
    })
    .unwrap_or_else(|| panic!("{kind}: `{snippet}` published no setpoint"));
    match ce.payload {
        dali2rust_contracts::msg::BusCommandPayload::DaliSetTargetStateCommand(ts) => ts.setpoint,
        _ => unreachable!("filtered above"),
    }
}

#[test]
fn every_light_argument_reaches_the_setpoint() {
    use dali2rust_contracts::msg::{ColorMode, PowerState};

    let sp = landing_setpoint("level-absolute", "lamp(0).on(level=200)");
    assert_eq!((sp.power, sp.level), (PowerState::On, 200));

    let sp = landing_setpoint("level-relative", "lamp(0).level(+10)");
    assert_eq!(sp.level, 130, "a relative step must apply to the stored level");

    let sp = landing_setpoint("level-saturating", "lamp(0).level(+200)");
    assert_eq!(sp.level, 254, "a relative step saturates, it does not wrap");

    let sp = landing_setpoint("cct-absolute", "lamp(0).cct(2700)");
    let color = sp.color.expect("a cct verb states a colour");
    assert_eq!(color.mode, ColorMode::Cct);
    assert_eq!(color.color_temperature_kelvin, 2700);
    assert_eq!(
        sp.power,
        PowerState::Unknown,
        "a colour verb must not touch power — a slider cannot light a dark lamp"
    );

    let sp = landing_setpoint("cct-relative", "lamp(0).cct(+200)");
    let color = sp.color.expect("a cct verb states a colour");
    assert_eq!(color.color_temperature_kelvin, 3200);

    let sp = landing_setpoint("rgb", "lamp(0).rgb(254, 0, 12)");
    let color = sp.color.expect("an rgb verb states a colour");
    assert_eq!(color.mode, ColorMode::Rgb);
    assert_eq!((color.r, color.g, color.b), (254, 0, 12));

    let sp = landing_setpoint("xy", "lamp(0).xy(0.3128, 0.3290)");
    let color = sp.color.expect("an xy verb states a colour");
    assert_eq!(color.mode, ColorMode::Xy);
    assert_eq!((color.x, color.y), (3128, 3290));
}

#[test]
fn hold_hcl_true_changes_nothing_and_false_never_reaches_the_bus() {
    let plain = landing_setpoint("hold-hcl-default", "lamp(0).level(200)");
    let explicit = landing_setpoint("hold-hcl-true", "lamp(0).level(200, hold_hcl=true)");
    assert_eq!(
        plain, explicit,
        "`hold_hcl=true` is the default written out; it must not change the \
         published command"
    );

    let h = harness("landing-hold-hcl-false");
    publish_document(&h, 1, &landing_document("lamp(0).level(200, hold_hcl=false)"), 0);
    let sig = recv_signal(&h, 1);
    assert!(
        sig.error.is_some(),
        "`hold_hcl=false` must be refused at the commit, not accepted and \
         silently dropped (ISSUE-96)"
    );
}

#[test]
fn fade_is_refused_at_the_document_boundary() {
    let h = harness("landing-fade-refused");
    publish_document(&h, 1, &landing_document("lamp(0).off(fade=3s)"), 0);
    let sig = recv_signal(&h, 1);
    assert!(
        sig.error.is_some(),
        "a document carrying `fade=` must be refused at the commit, not \
         accepted and silently dropped (ISSUE-94)"
    );
}

#[test]
fn a_slice_reload_reaches_a_live_engine_issue101() {
    let files = Arc::new(dali2rust_test_support::fs::temp_slice_store("rules-reload"));
    let standby = harness_on(Arc::clone(&files));
    let active = harness_on(Arc::clone(&files));

    publish_document(&active, 71, DOC, 0);
    let signal = recv_signal(&active, 71);
    assert!(signal.error.is_none(), "the active must commit: {signal:?}");
    wait_revision(&active.store, 1);

    assert_eq!(
        standby.store.document().source,
        "",
        "the standby hydrated before the write and must still hold nothing"
    );

    publish_bus_event(
        &standby,
        dali2rust_contracts::CORRELATION_NONE,
        dali2rust_contracts::msg::RegistrySliceReloadedEvent {
            slice_name: dali2rust_contracts::msg::fixed_text_32("rules_b0"),
        },
    );

    wait_revision(&standby.store, 1);
    let doc = standby.store.document();
    assert_eq!(
        doc.source, DOC,
        "the standby must re-read the replicated document without a reboot"
    );
    assert_eq!(doc.revision, 1);
    assert_eq!(
        doc.compiled.as_ref().map(|s| s.rules.len()),
        Some(1),
        "a document that hydrated but did not compile would leave the engine \
         empty while REST reported a rule — the half `hydrate_failed` counts"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while standby.cells.rules_loaded.load(std::sync::atomic::Ordering::Relaxed) == 0
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        standby.cells.rules_loaded.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "the store moved and the engine did not: that is a standby that would \
         take the bus running the document it booted with"
    );
}
