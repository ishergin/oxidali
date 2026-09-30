use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx,
    CorrelationIdAllocator, PublishResult,
};
use dali2rust_contracts::msg::{
    BusCommandPayload, ColorMode, ColorValue, DaliTargetScope, HclAlgorithm, HclLevelMode,
    HclOverrideClearCommand, HclOverrideHoldCommand, HclOverrideResumeCommand, HclOverrideTarget,
    HclSchedulePointRow, HclTargetRow, HclTargetScope, HclTimeRef, LightSetpoint, PowerState,
    RuntimeSource,
};
use dali2rust_domain::registry::{
    AdapterReadPort, AdapterView, GroupApplyRowView, GroupApplySnapshot,
    GroupMembershipMatrixView, GroupReadPort, GroupView, HclOverrideReadPort, HclOverrideView,
    HclScheduleReadPort, HclScheduleView, HclSchedulerReadPort,
};
use dali2rust_hcl_runtime::{
    spawn_hcl_scheduler_worker, HclConfig, HclOverrideLedgerRead, HclSchedulerCounters,
    OverrideLedger, SharedOverrideLedger, HCL_SCHEDULER_HANDLED_COMMANDS,
    HCL_SCHEDULER_HANDLED_EVENTS,
};
use dali2rust_platform::wall_clock::{LocalCivilTime, TimeError, TimeSource, WallClock};
use dali2rust_test_support::wait_until;

const TEST_TICK_MS: u64 = 300;
const TEST_COMMAND_TIMEOUT_MS: u64 = 200;
const YEAR_DAY: u16 = 172;
const WEDNESDAY: u8 = 2;
const ALL_DAYS: u8 = 0b0111_1111;

struct StubClock {
    local: Mutex<Option<LocalCivilTime>>,
}

impl StubClock {
    fn at(minutes_since_midnight: u16) -> Self {
        Self::at_weekday(minutes_since_midnight, WEDNESDAY)
    }

    fn at_weekday(minutes_since_midnight: u16, weekday: u8) -> Self {
        Self {
            local: Mutex::new(Some(LocalCivilTime {
                minutes_since_midnight,
                weekday,
                year_day: YEAR_DAY,
                utc_offset_minutes: 180,
            })),
        }
    }

    fn unanchored() -> Self {
        Self {
            local: Mutex::new(None),
        }
    }

    fn set_local(&self, minutes_since_midnight: u16, year_day: u16) {
        *self.local.lock().expect("clock") = Some(LocalCivilTime {
            minutes_since_midnight,
            weekday: WEDNESDAY,
            year_day,
            utc_offset_minutes: 180,
        });
    }
}

impl WallClock for StubClock {
    fn now_ms(&self) -> Option<u64> {
        self.local.lock().expect("clock").map(|_| 1_750_000_000_000)
    }
    fn source(&self) -> TimeSource {
        TimeSource::Manual
    }
    fn set_manual_ms(&self, _unix_ms: u64) -> Result<(), TimeError> {
        Ok(())
    }
    fn timezone(&self) -> String {
        "UTC0".to_string()
    }
    fn set_timezone(&self, _posix_tz: &str) -> Result<(), TimeError> {
        Ok(())
    }
    fn local(&self) -> Option<LocalCivilTime> {
        *self.local.lock().expect("clock")
    }
}

#[derive(Default)]
struct StubRegistry {
    schedules: Vec<HclScheduleView>,
    membership: Vec<(u8, u16)>,
}

impl AdapterReadPort for StubRegistry {
    fn adapter_count(&self) -> u8 {
        1
    }
    fn adapter_view(&self, _adapter_id: u8) -> Option<AdapterView> {
        None
    }
    fn list_adapter_views(&self) -> Vec<AdapterView> {
        Vec::new()
    }
}

impl GroupReadPort for StubRegistry {
    fn group_view(&self, _adapter_id: u8, _group_id: u8) -> Option<GroupView> {
        None
    }
    fn list_group_views(&self, _adapter_id: u8) -> Vec<GroupView> {
        Vec::new()
    }
    fn group_membership_matrix_view(&self, _adapter_id: u8) -> Option<GroupMembershipMatrixView> {
        None
    }
    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
        Some(GroupApplySnapshot {
            adapter_id,
            rows: self
                .membership
                .iter()
                .map(|(lamp, applied)| GroupApplyRowView {
                    virtual_lamp_id: *lamp,
                    desired_groups_mask: *applied,
                    applied_groups_mask: *applied,
                    binding_short: Some(*lamp),
                })
                .collect(),
        })
    }

}

impl HclScheduleReadPort for StubRegistry {
    fn hcl_schedule_view(&self, schedule_id: &str) -> Option<HclScheduleView> {
        self.schedules
            .iter()
            .find(|view| view.schedule_id == schedule_id)
            .cloned()
    }
    fn list_hcl_schedule_views(&self) -> Vec<HclScheduleView> {
        self.schedules.clone()
    }
}

fn point(
    offset: i16,
    level_mode: HclLevelMode,
    level: Option<u8>,
    kelvin: Option<u16>,
) -> HclSchedulePointRow {
    HclSchedulePointRow {
        time_ref: HclTimeRef::Absolute,
        offset_minutes: offset,
        level_mode,
        level,
        color_temperature_kelvin: kelvin,
    }
}

fn group_target(adapter_id: u8, groups: &[u8]) -> HclTargetRow {
    HclTargetRow {
        adapter_id,
        scope: HclTargetScope::Group,
        group_mask: groups.iter().fold(0u16, |mask, g| mask | (1u16 << g)),
    }
}

fn broadcast_target(adapter_id: u8) -> HclTargetRow {
    HclTargetRow {
        adapter_id,
        scope: HclTargetScope::Broadcast,
        group_mask: 0,
    }
}

fn schedule(
    schedule_id: &str,
    targets: Vec<HclTargetRow>,
    points: Vec<HclSchedulePointRow>,
) -> HclScheduleView {
    HclScheduleView {
        schedule_id: schedule_id.to_string(),
        enabled: true,
        algorithm: HclAlgorithm::Stepped,
        active_days_mask: ALL_DAYS,
        latitude_microdeg: None,
        longitude_microdeg: None,
        targets,
        points,
    }
}

fn commit_event(
    virtual_lamp_id: u8,
    value_source: RuntimeSource,
    state_setpoint: dali2rust_contracts::msg::LightSetpoint,
) -> dali2rust_contracts::msg::EventEnvelope {
    dali2rust_contracts::bus::event_envelope(
        dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
        0,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::RuntimeStateChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: Some(virtual_lamp_id),
            short_address: Some(0),
            commit_dimensions: state_setpoint.dimensions(),
            state_setpoint,
            commit_source: value_source,
            state_observation: dali2rust_contracts::msg::RuntimeObservation {
                value_source: Some(value_source),
                ..Default::default()
            },
            commit_holds_hcl: true,
        },
    )
}

