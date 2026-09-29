use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{BusEventPayload, DiscoveryMode, OperationWorkerSignal};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{spawn_dali_worker, DaliWorkerCounters, DALI_WORKER_HANDLED_COMMANDS};
use dali2rust_domain::dali::commands::{DaliCommand, DaliResponse};
use dali2rust_domain::dali::controller::{DaliApplicationController, DaliProductController};
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::ses::DaliSession;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_domain::registry::{AdapterSnapshot, RegistryReadPort, VirtualLampSnapshot};
use dali2rust_test_support::{hold_the_events_ingress_full, recv_event_matching, try_wait_until};

const TAPPED_EVENTS: &[&str] = &[
    "DaliDiscoveryProgressEvent",
    "DaliDiscoveryScanReconciledEvent",
    "DaliDiscoveryCompletedEvent",
    "OperationWorkerSignalEvent",
];

const FLEET_SIZE: u8 = 64;

const TERMINAL_DEADLINE: Duration = Duration::from_secs(20);

const STEP_DEADLINE: Duration = Duration::from_secs(10);

fn random_address_of(short: u8) -> u32 {
    0x5C_0000 | (u32::from(short) << 8) | u32::from(short)
}

struct FleetController {
    session: DaliSession,
    search_address: u32,
    withdrawn: Vec<bool>,
    wire: Arc<Mutex<()>>,
}

impl FleetController {
    fn new(wire: Arc<Mutex<()>>) -> Self {
        Self {
            session: DaliSession::new(),
            search_address: 0,
            withdrawn: vec![false; usize::from(FLEET_SIZE)],
            wire,
        }
    }

    fn selected(&self) -> Option<u8> {
        (0..FLEET_SIZE).find(|&short| {
            random_address_of(short) == self.search_address
                && !self.withdrawn[usize::from(short)]
        })
    }

    fn answer_standard(&mut self, short: u8, command: StandardCommand) -> DaliResponse {
        if short >= FLEET_SIZE {
            return DaliResponse::NoAnswer;
        }
        let random = random_address_of(short);
        match command {
            StandardCommand::QueryControlGearPresent => DaliResponse::Answer(0xFF),
            StandardCommand::QueryRandomAddressH => DaliResponse::Answer((random >> 16) as u8),
            StandardCommand::QueryRandomAddressM => DaliResponse::Answer((random >> 8) as u8),
            StandardCommand::QueryRandomAddressL => DaliResponse::Answer(random as u8),
            StandardCommand::QueryDeviceType => DaliResponse::Answer(6),
            StandardCommand::QueryNextDeviceType => DaliResponse::NoAnswer,
            _ => DaliResponse::NoAnswer,
        }
    }

    fn answer_special(&mut self, command: SpecialCommand) -> DaliResponse {
        match command {
            SpecialCommand::SearchAddrH(byte) => {
                self.search_address = (self.search_address & 0x00_FFFF) | (u32::from(byte) << 16);
                DaliResponse::NoAnswer
            }
            SpecialCommand::SearchAddrM(byte) => {
                self.search_address = (self.search_address & 0xFF_00FF) | (u32::from(byte) << 8);
                DaliResponse::NoAnswer
            }
            SpecialCommand::SearchAddrL(byte) => {
                self.search_address = (self.search_address & 0xFF_FF00) | u32::from(byte);
                DaliResponse::NoAnswer
            }
            SpecialCommand::QueryShortAddress => match self.selected() {
                Some(short) => DaliResponse::Answer((short << 1) | 0x01),
                None => DaliResponse::NoAnswer,
            },
            SpecialCommand::Withdraw => {
                if let Some(short) = self.selected() {
                    self.withdrawn[usize::from(short)] = true;
                }
                DaliResponse::NoAnswer
            }
            SpecialCommand::Compare => DaliResponse::NoAnswer,
            _ => DaliResponse::NoAnswer,
        }
    }
}

impl DaliProductController for FleetController {
    type Error = ();

    fn send_command(&mut self, cmd: &DaliCommand) -> Result<DaliResponse, Self::Error> {
        let wire = Arc::clone(&self.wire);
        let _exchange = wire.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(match *cmd {
            DaliCommand::Standard {
                address: DaliAddress::Short(short),
                command,
            } => self.answer_standard(short, command),
            DaliCommand::Special(command) => self.answer_special(command),
            _ => DaliResponse::NoAnswer,
        })
    }

    fn session(&self) -> &DaliSession {
        &self.session
    }
}

