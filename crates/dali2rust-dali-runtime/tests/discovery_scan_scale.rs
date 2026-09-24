use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{BusEventPayload, DiscoveryMode, OperationWorkerSignal};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{spawn_dali_worker, DaliWorkerCounters};
use dali2rust_domain::dali::commands::{DaliCommand, DaliResponse};
use dali2rust_domain::dali::controller::{DaliApplicationController, DaliProductController};
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::ses::DaliSession;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_domain::registry::{AdapterSnapshot, RegistryReadPort, VirtualLampSnapshot};
use dali2rust_test_support::wait_until;

const TAPPED_EVENTS: &[&str] = &[
    "DaliDiscoveryProgressEvent",
    "DaliDiscoveryScanReconciledEvent",
    "DaliDiscoveryCompletedEvent",
    "OperationWorkerSignalEvent",
];

const FLEET_SIZE: u8 = 64;

const TERMINAL_DEADLINE: Duration = Duration::from_secs(20);

fn random_address_of(short: u8) -> u32 {
    0x5C_0000 | (u32::from(short) << 8) | u32::from(short)
}

struct FleetController {
    session: DaliSession,
    search_address: u32,
    withdrawn: Vec<bool>,
}

impl FleetController {
    fn new() -> Self {
        Self {
            session: DaliSession::new(),
            search_address: 0,
            withdrawn: vec![false; usize::from(FLEET_SIZE)],
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
                reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_events(512, TAPPED_EVENTS),
                reg.subscribe_commands(8, &["PolicyApplyExecuteCommand"]),
            )
        });
        let counters = Arc::new(DaliWorkerCounters::default());
        let worker = spawn_dali_worker(
            worker_cmd,
            FleetController::new(),
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

const FLOOD_CEILING: Duration = TERMINAL_DEADLINE;

fn flood_events_ingress(
    publisher: dali2rust_bus::BusPublisher,
    stop: Arc<std::sync::atomic::AtomicBool>,
    progress: Arc<dali2rust_test_support::FloodProgress>,
) -> std::thread::JoinHandle<dali2rust_test_support::FloodOutcome> {
    std::thread::spawn(move || {
        dali2rust_test_support::flood_in_bursts_observed(
            &publisher,
            BusChannel::Events,
            || {
                BusFrame::event(dali2rust_contracts::bus::event_envelope(
                    SOURCE_ID_UNSPECIFIED,
                    0,
                    BusId::default().0,
                    Some(dali2rust_contracts::msg::Origin::Internal),
                    dali2rust_contracts::msg::DaliEventPayload {
                        wire_address: 0xFE,
                        command: 0x00,
                        repeat_count: 1,
                    },
                ))
            },
            &stop,
            FLOOD_CEILING,
            &progress,
        )
    })
}

#[test]
fn a_terminal_outcome_survives_a_flooded_events_ingress() {
    let harness = ScanHarness::new();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let progress = Arc::new(dali2rust_test_support::FloodProgress::default());
    let flood = flood_events_ingress(
        harness.publisher.clone(),
        Arc::clone(&stop),
        Arc::clone(&progress),
    );

    wait_until(
        || progress.snapshot().drained > 0,
        Duration::from_secs(5),
    );

    let before = progress.snapshot();
    harness.start_scan(4400, DiscoveryMode::RefreshKnown);
    let events = harness.drain_until_terminal(4400);
    wait_until(
        || progress.snapshot().bursts >= before.bursts.saturating_add(2),
        Duration::from_secs(2),
    );
    let window = progress.snapshot().since(before);
    stop.store(true, Ordering::Relaxed);
    let flood = flood.join().expect("flood thread");

    let retried = harness.counters.event_publish_retried.load(Ordering::Relaxed);
    let terminal_seen = events.iter().any(|e| {
        matches!(
            e,
            BusEventPayload::OperationWorkerSignalEvent(body)
                if body.signal == OperationWorkerSignal::WorkerSucceeded
        )
    });
    let dropped = harness.counters.event_publish_failed.load(Ordering::Relaxed);

    if !terminal_seen || dropped != 0 {
        assert!(
            window.burst_pressure_held(),
            "the bus task never drained between bursts DURING THE SCAN on this \
             host (window {window:?} of lifetime {flood:?}) — the stimulus was a \
             saturated bus, the case ADR-021 withholds the guarantee for. This \
             run says nothing about required delivery; it says this host could \
             not schedule the consumer while the experiment ran"
        );
    }

    assert!(
        terminal_seen,
        "the terminal outcome was lost to a full ingress ({retried} retries spent, \
         window {window:?}); the operation can now only end by TTL — ISSUE-50"
    );
    assert_eq!(
        dropped, 0,
        "a required event was dropped even with the backoff (window {window:?})"
    );

    assert!(
        retried > 0,
        "the scan reported its outcome, but no required publish ever met a full \
         ingress ({flood:?}) — the backoff was never exercised, so this run is \
         an inconclusive experiment rather than a pass"
    );
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

    let mut applies = Vec::new();
    while let Ok(BusFrame::Command(ce)) = harness.policy_rx.try_recv() {
        if let dali2rust_contracts::msg::BusCommandPayload::PolicyApplyExecuteCommand(body) =
            &ce.payload
        {
            applies.push((ce.meta.correlation_id, body.operation_key.as_str().to_string()));
        }
    }
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
    assert!(
        harness.policy_rx.try_recv().is_err(),
        "a disarmed policy must put nothing on the bus"
    );
}