struct Harness {
    publisher: BusPublisher,
    confirmations: BusSubscriberRx,
    commands: Arc<Mutex<Vec<BusCommandPayload>>>,
    counters: Arc<HclSchedulerCounters>,
    overrides: SharedOverrideLedger,
    _host: BusHost,
    _worker: std::thread::JoinHandle<()>,
    _pump: Option<std::thread::JoinHandle<()>>,
    _silent_rx: Option<BusSubscriberRx>,
    liveness: Arc<dali2rust_platform::liveness::LivenessBeat>,
}

impl Harness {
    fn commands(&self) -> Vec<BusCommandPayload> {
        self.commands.lock().expect("commands").clone()
    }

    fn override_view(&self, schedule_id: &str) -> HclOverrideView {
        HclOverrideLedgerRead::new(Arc::clone(&self.overrides)).hcl_override_view(schedule_id)
    }

    fn publish_manual_commit_on_lamp(&self, virtual_lamp_id: u8) {
        self.publish_commit_on_lamp(virtual_lamp_id, RuntimeSource::Api);
    }

    fn publish_commit_on_lamp(&self, virtual_lamp_id: u8, value_source: RuntimeSource) {
        self.publish_setpoint_commit_on_lamp(
            virtual_lamp_id,
            value_source,
            dali2rust_contracts::msg::LightSetpoint {
                power: PowerState::On,
                level: Some(254),
                color: None,
            },
        );
    }

    fn publish_setpoint_commit_on_lamp(
        &self,
        virtual_lamp_id: u8,
        value_source: RuntimeSource,
        setpoint: dali2rust_contracts::msg::LightSetpoint,
    ) {
        assert_eq!(
            self.publisher.try_publish(
                BusChannel::Events,
                BusFrame::event(commit_event(virtual_lamp_id, value_source, setpoint)),
            ),
            PublishResult::Queued
        );
    }

    fn wait_for_commands(&self, count: usize) -> Vec<BusCommandPayload> {
        wait_until(
            || self.commands.lock().expect("commands").len() >= count,
            Duration::from_secs(3),
        );
        self.commands()
    }

    fn publish_command<P>(&self, correlation_id: u64, payload: P)
    where
        BusCommandPayload: From<P>,
    {
        let envelope = dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            correlation_id,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Rules),
            payload,
        );
        assert_eq!(
            self.publisher
                .try_publish(BusChannel::Commands, BusFrame::command(envelope)),
            PublishResult::Queued
        );
    }

    fn hold(&self, correlation_id: u64, target: HclOverrideTarget) {
        self.publish_command(
            correlation_id,
            HclOverrideHoldCommand {
                registry_adapter_id: 0,
                target,
            },
        );
    }

    fn resume(&self, correlation_id: u64, target: HclOverrideTarget) {
        self.publish_command(
            correlation_id,
            HclOverrideResumeCommand {
                registry_adapter_id: 0,
                target,
            },
        );
    }

    fn clear(&self, correlation_id: u64, schedule_id: &str) {
        self.publish_command(
            correlation_id,
            HclOverrideClearCommand {
                schedule_id: dali2rust_contracts::msg::fixed_text_32(schedule_id),
            },
        );
    }

    fn confirmation_for(
        &self,
        correlation_id: u64,
    ) -> dali2rust_contracts::msg::ConfirmationEnvelope {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(BusFrame::Confirmation(envelope)) =
                self.confirmations.recv_timeout(Duration::from_millis(50))
            {
                if envelope.meta.correlation_id == correlation_id {
                    return (*envelope).clone();
                }
            }
        }
        panic!("no confirmation for {correlation_id}");
    }

    fn confirmation_order(&self, correlation_ids: &[u64]) -> Vec<u64> {
        let mut order = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while order.len() < correlation_ids.len() && std::time::Instant::now() < deadline {
            if let Ok(BusFrame::Confirmation(envelope)) =
                self.confirmations.recv_timeout(Duration::from_millis(50))
            {
                if correlation_ids.contains(&envelope.meta.correlation_id) {
                    order.push(envelope.meta.correlation_id);
                }
            }
        }
        order
    }

    fn wait_for_ticks(&self, extra: u32) {
        let target = self.counters.ticks.load(Ordering::Relaxed) + extra;
        wait_until(
            || self.counters.ticks.load(Ordering::Relaxed) >= target,
            Duration::from_secs(5),
        );
    }
}

fn spawn_confirmation_pump(
    cmd_rx: BusSubscriberRx,
    publisher: BusPublisher,
    seen: Arc<Mutex<Vec<BusCommandPayload>>>,
    status: dali2rust_contracts::msg::DeliveryStatus,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(BusFrame::Command(envelope)) = cmd_rx.recv() {
            seen.lock().expect("commands").push(envelope.payload.clone());
            let confirmation = dali2rust_contracts::bus::build_confirmation_envelope(
                envelope.meta.correlation_id,
                status,
                0,
                dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            );
            let _ = publisher.try_publish(
                BusChannel::Confirmations,
                BusFrame::confirmation(confirmation),
            );
        }
    })
}

const PUMPED_COMMANDS: &[&str] = &[
    "DaliSetTargetStateCommand",
    "DaliRecallLastActiveLevelCommand",
];

fn spawn_harness(registry: StubRegistry, clock: Arc<StubClock>) -> Harness {
    spawn_harness_inner(
        Arc::new(registry),
        clock,
        Some(dali2rust_contracts::msg::DeliveryStatus::Ok),
    )
}

fn spawn_harness_with_failing_confirmations(
    registry: StubRegistry,
    clock: Arc<StubClock>,
) -> Harness {
    spawn_harness_inner(
        Arc::new(registry),
        clock,
        Some(dali2rust_contracts::msg::DeliveryStatus::ExecutionFailed),
    )
}

fn spawn_harness_without_confirmations(registry: StubRegistry, clock: Arc<StubClock>) -> Harness {
    spawn_harness_inner(Arc::new(registry), clock, None)
}

const SCHEDULER_INBOX: usize = 32;
const DEEP_SCHEDULER_INBOX: usize = 256;

fn spawn_harness_inner(
    registry: Arc<dyn HclSchedulerReadPort>,
    clock: Arc<StubClock>,
    answer: Option<dali2rust_contracts::msg::DeliveryStatus>,
) -> Harness {
    spawn_harness_sized(registry, clock, answer, SCHEDULER_INBOX)
}