impl DaliApplicationController for FleetController {
    fn send_raw(
        &mut self,
        _frame: ForwardFrame,
        _expects_backward: bool,
    ) -> Result<DaliResponse, Self::Error> {
        let _exchange = self.wire.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(DaliResponse::NoAnswer)
    }
}

#[derive(Default)]
struct EnabledAdapter {
    policy_armed: bool,
}

impl RegistryReadPort for EnabledAdapter {
    fn application_controller_active(&self) -> bool {
        true
    }

    fn virtual_lamp_snapshot(&self, _adapter_id: u8, _virtual_lamp_id: u8) -> VirtualLampSnapshot {
        VirtualLampSnapshot::default()
    }

    fn virtual_lamp_binding_short(&self, _adapter_id: u8, _virtual_lamp_id: u8) -> Option<u8> {
        None
    }

    fn physical_dt8_gear_features(&self, _adapter_id: u8, _short_address: u8) -> Option<u8> {
        None
    }

    fn physical_dt8_rgbwaf_control_assert_allowed(&self, _adapter_id: u8, _short: u8) -> bool {
        false
    }

    fn physical_dt8_auto_activation_repair_allowed(&self, _adapter_id: u8, _short: u8) -> bool {
        false
    }

    fn apply_on_discovery_armed(&self, _adapter_id: u8) -> bool {
        self.policy_armed
    }

    fn adapter_snapshot(&self, _adapter_id: u8) -> AdapterSnapshot {
        AdapterSnapshot { enabled: true }
    }

    fn known_physical_short_addresses(&self, _adapter_id: u8) -> Vec<u8> {
        (0..FLEET_SIZE).collect()
    }

    fn first_free_short_address(&self, _adapter_id: u8) -> Option<u8> {
        None
    }
}

struct ScanHarness {
    publisher: dali2rust_bus::BusPublisher,
    ev_rx: std::sync::mpsc::Receiver<BusFrame>,
    policy_rx: std::sync::mpsc::Receiver<BusFrame>,
    counters: Arc<DaliWorkerCounters>,
    wire: Arc<Mutex<()>>,
    _worker: std::thread::JoinHandle<()>,
    _host: BusHost,
}

impl ScanHarness {
    fn new() -> Self {
        Self::with_policy(false)
    }

    fn with_policy(policy_armed: bool) -> Self {
        let (host, publisher, (worker_cmd, ev_tap, policy_tap)) =
            BusHost::spawn(BusConfig::default(), |reg| {
            (
                reg.subscribe_commands(32, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_events(512, TAPPED_EVENTS),
                reg.subscribe_commands(8, &["PolicyApplyExecuteCommand"]),
            )
        });
        let counters = Arc::new(DaliWorkerCounters::default());
        let wire = Arc::new(Mutex::new(()));
        let worker = spawn_dali_worker(
            worker_cmd,
            FleetController::new(Arc::clone(&wire)),
            dali2rust_dali_runtime::DaliRuntimeConfig::default(),
            Arc::new(EnabledAdapter { policy_armed }),
            publisher.clone(),
            BusId::default(),
            Arc::clone(&counters),
            Arc::new(dali2rust_platform::dali::WireActivity::new()),
            Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
        );
        Self {
            publisher,
            ev_rx: ev_tap,
            policy_rx: policy_tap,
            counters,
            wire,
            _worker: worker,
            _host: host,
        }
    }

    fn start_scan(&self, correlation_id: u64, mode: DiscoveryMode) {
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::DaliDiscoverDevicesCommand {
                mode,
                registry_adapter_id: 0,
            },
        );
        assert_eq!(
            self.publisher
                .try_publish(BusChannel::Commands, BusFrame::command(cmd)),
            PublishResult::Queued
        );
    }

    fn hold_the_wire(&self) -> MutexGuard<'_, ()> {
        self.wire.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn events_refused(&self) -> u32 {
        self.publisher.counters_snapshot().events.ingress_overflow
    }

    fn wait_for_started(&self, correlation_id: u64) {
        recv_event_matching(&self.ev_rx, STEP_DEADLINE, |payload| {
            matches!(
                payload,
                BusEventPayload::OperationWorkerSignalEvent(body)
                    if body.workflow_correlation_id == correlation_id
                        && body.signal == OperationWorkerSignal::WorkerStarted
            )
        });
    }

