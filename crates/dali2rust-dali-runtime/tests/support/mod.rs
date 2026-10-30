#![allow(dead_code, reason = "Each `tests/*.rs` is its own binary and compiles this whole module, so a helper one binary does not call is \"never used\" from that binary's point of view — the same reason `dali2rust-registry-runtime`'s support module carries this, and the same pressure that produced the local copies in the first place")]

use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx, PublishResult,
};
use dali2rust_contracts::msg::{
    BusEventPayload, DaliAttributeGroup, DaliReadAttributesCommand, DaliSetTargetStateCommand,
    DaliTargetScope, ErrorCode, LightSetpoint, MemoryBankReadPreset, OperationWorkerSignal,
    OperationWorkerSignalEvent, Origin, PowerState,
};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::banks::IdentityRead;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_domain::registry::{
    AdapterSnapshot, InputInstanceTypeReadPort, RegistryReadPort, VirtualLampSnapshot,
};
use dali2rust_platform::dali::{DaliTransport, WireActivity};
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::{hold_the_events_ingress_full, try_wait_until};

pub const SHORT: u8 = 5;
pub const LEVEL: u8 = 220;
pub const EVENT_BUDGET: Duration = Duration::from_secs(5);
pub const SIGNAL_DEADLINE: Duration = Duration::from_secs(10);
const HELD_EVENTS_INGRESS: usize = 4;
const WORKER_INBOX: usize = 16;
const SIGNAL_INBOX: usize = 64;

pub fn standard_frame(command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: DaliAddress::short(SHORT).expect("short"),
        command,
    }
    .to_forward_frame()
    .raw()
}

pub fn dapc_frame() -> u16 {
    standard_frame(StandardCommand::DirectArcPower { level: LEVEL })
}

pub fn read_frame(correlation_id: u64, origin: Origin, groups: DaliAttributeGroup) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        Some(origin),
        DaliReadAttributesCommand {
            registry_adapter_id: 0,
            short_address: SHORT,
            attribute_groups_mask: groups.mask_bit(),
            memory_banks: MemoryBankReadPreset::None,
        },
    ))
}

pub fn api_read_frame(correlation_id: u64) -> BusFrame {
    read_frame(correlation_id, Origin::Api, DaliAttributeGroup::RuntimeStatus)
}

pub fn poller_read_frame(correlation_id: u64) -> BusFrame {
    read_frame(
        correlation_id,
        Origin::Poller,
        DaliAttributeGroup::RuntimeStatus,
    )
}

pub fn dt8_read_frame(correlation_id: u64) -> BusFrame {
    read_frame(correlation_id, Origin::Api, DaliAttributeGroup::Dt8Color)
}

pub fn target_state_frame(
    correlation_id: u64,
    origin: Origin,
    scope: DaliTargetScope,
    short_address: u8,
    level: u8,
) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        Some(origin),
        DaliSetTargetStateCommand {
            registry_adapter_id: 0,
            scope,
            short_address,
            virtual_lamp_id: 0,
            group_id: 0,
            setpoint: LightSetpoint {
                power: PowerState::On,
                level: Some(level),
                color: None,
            },
            hold_hcl: true,
        },
    ))
}

pub fn interactive_target_state_frame(correlation_id: u64) -> BusFrame {
    target_state_frame(
        correlation_id,
        Origin::Api,
        DaliTargetScope::Short,
        SHORT,
        LEVEL,
    )
}