fn spawn_harness_sized(
    registry: Arc<dyn HclSchedulerReadPort>,
    clock: Arc<StubClock>,
    answer: Option<dali2rust_contracts::msg::DeliveryStatus>,
    inbox: usize,
) -> Harness {
    let (host, publisher, (rx, conf_rx, cmd_rx, confirmations)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands_and_events(inbox, HCL_SCHEDULER_HANDLED_COMMANDS, HCL_SCHEDULER_HANDLED_EVENTS),
            reg.subscribe_confirmations(32),
            reg.subscribe_commands(64, PUMPED_COMMANDS),
            reg.subscribe_confirmations(32),
        )
    });
    let commands = Arc::new(Mutex::new(Vec::new()));
    let mut silent_rx = None;
    let pump = if let Some(status) = answer {
        Some(spawn_confirmation_pump(
            cmd_rx,
            publisher.clone(),
            Arc::clone(&commands),
            status,
        ))
    } else {
        silent_rx = Some(cmd_rx);
        None
    };
    let counters = Arc::new(HclSchedulerCounters::default());
    let overrides: SharedOverrideLedger = Arc::new(Mutex::new(OverrideLedger::new()));
    let liveness = Arc::new(dali2rust_platform::liveness::LivenessBeat::new(
        "test", 3_000,
    ));
    let worker = spawn_hcl_scheduler_worker(
        rx,
        conf_rx,
        publisher.clone(),
        BusId::default(),
        registry,
        clock,
        Arc::new(CorrelationIdAllocator::new()),
        HclConfig {
            tick_period_ms: TEST_TICK_MS,
            command_timeout_ms: TEST_COMMAND_TIMEOUT_MS,
        },
        Arc::clone(&counters),
        Arc::clone(&overrides),
        Arc::clone(&liveness),
        std::sync::Arc::new(ActiveRole),
        Arc::new(dali2rust_platform::firmware::MaintenanceHold::new()),
    );
    Harness {
        publisher,
        confirmations,
        commands,
        counters,
        overrides,
        _host: host,
        _worker: worker,
        _pump: pump,
        _silent_rx: silent_rx,
        liveness,
    }
}

#[test]
fn a_stepped_point_is_published_once_and_then_held() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "morning",
            vec![group_target(0, &[3])],
            vec![point(360, HclLevelMode::Absolute, Some(80), Some(2700))],
        )],
        ..StubRegistry::default()
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(420)));

    let commands = harness.wait_for_commands(1);
    let BusCommandPayload::DaliSetTargetStateCommand(body) = &commands[0] else {
        panic!("expected a target-state command, got {:?}", commands[0]);
    };
    assert_eq!(body.scope, DaliTargetScope::Group);
    assert_eq!(body.group_id, 3);
    assert_eq!(body.setpoint.level, Some(80));
    assert_eq!(body.setpoint.power, PowerState::On);
    assert_eq!(
        body.setpoint
            .color
            .as_ref()
            .map(|c| c.color_temperature_kelvin),
        Some(2700)
    );

    harness.wait_for_ticks(3);
    assert_eq!(harness.commands().len(), 1, "a held point is not resent");
}

#[test]
fn a_failed_confirmation_is_not_memoized_and_the_point_is_retried() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "morning",
            vec![group_target(0, &[3])],
            vec![point(360, HclLevelMode::Absolute, Some(80), Some(2700))],
        )],
        ..StubRegistry::default()
    };
    let harness =
        spawn_harness_with_failing_confirmations(registry, Arc::new(StubClock::at(420)));

    harness.wait_for_commands(1);
    harness.wait_for_ticks(3);
    assert!(
        harness.commands().len() >= 2,
        "a refused point must be re-derived and re-sent, not memoized as driven \
         (got {} publish(es))",
        harness.commands().len()
    );
    assert!(
        harness.counters.command_failures.load(Ordering::Relaxed) >= 1,
        "a failed confirmation is countable, and it is not a timeout"
    );
    assert_eq!(
        harness.counters.command_timeouts.load(Ordering::Relaxed),
        0,
        "nothing here timed out — the answer arrived and said no"
    );
}

#[test]
fn a_pair_straddling_the_tick_cap_is_deferred_whole_and_replayed_next_tick() {
    let registry = StubRegistry {
        schedules: vec![
            schedule(
                "a-single",
                vec![group_target(0, &[0])],
                vec![point(0, HclLevelMode::Absolute, Some(100), None)],
            ),
            schedule(
                "b-pairs",
                vec![
                    group_target(0, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]),
                    broadcast_target(1),
                ],
                vec![point(0, HclLevelMode::LastActive, None, Some(2700))],
            ),
        ],
        ..StubRegistry::default()
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));

    let commands = harness.wait_for_commands(33);
    assert!(
        commands.iter().any(|payload| matches!(
            payload,
            BusCommandPayload::DaliRecallLastActiveLevelCommand(recall)
                if recall.scope == HclTargetScope::Broadcast
        )),
        "the deferred pair's recall half must reach the wire"
    );
    assert_eq!(
        harness.counters.commands_dropped_cap.load(Ordering::Relaxed),
        2,
        "both halves of the straddling pair are counted refused"
    );
}

#[test]
fn a_ramp_step_that_encodes_to_the_same_mirek_is_not_republished() {
    let clock = Arc::new(StubClock::at(600));
    let mut ramp = schedule(
        "ramp",
        vec![group_target(0, &[3])],
        vec![
            point(600, HclLevelMode::None, None, Some(6000)),
            point(700, HclLevelMode::None, None, Some(6500)),
        ],
    );
    ramp.algorithm = HclAlgorithm::Interpolated;
    let harness = spawn_harness(
        StubRegistry {
            schedules: vec![ramp],
            ..StubRegistry::default()
        },
        Arc::clone(&clock),
    );
    harness.wait_for_commands(1);

    clock.set_local(601, YEAR_DAY);
    harness.wait_for_ticks(3);
    assert_eq!(
        harness.commands().len(),
        1,
        "a wire-identical colour must be held, not republished"
    );

    clock.set_local(650, YEAR_DAY);
    harness.wait_for_commands(2);
}

#[test]
fn the_midnight_roll_forgets_what_was_published_and_the_new_day_reasserts() {
    let clock = Arc::new(StubClock::at(600));
    let registry = StubRegistry {
        schedules: vec![schedule(
            "all-day",
            vec![group_target(0, &[3])],
            vec![point(0, HclLevelMode::Absolute, Some(80), None)],
        )],
        ..StubRegistry::default()
    };
    let harness = spawn_harness(registry, Arc::clone(&clock));
    harness.wait_for_commands(1);

    clock.set_local(600, YEAR_DAY + 1);
    let commands = harness.wait_for_commands(2);
    assert_eq!(commands.len(), 2, "the new day reasserts the held point once");
    harness.wait_for_ticks(3);
    assert_eq!(
        harness.commands().len(),
        2,
        "and then the point is held again for the rest of the day"
    );
}

#[test]
fn an_unanchored_clock_keeps_the_scheduler_silent() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "morning",
            vec![broadcast_target(0)],
            vec![point(0, HclLevelMode::Absolute, Some(200), Some(4000))],
        )],
        ..StubRegistry::default()
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::unanchored()));

    wait_until(
        || harness.counters.ticks_time_unsynced.load(Ordering::Relaxed) >= 1,
        Duration::from_secs(3),
    );
    assert!(
        harness.commands().is_empty(),
        "a controller that does not know the time must not drive lights"
    );
    assert!(harness.counters.ticks_time_unsynced.load(Ordering::Relaxed) >= 1);
}