    fn drain_until_terminal(&self, correlation_id: u64) -> Vec<BusEventPayload> {
        let deadline = Instant::now() + TERMINAL_DEADLINE;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            let Ok(BusFrame::Event(ev)) = self.ev_rx.recv_timeout(Duration::from_millis(200))
            else {
                continue;
            };
            if ev.meta.correlation_id != correlation_id {
                continue;
            }
            let terminal = matches!(
                &ev.payload,
                BusEventPayload::OperationWorkerSignalEvent(body)
                    if body.signal != OperationWorkerSignal::WorkerStarted
            );
            seen.push(ev.payload.clone());
            if terminal {
                return seen;
            }
        }
        seen
    }
}

fn count_progress(events: &[BusEventPayload]) -> HashMap<u8, u32> {
    let mut per_short = HashMap::new();
    for event in events {
        if let BusEventPayload::DaliDiscoveryProgressEvent(body) = event {
            *per_short.entry(body.short_address).or_insert(0) += 1;
        }
    }
    per_short
}

#[test]
fn a_full_segment_scan_reports_its_terminal_outcome() {
    let harness = ScanHarness::new();
    harness.start_scan(4200, DiscoveryMode::ScanKnownShortAddresses);
    let events = harness.drain_until_terminal(4200);

    let progress = count_progress(&events);
    assert_eq!(
        progress.len(),
        usize::from(FLEET_SIZE),
        "expected one progress event per device, got {} distinct shorts",
        progress.len()
    );
    assert!(
        progress.values().all(|&n| n == 1),
        "a device was announced more than once: {progress:?}"
    );

    assert!(
        events
            .iter()
            .any(|e| matches!(e, BusEventPayload::DaliDiscoveryScanReconciledEvent(_))),
        "the reconcile summary was lost — the registry's only trigger for ISSUE-4 \
         phantom eviction, and on a full segment it is always the frame right after \
         the last progress event"
    );

    let terminal = events.iter().find_map(|e| match e {
        BusEventPayload::OperationWorkerSignalEvent(body)
            if body.signal != OperationWorkerSignal::WorkerStarted =>
        {
            Some(body.signal)
        }
        _ => None,
    });
    assert_eq!(
        terminal,
        Some(OperationWorkerSignal::WorkerSucceeded),
        "no terminal outcome reached the bus, so the operation could only end by TTL \
         (ISSUE-50); {} events arrived for this run",
        events.len()
    );

    assert_eq!(
        harness.counters.event_publish_failed.load(Ordering::Relaxed),
        0,
        "an event the worker publishes as required-delivery was still lost"
    );
}

#[test]
fn a_full_segment_refresh_reports_its_terminal_outcome() {
    let harness = ScanHarness::new();
    harness.start_scan(4300, DiscoveryMode::RefreshKnown);
    let events = harness.drain_until_terminal(4300);

    assert_eq!(count_progress(&events).len(), usize::from(FLEET_SIZE));
    let terminal = events.iter().any(|e| {
        matches!(
            e,
            BusEventPayload::OperationWorkerSignalEvent(body)
                if body.signal == OperationWorkerSignal::WorkerSucceeded
        )
    });
    assert!(
        terminal,
        "refresh of {FLEET_SIZE} devices produced no terminal outcome"
    );
    assert_eq!(
        harness.counters.event_publish_failed.load(Ordering::Relaxed),
        0
    );
}

fn backoff_schedule() -> Duration {
    Duration::from_millis(dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS.iter().sum())
}

fn wait_for_the_first_refused_progress(harness: &ScanHarness, refused_by_the_fill: u32) {
    assert!(
        try_wait_until(|| harness.events_refused() > refused_by_the_fill, STEP_DEADLINE),
        "the scan's first progress event never met the full events ingress"
    );
}

fn assert_the_retried_scan_succeeded(
    harness: &ScanHarness,
    events: &[BusEventPayload],
    held_for: Duration,
) {
    assert!(
        held_for < backoff_schedule(),
        "inconclusive: the ingress stayed full for {held_for:?}, past the {:?} backoff \
         schedule, because this host starved the test thread; ADR-021 promises nothing \
         past the schedule",
        backoff_schedule()
    );
    assert!(
        harness.counters.event_publish_retried.load(Ordering::Relaxed) > 0,
        "a progress event met the full ingress, yet no retry was counted"
    );
    assert_eq!(
        harness.counters.event_publish_failed.load(Ordering::Relaxed),
        0,
        "a required event was dropped even with the backoff (the ingress was full for \
         {held_for:?})"
    );
    assert_eq!(
        count_progress(events).len(),
        usize::from(FLEET_SIZE),
        "the progress event that met the full ingress was not delivered by its retry"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            BusEventPayload::OperationWorkerSignalEvent(body)
                if body.signal == OperationWorkerSignal::WorkerSucceeded
        )),
        "the terminal outcome was lost after a full ingress; the operation can now only \
         end by TTL — ISSUE-50"
    );
}