pub fn spawn_mock_worker(
    worker_cmd: BusSubscriberRx,
    transport: &Arc<Mutex<MockDaliTransport>>,
    publisher: &BusPublisher,
    counters: &Arc<DaliWorkerCounters>,
    interactive: Arc<WireActivity>,
) -> std::thread::JoinHandle<()> {
    spawn_dali_worker(
        worker_cmd,
        DaliController::new(Arc::clone(transport), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::new(RegistryStore::with_adapter_count(1)),
        publisher.clone(),
        BusId::default(),
        Arc::clone(counters),
        interactive,
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    )
}

pub fn spawn_worker_on<T: DaliTransport + Send + 'static>(
    commands: BusSubscriberRx,
    publisher: &BusPublisher,
    transport: &Arc<Mutex<T>>,
    counters: &Arc<DaliWorkerCounters>,
) -> std::thread::JoinHandle<()> {
    spawn_dali_worker(
        commands,
        DaliController::new(Arc::clone(transport), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::new(RegistryStore::with_adapter_count(1)),
        publisher.clone(),
        BusId::default(),
        Arc::clone(counters),
        Arc::new(WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    )
}

pub fn spawn_signal_bus() -> (BusHost, BusPublisher, BusSubscriberRx, BusSubscriberRx) {
    let (host, publisher, (worker_commands, signals)) = BusHost::spawn(
        BusConfig {
            events_ingress: HELD_EVENTS_INGRESS,
            ..BusConfig::default()
        },
        |reg| {
            (
                reg.subscribe_commands(WORKER_INBOX, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_events(SIGNAL_INBOX, &["OperationWorkerSignalEvent"]),
            )
        },
    );
    (host, publisher, worker_commands, signals)
}

pub fn next_signal(
    signals: &BusSubscriberRx,
    correlation_id: u64,
) -> Option<OperationWorkerSignalEvent> {
    next_signal_within(signals, correlation_id, SIGNAL_DEADLINE)
}

pub fn next_signal_within(
    signals: &BusSubscriberRx,
    correlation_id: u64,
    window: Duration,
) -> Option<OperationWorkerSignalEvent> {
    let deadline = Instant::now() + window;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())?;
        match signals.recv_timeout(remaining) {
            Ok(BusFrame::Event(envelope)) => match &envelope.payload {
                BusEventPayload::OperationWorkerSignalEvent(body)
                    if body.workflow_correlation_id == correlation_id =>
                {
                    return Some(body.clone())
                }
                _ => {}
            },
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("signal tap died"),
        }
    }
}

pub struct DemoBusOperation {
    pub frame: fn() -> BusFrame,
    pub correlation_id: u64,
    pub announces_start: bool,
    pub lost: &'static str,
}

fn terminal_signal_on_the_demo_bus(
    operation: &DemoBusOperation,
    hold_the_ingress: bool,
) -> OperationWorkerSignalEvent {
    let (_host, publisher, worker_commands, signals) = spawn_signal_bus();
    let transport = Arc::new(Mutex::new(SimDaliTransport::demo_bus()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_worker_on(worker_commands, &publisher, &transport, &counters);
    let wire_held = transport.lock().expect("sim");
    let published = publisher.try_publish(BusChannel::Commands, (operation.frame)());
    assert_eq!(published, PublishResult::Queued);
    if operation.announces_start {
        assert_eq!(
            next_signal(&signals, operation.correlation_id).map(|started| started.signal),
            Some(OperationWorkerSignal::WorkerStarted),
            "the operation never started"
        );
    }
    let ingress = hold_the_ingress.then(|| hold_the_events_ingress_full(&publisher));
    drop(wire_held);
    if let Some(ingress) = ingress {
        assert!(
            try_wait_until(
                || counters.event_publish_failed.load(Ordering::Relaxed) > 0,
                SIGNAL_DEADLINE
            ),
            "inconclusive: the events ingress was held full, yet no required publish gave up"
        );
        ingress.release();
    }
    next_signal(&signals, operation.correlation_id).expect("the operation never ended")
}

pub fn assert_a_lost_fact_fails_the_operation(operation: &DemoBusOperation) {
    let open = terminal_signal_on_the_demo_bus(operation, false);
    assert_eq!(
        open.signal,
        OperationWorkerSignal::WorkerSucceeded,
        "control: with the events ingress open the operation ends {open:?}"
    );
    let held = terminal_signal_on_the_demo_bus(operation, true);
    let error = held.error.as_ref().map(|error| (error.code, error.message.as_str()));
    assert_eq!(
        (held.signal, error),
        (
            OperationWorkerSignal::WorkerFailed,
            Some((ErrorCode::CommandsIngressOverload, operation.lost))
        ),
        "the full ingress refused a fact the operation owed the registry, yet it ended \
         {held:?} — a green operation over a registry that never heard it"
    );
}

#[derive(Default)]
pub struct ReadPortDouble {
    pub policy_armed: bool,
    pub known_shorts: Vec<u8>,
    pub identity: IdentityRead,
    pub held_after: Option<HeldAfter>,
}

#[derive(Clone, Copy)]
pub enum SentFrames {
    Of24(usize),
    Of16(usize),
    AfterQueries(usize),
}

pub struct HeldAfter {
    pub transport: Arc<Mutex<MockDaliTransport>>,
    pub sent: SentFrames,
    pub passive: bool,
    pub queries: std::sync::atomic::AtomicUsize,
}

impl ReadPortDouble {
    fn held(&self, passive: bool) -> bool {
        self.held_after.as_ref().is_some_and(|held| {
            if held.passive != passive {
                return false;
            }
            let mock = held.transport.lock().expect("mock lock");
            match held.sent {
                SentFrames::Of24(frames) => mock.sent_frames24().len() >= frames,
                SentFrames::Of16(frames) => mock.sent_frames().len() >= frames,
                SentFrames::AfterQueries(queries) => held.queries.fetch_add(1, Ordering::SeqCst) >= queries,
            }
        })
    }
}

impl InputInstanceTypeReadPort for ReadPortDouble {
    fn input_instance_type(&self, _adapter_id: u8, _short: u8, _instance: u8) -> Option<u8> {
        None
    }
}

impl RegistryReadPort for ReadPortDouble {
    fn application_controller_active(&self) -> bool {
        !self.held(true)
    }

    fn controller_quiescent(&self) -> bool {
        self.held(false)
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
        self.known_shorts.clone()
    }

    fn first_free_short_address(&self, _adapter_id: u8) -> Option<u8> {
        None
    }

    fn physical_device_identity(&self, _adapter_id: u8, _short: u8) -> IdentityRead {
        self.identity
    }
}