#[test]
fn a_disabled_schedule_publishes_nothing() {
    let mut view = schedule(
        "morning",
        vec![broadcast_target(0)],
        vec![point(0, HclLevelMode::Absolute, Some(200), None)],
    );
    view.enabled = false;
    let harness = spawn_harness(
        StubRegistry {
            schedules: vec![view],
            ..StubRegistry::default()
        },
        Arc::new(StubClock::at(600)),
    );

    wait_until(
        || harness.counters.ticks.load(Ordering::Relaxed) >= 2,
        Duration::from_secs(3),
    );
    assert!(harness.commands().is_empty());
}

#[test]
fn a_day_the_schedule_does_not_run_publishes_nothing() {
    let mut view = schedule(
        "weekend",
        vec![broadcast_target(0)],
        vec![point(0, HclLevelMode::Absolute, Some(200), None)],
    );
    view.active_days_mask = 0b0110_0000;
    let harness = spawn_harness(
        StubRegistry {
            schedules: vec![view],
            ..StubRegistry::default()
        },
        Arc::new(StubClock::at(600)),
    );

    wait_until(
        || harness.counters.ticks.load(Ordering::Relaxed) >= 2,
        Duration::from_secs(3),
    );
    assert!(harness.commands().is_empty());
}

#[test]
fn a_weekday_the_mask_cannot_name_keeps_the_scheduler_alive_and_silent() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "morning",
            vec![group_target(0, &[3])],
            vec![point(0, HclLevelMode::Absolute, Some(80), None)],
        )],
        ..StubRegistry::default()
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at_weekday(600, 8)));

    harness.wait_for_ticks(3);
    assert!(
        harness.commands().is_empty(),
        "a day the mask cannot name never runs a schedule"
    );
}

#[test]
fn last_active_sends_the_colour_and_the_recall_to_the_same_target() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "evening",
            vec![group_target(0, &[5])],
            vec![point(0, HclLevelMode::LastActive, None, Some(2200))],
        )],
        ..StubRegistry::default()
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(1200)));

    let commands = harness.wait_for_commands(2);
    let BusCommandPayload::DaliSetTargetStateCommand(color) = &commands[0] else {
        panic!("colour first, got {:?}", commands[0]);
    };
    assert_eq!(color.setpoint.power, PowerState::Unknown, "brightness untouched");
    assert_eq!(color.setpoint.level, None);

    let BusCommandPayload::DaliRecallLastActiveLevelCommand(recall) = &commands[1] else {
        panic!("recall second, got {:?}", commands[1]);
    };
    assert_eq!(recall.scope, HclTargetScope::Group);
    assert_eq!(recall.group_id, Some(5));
}

#[test]
fn one_schedule_fans_out_across_groups_and_adapters() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "whole-house",
            vec![group_target(0, &[1, 5]), broadcast_target(1)],
            vec![point(0, HclLevelMode::Absolute, Some(120), None)],
        )],
        ..StubRegistry::default()
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));

    let commands = harness.wait_for_commands(3);
    let mut seen: Vec<(u8, u8, u8)> = commands
        .iter()
        .filter_map(|payload| match payload {
            BusCommandPayload::DaliSetTargetStateCommand(body) => {
                Some((body.registry_adapter_id, body.scope as u8, body.group_id))
            }
            _ => None,
        })
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![
            (0, DaliTargetScope::Group as u8, 1),
            (0, DaliTargetScope::Group as u8, 5),
            (1, DaliTargetScope::Broadcast as u8, 0),
        ]
    );
}

#[test]
fn two_schedules_claiming_one_group_send_a_single_command() {
    let registry = StubRegistry {
        schedules: vec![
            schedule(
                "a-early",
                vec![group_target(0, &[7])],
                vec![point(0, HclLevelMode::Absolute, Some(50), None)],
            ),
            schedule(
                "b-late",
                vec![group_target(0, &[7])],
                vec![point(0, HclLevelMode::Absolute, Some(200), None)],
            ),
        ],
        ..StubRegistry::default()
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));

    let commands = harness.wait_for_commands(1);
    harness.wait_for_ticks(2);
    assert_eq!(harness.commands().len(), 1, "the duplicate target coalesced");
    let BusCommandPayload::DaliSetTargetStateCommand(body) = &commands[0] else {
        panic!("expected a target-state command");
    };
    assert_eq!(body.setpoint.level, Some(200), "the later schedule id wins");
}

fn group_three_registry() -> StubRegistry {
    let membership = 1u16 << 3;
    StubRegistry {
        schedules: vec![schedule(
            "morning",
            vec![group_target(0, &[3])],
            vec![point(0, HclLevelMode::Absolute, Some(80), None)],
        )],
        membership: vec![(1, membership)],
    }
}

fn wait_for_overrides_started(harness: &Harness, count: u32) {
    wait_until(
        || harness.counters.overrides_started.load(Ordering::Relaxed) >= count,
        Duration::from_secs(3),
    );
}

#[test]
fn a_manual_commit_suspends_the_schedule_for_that_group() {
    let harness = spawn_harness(group_three_registry(), Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);

    harness.publish_manual_commit_on_lamp(1);
    wait_for_overrides_started(&harness, 1);

    let before = harness.commands().len();
    harness.wait_for_ticks(3);
    assert_eq!(
        harness.commands().len(),
        before,
        "an overridden target is left alone for the rest of the day"
    );
}

#[test]
fn a_foreign_level_command_leaves_a_colour_only_schedule_driving() {
    let clock = Arc::new(StubClock::at(600));
    let harness = spawn_harness(colour_only_registry(), Arc::clone(&clock));
    harness.wait_for_commands(1);

    harness.publish_manual_commit_on_lamp(1);
    harness.wait_for_ticks(3);

    assert_eq!(
        harness.counters.overrides_started.load(Ordering::Relaxed),
        0,
        "a brightness command is not a claim on a colour curve"
    );
    assert!(!harness.override_view("colour-only").suspended);
    clock.set_local(660, YEAR_DAY);
    harness.wait_for_commands(2);
}

#[test]
fn a_foreign_colour_command_stands_down_a_colour_only_schedule() {
    let clock = Arc::new(StubClock::at(600));
    let harness = spawn_harness(colour_only_registry(), Arc::clone(&clock));
    harness.wait_for_commands(1);

    harness.publish_setpoint_commit_on_lamp(
        1,
        RuntimeSource::Api,
        LightSetpoint {
            power: PowerState::Unknown,
            level: None,
            color: Some(ColorValue {
                mode: ColorMode::Cct,
                color_temperature_kelvin: 3000,
                ..ColorValue::default()
            }),
        },
    );
    wait_for_overrides_started(&harness, 1);
    assert!(harness.override_view("colour-only").suspended);

    let before = harness.commands().len();
    clock.set_local(660, YEAR_DAY);
    harness.wait_for_ticks(3);
    assert_eq!(
        harness.commands().len(),
        before,
        "the next point of an overridden target is not driven today"
    );
}