#[test]
fn a_terminal_outcome_survives_a_full_events_ingress() {
    let harness = ScanHarness::new();
    let wire_held = harness.hold_the_wire();
    harness.start_scan(4400, DiscoveryMode::RefreshKnown);
    harness.wait_for_started(4400);
    let ingress = hold_the_events_ingress_full(&harness.publisher);
    let refused_by_the_fill = harness.events_refused();
    drop(wire_held);
    let held_since = Instant::now();

    wait_for_the_first_refused_progress(&harness, refused_by_the_fill);
    ingress.release();
    let held_for = held_since.elapsed();
    let events = harness.drain_until_terminal(4400);
    assert_the_retried_scan_succeeded(&harness, &events, held_for);
}

#[test]
fn a_finished_scan_starts_the_policy_apply_when_armed_issue79() {
    let harness = ScanHarness::with_policy(true);
    harness.start_scan(5100, DiscoveryMode::RefreshKnown);
    let events = harness.drain_until_terminal(5100);
    assert!(
        events.iter().any(|e| matches!(
            e,
            BusEventPayload::OperationWorkerSignalEvent(body)
                if body.signal == OperationWorkerSignal::WorkerSucceeded
        )),
        "the scan itself must succeed, or this proves nothing about its epilogue"
    );

    let applies = policy_applies_before_a_probe(&harness);
    assert_eq!(
        applies.len(),
        1,
        "one command however many devices there are (`ADR-007`): {applies:?}"
    );
    let (workflow, key) = &applies[0];
    assert_ne!(
        *workflow, 5100,
        "the apply must not ride the scan's workflow: {applies:?}"
    );
    assert_ne!(*workflow, dali2rust_contracts::CORRELATION_NONE);
    assert!(
        key.starts_with("policy-apply-0-"),
        "the operation key names the adapter and the workflow: {key}"
    );
}

#[test]
fn a_finished_scan_publishes_no_policy_apply_when_disarmed_issue79() {
    let harness = ScanHarness::new();
    harness.start_scan(5200, DiscoveryMode::RefreshKnown);
    let events = harness.drain_until_terminal(5200);
    assert!(events.iter().any(|e| matches!(
        e,
        BusEventPayload::OperationWorkerSignalEvent(body)
            if body.signal == OperationWorkerSignal::WorkerSucceeded
    )));
    let applies = policy_applies_before_a_probe(&harness);
    assert!(
        applies.is_empty(),
        "a disarmed policy must put nothing on the bus: {applies:?}"
    );
}

const POLICY_PROBE_CORRELATION: u64 = 5_999;

fn policy_probe() -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        POLICY_PROBE_CORRELATION,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::PolicyApplyExecuteCommand {
            registry_adapter_id: 0,
            operation_key: dali2rust_contracts::msg::fixed_text_32("probe"),
        },
    ))
}

fn policy_applies_before_a_probe(harness: &ScanHarness) -> Vec<(u64, String)> {
    assert_eq!(
        harness.publisher.try_publish(BusChannel::Commands, policy_probe()),
        PublishResult::Queued
    );
    let deadline = Instant::now() + TERMINAL_DEADLINE;
    let mut applies = Vec::new();
    while let Some(apply) = next_policy_apply(harness, deadline) {
        if apply.0 == POLICY_PROBE_CORRELATION {
            return applies;
        }
        applies.push(apply);
    }
    panic!(
        "the probe published after the terminal never reached the policy tap, so nothing \
         the scan published before its terminal is proven delivered: {applies:?}"
    );
}

fn next_policy_apply(harness: &ScanHarness, deadline: Instant) -> Option<(u64, String)> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match harness.policy_rx.recv_timeout(remaining).ok()? {
        BusFrame::Command(ce) => match &ce.payload {
            dali2rust_contracts::msg::BusCommandPayload::PolicyApplyExecuteCommand(body) => {
                Some((ce.meta.correlation_id, body.operation_key.as_str().to_string()))
            }
            other => panic!("the policy tap carried {other:?}"),
        },
        other => panic!("the policy tap carried {other:?}"),
    }
}