fn colour_only_registry() -> StubRegistry {
    StubRegistry {
        schedules: vec![schedule(
            "colour-only",
            vec![group_target(0, &[3])],
            vec![
                point(600, HclLevelMode::None, None, Some(4000)),
                point(660, HclLevelMode::None, None, Some(5000)),
            ],
        )],
        membership: vec![(1, 1u16 << 3)],
    }
}

#[test]
fn a_manual_commit_suspends_every_schedule_on_the_shared_group() {
    let membership = 1u16 << 3;
    let registry = StubRegistry {
        schedules: vec![
            schedule(
                "a-early",
                vec![group_target(0, &[3])],
                vec![point(0, HclLevelMode::Absolute, Some(50), None)],
            ),
            schedule(
                "b-late",
                vec![group_target(0, &[3])],
                vec![point(0, HclLevelMode::Absolute, Some(200), None)],
            ),
        ],
        membership: vec![(1, membership)],
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);

    harness.publish_manual_commit_on_lamp(1);
    wait_for_overrides_started(&harness, 2);

    assert!(harness.override_view("a-early").suspended);
    assert!(
        harness.override_view("b-late").suspended,
        "the schedule the coalesce actually drives must stand down too"
    );
    let before = harness.commands().len();
    harness.wait_for_ticks(3);
    assert_eq!(
        harness.commands().len(),
        before,
        "neither schedule may drive the overridden group again today"
    );
}

#[test]
fn a_poller_readback_leaves_the_schedule_running() {
    let harness = spawn_harness(group_three_registry(), Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);

    harness.publish_commit_on_lamp(1, RuntimeSource::Poller);
    harness.wait_for_ticks(3);

    assert_eq!(
        harness.counters.overrides_started.load(Ordering::Relaxed),
        0,
        "a background read is not an override"
    );
    assert!(!harness.override_view("morning").suspended);
}

#[test]
fn a_rule_commit_that_does_not_hold_leaves_the_schedule_running() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "evening",
            vec![group_target(0, &[3, 5])],
            vec![point(0, HclLevelMode::Absolute, Some(80), None)],
        )],
        membership: vec![(1, 1u16 << 3), (2, 1u16 << 5)],
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));
    harness.wait_for_commands(2);

    let level = dali2rust_contracts::msg::LightSetpoint::from_level(200, None);
    let mut spared = commit_event(1, RuntimeSource::Rules, level);
    let dali2rust_contracts::msg::BusEventPayload::RuntimeStateChangedEvent(body) =
        &mut spared.payload
    else {
        unreachable!("commit_event builds a runtime commit");
    };
    body.commit_holds_hcl = false;
    assert_eq!(
        harness.publisher.try_publish(BusChannel::Events, BusFrame::event(spared)),
        PublishResult::Queued
    );
    harness.publish_commit_on_lamp(2, RuntimeSource::Rules);
    wait_for_overrides_started(&harness, 1);

    let view = harness.override_view("evening");
    let groups: Vec<Option<u8>> = view.targets.iter().map(|target| target.group_id).collect();
    assert_eq!(
        groups,
        vec![Some(5)],
        "the rule that held stood group 5 down; the one that said hold_hcl false left group 3 driven"
    );
    assert_eq!(harness.counters.overrides_started.load(Ordering::Relaxed), 1);
}

#[test]
fn a_manual_commit_before_the_schedule_ran_leaves_it_running() {
    let membership = 1u16 << 1;
    let registry = StubRegistry {
        schedules: vec![schedule(
            "evening",
            vec![broadcast_target(0)],
            vec![point(1080, HclLevelMode::Absolute, Some(80), None)],
        )],
        membership: vec![(1, membership)],
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));
    harness.wait_for_ticks(2);
    assert!(harness.commands().is_empty(), "the curve has not started");

    harness.publish_manual_commit_on_lamp(1);
    harness.wait_for_ticks(3);

    assert_eq!(
        harness.counters.overrides_started.load(Ordering::Relaxed),
        0,
        "a target the schedule never drove cannot have been overridden"
    );
    assert!(!harness.override_view("evening").suspended);
}

#[test]
fn the_override_reports_which_target_stopped_and_when() {
    let harness = spawn_harness(group_three_registry(), Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);
    harness.publish_manual_commit_on_lamp(1);
    wait_for_overrides_started(&harness, 1);

    let view = harness.override_view("morning");
    assert!(view.suspended);
    assert_eq!(view.since_local_minutes, Some(600), "10:00, the stub clock");
    assert_eq!(view.targets.len(), 1);
    assert_eq!(view.targets[0].scope, HclTargetScope::Group);
    assert_eq!(view.targets[0].group_id, Some(3));
}

#[test]
fn a_flood_of_events_during_a_publish_window_is_bounded_and_counted() {
    let membership = 1u16 << 3;
    let registry = StubRegistry {
        schedules: vec![schedule(
            "wide",
            vec![group_target(0, &[1, 2, 3, 4, 5, 6, 7, 8])],
            vec![point(0, HclLevelMode::Absolute, Some(120), None)],
        )],
        membership: vec![(1, membership)],
    };
    let harness = spawn_harness_without_confirmations(registry, Arc::new(StubClock::at(600)));

    let stop = Arc::new(AtomicBool::new(false));
    let flood = spawn_commit_flood(harness.publisher.clone(), Arc::clone(&stop));
    wait_until(
        || harness.counters.deferred_dropped_cap.load(Ordering::Relaxed) >= 1,
        Duration::from_secs(10),
    );
    stop.store(true, Ordering::Relaxed);
    flood.join().expect("flood thread");

    wait_for_overrides_started(&harness, 1);
    assert!(harness.override_view("wide").suspended);
    harness.wait_for_ticks(1);
}

fn spawn_commit_flood(
    publisher: BusPublisher,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let _ = publisher.try_publish(
                BusChannel::Events,
                BusFrame::event(commit_event(
                    1,
                    RuntimeSource::Api,
                    LightSetpoint::from_level(254, None),
                )),
            );
            std::thread::yield_now();
        }
    })
}

#[test]
fn a_clear_command_lifts_the_override_and_the_next_tick_drives_again() {
    let harness = spawn_harness(group_three_registry(), Arc::new(StubClock::at(600)));
    let published = harness.wait_for_commands(1).len();
    harness.publish_manual_commit_on_lamp(1);
    wait_for_overrides_started(&harness, 1);

    let clear = dali2rust_contracts::bus::command_envelope(
        dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
        7,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Api),
        HclOverrideClearCommand {
            schedule_id: dali2rust_contracts::msg::fixed_text_32("morning"),
        },
    );
    assert_eq!(
        harness
            .publisher
            .try_publish(BusChannel::Commands, BusFrame::command(clear)),
        PublishResult::Queued
    );
    wait_until(
        || harness.counters.overrides_reset.load(Ordering::Relaxed) >= 1,
        Duration::from_secs(3),
    );

    assert!(!harness.override_view("morning").suspended);
    let after = harness.wait_for_commands(published + 1);
    assert!(after.len() > published, "the schedule drives the group again");
}

#[test]
fn deleting_a_schedule_takes_its_override_flags_with_it() {
    let harness = spawn_harness(group_three_registry(), Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);
    harness.publish_manual_commit_on_lamp(1);
    wait_for_overrides_started(&harness, 1);
    assert!(harness.override_view("morning").suspended);

    let removal = dali2rust_contracts::bus::event_envelope(
        dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
        0,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Registry),
        dali2rust_contracts::msg::HclScheduleChangedEvent {
            schedule_id: dali2rust_contracts::msg::fixed_text_32("morning"),
            removed: true,
            enabled: false,
        },
    );
    assert_eq!(
        harness
            .publisher
            .try_publish(BusChannel::Events, BusFrame::event(removal)),
        PublishResult::Queued
    );
    wait_until(
        || !harness.override_view("morning").suspended,
        Duration::from_secs(3),
    );
    assert_eq!(
        harness.counters.overrides_reset.load(Ordering::Relaxed),
        1,
        "the lifted flag is counted, not silently dropped"
    );
}

fn schedule_changed(schedule_id: &str, removed: bool) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
        0,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Registry),
        dali2rust_contracts::msg::HclScheduleChangedEvent {
            schedule_id: dali2rust_contracts::msg::fixed_text_32(schedule_id),
            removed,
            enabled: !removed,
        },
    ))
}

#[test]
fn an_edit_the_tick_already_drove_leaves_the_override_gate_armed() {
    let harness = spawn_harness(group_three_registry(), Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);

    assert_eq!(
        harness
            .publisher
            .try_publish(BusChannel::Events, schedule_changed("morning", false)),
        PublishResult::Queued
    );
    harness.publish_manual_commit_on_lamp(1);
    wait_for_overrides_started(&harness, 1);

    assert!(
        harness.override_view("morning").suspended,
        "the tick drove the edited schedule before its change event was read, so a manual \
         command after it is an override of that schedule"
    );
    let before = harness.commands().len();
    harness.wait_for_ticks(3);
    assert_eq!(
        harness.commands().len(),
        before,
        "the operator's level stands; the schedule does not drive over it"
    );
}

#[test]
fn a_disabled_schedule_on_a_driven_target_collects_no_override_flag() {
    let mut paused = schedule(
        "paused",
        vec![group_target(0, &[3])],
        vec![point(0, HclLevelMode::Absolute, Some(40), None)],
    );
    paused.enabled = false;
    let mut registry = group_three_registry();
    registry.schedules.push(paused);
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);

    harness.publish_manual_commit_on_lamp(1);
    wait_for_overrides_started(&harness, 1);

    assert!(harness.override_view("morning").suspended);
    assert!(
        !harness.override_view("paused").suspended,
        "a schedule that drives nothing today cannot be overridden, and a flag raised now would \
         keep it silent for the rest of the day after it is enabled again"
    );
}

struct EditableRegistry {
    base: StubRegistry,
    schedules: Mutex<Vec<HclScheduleView>>,
}

impl EditableRegistry {
    fn new(base: StubRegistry) -> Self {
        let schedules = Mutex::new(base.schedules.clone());
        Self { base, schedules }
    }

    fn edit(&self, schedules: Vec<HclScheduleView>) {
        *self.schedules.lock().expect("schedules") = schedules;
    }
}

impl AdapterReadPort for EditableRegistry {
    fn adapter_count(&self) -> u8 {
        self.base.adapter_count()
    }
    fn adapter_view(&self, adapter_id: u8) -> Option<AdapterView> {
        self.base.adapter_view(adapter_id)
    }
    fn list_adapter_views(&self) -> Vec<AdapterView> {
        self.base.list_adapter_views()
    }
}

impl GroupReadPort for EditableRegistry {
    fn group_view(&self, adapter_id: u8, group_id: u8) -> Option<GroupView> {
        self.base.group_view(adapter_id, group_id)
    }
    fn list_group_views(&self, adapter_id: u8) -> Vec<GroupView> {
        self.base.list_group_views(adapter_id)
    }
    fn group_membership_matrix_view(&self, adapter_id: u8) -> Option<GroupMembershipMatrixView> {
        self.base.group_membership_matrix_view(adapter_id)
    }
    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
        self.base.group_apply_snapshot(adapter_id)
    }
}

impl HclScheduleReadPort for EditableRegistry {
    fn hcl_schedule_view(&self, schedule_id: &str) -> Option<HclScheduleView> {
        self.list_hcl_schedule_views()
            .into_iter()
            .find(|view| view.schedule_id == schedule_id)
    }
    fn list_hcl_schedule_views(&self) -> Vec<HclScheduleView> {
        self.schedules.lock().expect("schedules").clone()
    }
}

#[test]
fn an_edit_no_tick_has_driven_yet_leaves_the_target_unarmed_until_it_is_published() {
    let registry = Arc::new(EditableRegistry::new(group_three_registry()));
    let harness = spawn_harness_inner(
        Arc::clone(&registry) as Arc<dyn HclSchedulerReadPort>,
        Arc::new(StubClock::at(600)),
        Some(dali2rust_contracts::msg::DeliveryStatus::Ok),
    );
    let published = harness.wait_for_commands(1).len();

    registry.edit(vec![schedule(
        "morning",
        vec![group_target(0, &[3])],
        vec![point(0, HclLevelMode::Absolute, Some(120), None)],
    )]);
    assert_eq!(
        harness
            .publisher
            .try_publish(BusChannel::Events, schedule_changed("morning", false)),
        PublishResult::Queued
    );
    harness.publish_manual_commit_on_lamp(1);
    harness.wait_for_commands(published + 1);

    assert_eq!(
        harness.counters.overrides_started.load(Ordering::Relaxed),
        0,
        "the edit had not been driven when the manual command landed, so it overrode nothing"
    );
    harness.publish_manual_commit_on_lamp(1);
    wait_for_overrides_started(&harness, 1);
    assert!(
        harness.override_view("morning").suspended,
        "once the edited point is on the wire, a manual command overrides it"
    );
}

struct ActiveRole;

impl dali2rust_domain::registry::DaliSettingsReadPort for ActiveRole {
    fn dali_settings_view(&self) -> dali2rust_domain::registry::DaliSettingsView {
        dali2rust_domain::registry::DaliSettingsView {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: true,
            device_short_address: None,
        }
    }
}

#[test]
fn a_tick_that_is_publishing_keeps_beating() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "long-window",
            vec![group_target(0, &[1, 2, 3, 4, 5])],
            vec![point(360, HclLevelMode::Absolute, Some(80), Some(2700))],
        )],
        ..StubRegistry::default()
    };
    let harness = spawn_harness_without_confirmations(registry, Arc::new(StubClock::at(420)));

    let published = |n: u32| {
        wait_until(
            || harness.counters.commands_published.load(Ordering::Relaxed) >= n,
            Duration::from_secs(5),
        );
    };
    published(1);
    let window_start = std::time::Instant::now();
    published(4);
    let window_age = window_start.elapsed();
    let beat_age = harness
        .liveness
        .age_ms(dali2rust_platform::liveness::monotonic_ms())
        .expect("the worker has beaten at least once");

    assert!(
        window_age >= Duration::from_millis(400),
        "the window must actually be open for this to prove anything, was {window_age:?}"
    );
    assert!(
        u128::from(beat_age) * 2 < window_age.as_millis(),
        "a publishing tick must keep stamping: beat is {beat_age} ms old inside a \
         window {window_age:?} old, so it was stamped once at the top and left to age"
    );
    assert!(
        harness.counters.command_timeouts.load(Ordering::Relaxed) >= 1,
        "the window has to be built from real unanswered commands"
    );
}

const MOSCOW_LATITUDE_MICRODEG: i32 = 55_755_800;
const MOSCOW_LONGITUDE_MICRODEG: i32 = 37_617_300;

fn group_levels(commands: &[BusCommandPayload]) -> Vec<(u8, Option<u8>)> {
    commands
        .iter()
        .filter_map(|payload| match payload {
            BusCommandPayload::DaliSetTargetStateCommand(body)
                if body.scope == DaliTargetScope::Group =>
            {
                Some((body.group_id, body.setpoint.level))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_sunrise_point_resolves_from_the_location_next_to_a_schedule_with_no_state_yet() {
    let located = HclScheduleView {
        latitude_microdeg: Some(MOSCOW_LATITUDE_MICRODEG),
        longitude_microdeg: Some(MOSCOW_LONGITUDE_MICRODEG),
        ..schedule(
            "dawn",
            vec![group_target(0, &[2])],
            vec![
                point(0, HclLevelMode::Absolute, Some(40), None),
                HclSchedulePointRow {
                    time_ref: HclTimeRef::Sunrise,
                    ..point(0, HclLevelMode::Absolute, Some(200), None)
                },
            ],
        )
    };
    let not_yet = schedule(
        "late",
        vec![group_target(0, &[4])],
        vec![point(1380, HclLevelMode::Absolute, Some(90), None)],
    );
    let clock = Arc::new(StubClock::at(180));
    let harness = spawn_harness(
        StubRegistry {
            schedules: vec![located, not_yet],
            ..StubRegistry::default()
        },
        Arc::clone(&clock),
    );

    let before_sunrise = harness.wait_for_commands(1);
    assert_eq!(
        group_levels(&before_sunrise),
        vec![(2, Some(40))],
        "03:00 on the solstice is before a Moscow sunrise: the midnight point holds"
    );

    clock.set_local(720, YEAR_DAY);
    let after_sunrise = harness.wait_for_commands(2);
    assert_eq!(
        group_levels(&after_sunrise[1..]),
        vec![(2, Some(200))],
        "by noon the sunrise point, placed by the astronomy, has passed"
    );

    harness.wait_for_ticks(2);
    assert!(
        group_levels(&harness.commands()).iter().all(|(group, _)| *group != 4),
        "a schedule whose only point is still ahead sends nothing"
    );
}

fn evening_before_its_curve() -> StubRegistry {
    StubRegistry {
        schedules: vec![schedule(
            "evening",
            vec![broadcast_target(0)],
            vec![point(1080, HclLevelMode::Absolute, Some(80), None)],
        )],
        membership: vec![(1, 1u16 << 1)],
    }
}

#[test]
fn a_hold_stands_down_a_target_the_schedule_has_not_driven_yet() {
    let harness = spawn_harness(evening_before_its_curve(), Arc::new(StubClock::at(600)));
    harness.wait_for_ticks(2);
    assert!(harness.commands().is_empty(), "the curve has not started");

    harness.hold(71, HclOverrideTarget::Broadcast);
    let confirmation = harness.confirmation_for(71);

    assert_eq!(
        confirmation.status,
        dali2rust_contracts::msg::DeliveryStatus::Ok,
        "the hold confirms on the bus like a clear does"
    );
    let view = harness.override_view("evening");
    assert!(
        view.suspended,
        "an explicit hold does not wait for the schedule to have driven the target today"
    );
    assert_eq!(view.since_local_minutes, Some(600));
    assert_eq!(harness.counters.overrides_started.load(Ordering::Relaxed), 1);
}

#[test]
fn a_hold_on_a_lamp_stands_down_exactly_the_groups_that_hold_the_lamp() {
    let registry = StubRegistry {
        schedules: vec![schedule(
            "morning",
            vec![group_target(0, &[3, 5])],
            vec![point(0, HclLevelMode::Absolute, Some(80), None)],
        )],
        membership: vec![(1, 1u16 << 3)],
    };
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));
    harness.wait_for_commands(2);

    harness.hold(72, HclOverrideTarget::VirtualLamp { virtual_lamp_id: 1 });
    harness.confirmation_for(72);

    assert_eq!(
        suspended_groups(&harness, "morning"),
        vec![Some(3)],
        "the lamp's group stands down, the group without it keeps its schedule"
    );
}

fn suspended_groups(harness: &Harness, schedule_id: &str) -> Vec<Option<u8>> {
    harness
        .override_view(schedule_id)
        .targets
        .iter()
        .map(|target| target.group_id)
        .collect()
}

fn five_target_registry() -> StubRegistry {
    StubRegistry {
        schedules: vec![schedule(
            "wide",
            vec![broadcast_target(0), group_target(0, &[5, 7, 8, 9])],
            vec![point(0, HclLevelMode::Absolute, Some(80), None)],
        )],
        membership: vec![(1, (1u16 << 7) | (1u16 << 8)), (2, 1u16 << 9)],
    }
}

#[test]
fn a_group_hold_stands_down_what_a_commit_on_its_members_would() {
    let harness = spawn_harness(five_target_registry(), Arc::new(StubClock::at(600)));

    harness.hold(74, HclOverrideTarget::Group { group_id: 7 });
    harness.confirmation_for(74);
    assert_eq!(
        suspended_groups(&harness, "wide"),
        vec![None, Some(7), Some(8)],
        "lamp 1 is in groups 7 and 8 and in the broadcast; group 9 has no lamp of group 7"
    );

    harness.hold(75, HclOverrideTarget::Group { group_id: 5 });
    harness.confirmation_for(75);
    assert_eq!(
        suspended_groups(&harness, "wide"),
        vec![None, Some(5), Some(7), Some(8)],
        "a group without members stands down only itself"
    );
}

#[test]
fn a_broadcast_hold_stands_down_every_target_on_its_adapter() {
    let harness = spawn_harness(five_target_registry(), Arc::new(StubClock::at(600)));

    harness.hold(76, HclOverrideTarget::Broadcast);
    harness.confirmation_for(76);

    assert_eq!(
        suspended_groups(&harness, "wide"),
        vec![None, Some(5), Some(7), Some(8), Some(9)]
    );
}

#[test]
fn a_resume_lifts_only_the_targets_its_argument_covers() {
    let harness = spawn_harness(five_target_registry(), Arc::new(StubClock::at(600)));
    harness.hold(77, HclOverrideTarget::Broadcast);
    harness.confirmation_for(77);

    harness.resume(78, HclOverrideTarget::Group { group_id: 9 });
    harness.confirmation_for(78);
    assert_eq!(
        suspended_groups(&harness, "wide"),
        vec![Some(5), Some(7), Some(8)],
        "group 9 and the broadcast target its lamp is in"
    );

    harness.resume(79, HclOverrideTarget::VirtualLamp { virtual_lamp_id: 1 });
    harness.confirmation_for(79);
    assert_eq!(suspended_groups(&harness, "wide"), vec![Some(5)], "the groups of lamp 1");

    harness.resume(80, HclOverrideTarget::Broadcast);
    assert_eq!(
        harness.confirmation_for(80).status,
        dali2rust_contracts::msg::DeliveryStatus::Ok
    );
    assert!(suspended_groups(&harness, "wide").is_empty());
    assert_eq!(harness.counters.overrides_reset.load(Ordering::Relaxed), 5);
}

#[test]
fn a_hold_stands_down_a_colour_curve_that_a_level_command_would_leave_running() {
    let harness = spawn_harness(colour_only_registry(), Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);

    harness.hold(73, HclOverrideTarget::Group { group_id: 3 });
    harness.confirmation_for(73);

    assert!(
        harness.override_view("colour-only").suspended,
        "an explicit hold takes every dimension the schedule drives"
    );
}

#[test]
fn a_hold_without_a_clock_is_refused_by_name_and_holds_nothing() {
    let harness = spawn_harness(group_three_registry(), Arc::new(StubClock::unanchored()));

    harness.hold(74, HclOverrideTarget::Group { group_id: 3 });
    let confirmation = harness.confirmation_for(74);

    assert_eq!(
        confirmation.status,
        dali2rust_contracts::msg::DeliveryStatus::ExecutionFailed
    );
    let error = confirmation.confirmation.error.expect("a named refusal");
    assert_eq!(error.message.as_str(), "time_unsynced");
    assert!(!harness.override_view("morning").suspended);
}

#[test]
fn a_hold_leaves_a_schedule_that_does_not_run_today_alone() {
    let mut paused = schedule(
        "paused",
        vec![group_target(0, &[3])],
        vec![point(0, HclLevelMode::Absolute, Some(40), None)],
    );
    paused.enabled = false;
    let mut registry = group_three_registry();
    registry.schedules.push(paused);
    let harness = spawn_harness(registry, Arc::new(StubClock::at(600)));
    harness.wait_for_commands(1);

    harness.hold(75, HclOverrideTarget::Group { group_id: 3 });
    harness.confirmation_for(75);

    assert!(harness.override_view("morning").suspended);
    assert!(
        !harness.override_view("paused").suspended,
        "a schedule that drives nothing today has nothing to hold"
    );
}

const SIXTEEN_GROUPS: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const LAMP_IN_GROUP_THREE: u8 = 1;
const LAMP_IN_NO_GROUP: u8 = 2;
const TICK_EVENT_BUFFER: u32 = 64;
const PAST_THE_BUFFER: u32 = 36;
const SMALL_OVERFLOW: u32 = 6;

fn sixteen_group_registry() -> StubRegistry {
    StubRegistry {
        schedules: vec![schedule(
            "wide",
            vec![group_target(0, &SIXTEEN_GROUPS)],
            vec![point(0, HclLevelMode::Absolute, Some(120), None)],
        )],
        membership: vec![(LAMP_IN_GROUP_THREE, 1u16 << 3)],
    }
}

fn publishing_tick_on_a_deep_inbox() -> Harness {
    let harness = spawn_harness_sized(
        Arc::new(sixteen_group_registry()),
        Arc::new(StubClock::at(600)),
        None,
        DEEP_SCHEDULER_INBOX,
    );
    wait_until(
        || harness.counters.commands_published.load(Ordering::Relaxed) >= 1,
        Duration::from_secs(3),
    );
    harness
}

fn publish_commits(harness: &Harness, count: u32, virtual_lamp_id: u8, source: RuntimeSource) {
    for _ in 0..count {
        let event = commit_event(virtual_lamp_id, source, LightSetpoint::from_level(254, None));
        let frame = BusFrame::event(event);
        wait_until(
            || harness.publisher.try_publish(BusChannel::Events, frame.clone()) == PublishResult::Queued,
            Duration::from_secs(3),
        );
    }
}

fn wait_for_dropped(harness: &Harness, dropped: u32) {
    wait_until(
        || harness.counters.deferred_dropped_cap.load(Ordering::Relaxed) >= dropped,
        Duration::from_secs(3),
    );
}

#[test]
fn a_hold_and_a_clear_during_a_flooded_tick_are_served_at_once_and_in_order() {
    let harness = publishing_tick_on_a_deep_inbox();
    publish_commits(&harness, TICK_EVENT_BUFFER + PAST_THE_BUFFER, LAMP_IN_NO_GROUP, RuntimeSource::Api);
    wait_for_dropped(&harness, PAST_THE_BUFFER);

    harness.hold(76, HclOverrideTarget::Group { group_id: 3 });
    harness.clear(77, "wide");
    let order = harness.confirmation_order(&[76, 77]);
    let timeouts_when_served = harness.counters.command_timeouts.load(Ordering::Relaxed);

    assert_eq!(
        harness.counters.deferred_dropped_cap.load(Ordering::Relaxed),
        PAST_THE_BUFFER,
        "the tick's event buffer was full before the commands came"
    );
    assert_eq!(order, vec![76, 77], "both commands land, the hold first");
    assert!(
        timeouts_when_served < SIXTEEN_GROUPS.len() as u32,
        "both were answered while the tick was still publishing"
    );
    assert_eq!(harness.counters.overrides_started.load(Ordering::Relaxed), 1);
    assert!(
        !harness.override_view("wide").suspended,
        "the clear came after the hold, so the schedule runs"
    );
}

#[test]
fn the_schedules_own_commits_are_not_kept_for_after_the_tick() {
    let harness = publishing_tick_on_a_deep_inbox();
    publish_commits(&harness, TICK_EVENT_BUFFER + PAST_THE_BUFFER, LAMP_IN_GROUP_THREE, RuntimeSource::Hcl);
    publish_commits(&harness, TICK_EVENT_BUFFER + SMALL_OVERFLOW, LAMP_IN_NO_GROUP, RuntimeSource::Api);
    wait_for_dropped(&harness, SMALL_OVERFLOW);

    assert!(
        harness.counters.command_timeouts.load(Ordering::Relaxed) < SIXTEEN_GROUPS.len() as u32,
        "every commit arrived while the tick was publishing"
    );
    assert_eq!(
        harness.counters.deferred_dropped_cap.load(Ordering::Relaxed),
        SMALL_OVERFLOW,
        "a commit that cannot override takes no room in the tick's event buffer"
    );
}
