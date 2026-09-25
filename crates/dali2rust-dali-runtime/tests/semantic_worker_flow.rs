use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{
    BusCommandPayload, BusEventPayload, CommissioningStep, DaliProgramTarget, DaliTargetScope,
    DeliveryStatus, DiscoveryMode, ErrorCode, GroupMembershipAction, HclTargetScope,
    LightSetpoint, MemoryBankReadPreset, OperationWorkerSignal, PowerState, SceneProgramAction,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{spawn_dali_worker, DaliWorkerCounters};
use dali2rust_domain::dali::commands::{DaliCommand, DaliResponse};
use dali2rust_domain::dali::controller::{DaliApplicationController, DaliProductController};
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::ses::DaliSession;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_domain::registry::{AdapterSnapshot, RegistryReadPort, VirtualLampSnapshot};

fn setpoint(level: u8) -> LightSetpoint {
    let mut sp = LightSetpoint::default();
    sp.power = PowerState::On;
    sp.level = level;
    sp
}

#[derive(Clone, Copy)]
enum ControllerMode {
    NoAnswer,
    DiscoveryDt6 { short_address: u8 },
    Error,
}

struct TestController {
    mode: ControllerMode,
    session: DaliSession,
    sent: Arc<std::sync::Mutex<Vec<DaliCommand>>>,
    sent24: Arc<std::sync::Mutex<Vec<[u8; 3]>>>,
}

impl TestController {
    fn new(mode: ControllerMode, sent: Arc<std::sync::Mutex<Vec<DaliCommand>>>) -> Self {
        Self {
            mode,
            session: DaliSession::new(),
            sent,
            sent24: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl DaliProductController for TestController {
    type Error = ();

    fn send_command(&mut self, cmd: &DaliCommand) -> Result<DaliResponse, Self::Error> {
        self.sent.lock().expect("sent lock").push(*cmd);
        match self.mode {
            ControllerMode::NoAnswer => Ok(DaliResponse::NoAnswer),
            ControllerMode::Error => Err(()),
            ControllerMode::DiscoveryDt6 { short_address } => match *cmd {
                DaliCommand::Standard {
                    address: DaliAddress::Short(sa),
                    command: StandardCommand::QueryDeviceType,
                } if sa == short_address => Ok(DaliResponse::Answer(6)),
                DaliCommand::Standard {
                    address: DaliAddress::Short(sa),
                    command: StandardCommand::QueryNextDeviceType,
                } if sa == short_address => Ok(DaliResponse::NoAnswer),
                _ => Ok(DaliResponse::NoAnswer),
            },
        }
    }

    fn session(&self) -> &DaliSession {
        &self.session
    }
}

impl DaliApplicationController for TestController {
    fn send_raw(
        &mut self,
        _frame: ForwardFrame,
        _expects_backward: bool,
    ) -> Result<DaliResponse, Self::Error> {
        Ok(DaliResponse::NoAnswer)
    }

    fn send_frame24(
        &mut self,
        frame: [u8; 3],
        _expects_backward: bool,
    ) -> Result<DaliResponse, dali2rust_domain::dali::controller::Frame24Fault> {
        self.sent24.lock().expect("sent24 lock").push(frame);
        Ok(DaliResponse::NoAnswer)
    }

    fn supports_frame24(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct TestReadPort {
    enabled: bool,
    known_shorts: Vec<u8>,
    bindings: HashMap<(u8, u8), u8>,
    gear_features: HashMap<(u8, u8), u8>,
    repair_forbidden: std::collections::HashSet<(u8, u8)>,
    passive: bool,
}

impl RegistryReadPort for TestReadPort {
    fn application_controller_active(&self) -> bool {
        !self.passive
    }

    fn virtual_lamp_snapshot(&self, _adapter_id: u8, _virtual_lamp_id: u8) -> VirtualLampSnapshot {
        VirtualLampSnapshot::default()
    }

    fn virtual_lamp_binding_short(&self, adapter_id: u8, virtual_lamp_id: u8) -> Option<u8> {
        self.bindings.get(&(adapter_id, virtual_lamp_id)).copied()
    }

    fn physical_dt8_gear_features(&self, adapter_id: u8, short_address: u8) -> Option<u8> {
        self.gear_features.get(&(adapter_id, short_address)).copied()
    }

    fn physical_dt8_rgbwaf_control_assert_allowed(&self, _adapter_id: u8, _short: u8) -> bool {
        false
    }

    fn physical_dt8_auto_activation_repair_allowed(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> bool {
        !self.repair_forbidden.contains(&(adapter_id, short_address))
    }

    fn apply_on_discovery_armed(&self, _adapter_id: u8) -> bool {
        false
    }

    fn adapter_snapshot(&self, _adapter_id: u8) -> AdapterSnapshot {
        AdapterSnapshot {
            enabled: self.enabled,
        }
    }

    fn known_physical_short_addresses(&self, _adapter_id: u8) -> Vec<u8> {
        self.known_shorts.clone()
    }

    fn first_free_short_address(&self, _adapter_id: u8) -> Option<u8> {
        (0..=63).find(|short| !self.known_shorts.contains(short))
    }
}

struct WorkerHarness {
    publisher: dali2rust_bus::BusPublisher,
    cmd_tap_rx: std::sync::mpsc::Receiver<BusFrame>,
    conf_rx: std::sync::mpsc::Receiver<BusFrame>,
    ev_rx: std::sync::mpsc::Receiver<BusFrame>,
    counters: Arc<DaliWorkerCounters>,
    sent_commands: Arc<std::sync::Mutex<Vec<DaliCommand>>>,
    sent_frames24: Arc<std::sync::Mutex<Vec<[u8; 3]>>>,
    _worker: std::thread::JoinHandle<()>,
    _host: dali2rust_bus::BusHost,
}

impl WorkerHarness {
    fn new(read_port: Arc<dyn RegistryReadPort>, controller_mode: ControllerMode) -> Self {
        let (host, publisher, (worker_cmd, cmd_tap, conf_tap, ev_tap)) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                (
                    reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                    reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                    reg.subscribe_confirmations(32),
                    reg.subscribe_events(64, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
                )
            },
        );
        let counters = Arc::new(DaliWorkerCounters::default());
        let sent_commands = Arc::new(std::sync::Mutex::new(Vec::new()));
        let controller = TestController::new(controller_mode, Arc::clone(&sent_commands));
        let sent_frames24 = Arc::clone(&controller.sent24);
        let worker = spawn_dali_worker(
            worker_cmd,
            controller,
            dali2rust_dali_runtime::DaliRuntimeConfig::default(),
            read_port,
            publisher.clone(),
            BusId::default(),
            Arc::clone(&counters),
            Arc::new(dali2rust_platform::dali::WireActivity::new()),
            Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
        );
        Self {
            publisher,
            cmd_tap_rx: cmd_tap,
            conf_rx: conf_tap,
            ev_rx: ev_tap,
            counters,
            sent_commands,
            sent_frames24,
            _worker: worker,
            _host: host,
        }
    }

    fn publish(&self, cmd: dali2rust_contracts::msg::CommandEnvelope) {
        assert_eq!(
            self.publisher.try_publish(BusChannel::Commands, BusFrame::command(cmd)),
            PublishResult::Queued
        );
    }

    fn recv_confirmation_for(
        &self,
        correlation_id: u64,
    ) -> std::sync::Arc<dali2rust_contracts::msg::ConfirmationEnvelope> {
        for _ in 0..40 {
            let frame = self
                .conf_rx
                .recv_timeout(Duration::from_millis(100))
                .expect("confirmation");
            if let BusFrame::Confirmation(conf) = frame {
                if conf.meta.correlation_id == correlation_id {
                    return conf;
                }
            }
        }
        panic!("no confirmation for correlation {correlation_id}");
    }

    fn recv_event_matching(
        &self,
        correlation_id: u64,
        pred: impl Fn(&BusEventPayload) -> bool,
    ) -> std::sync::Arc<dali2rust_contracts::msg::EventEnvelope> {
        for _ in 0..80 {
            let frame = self
                .ev_rx
                .recv_timeout(Duration::from_millis(100))
                .expect("event");
            if let BusFrame::Event(ev) = frame {
                if ev.meta.correlation_id == correlation_id && pred(&ev.payload) {
                    return ev;
                }
            }
        }
        panic!("no matching event for correlation {correlation_id}");
    }

    fn recv_command_matching(
        &self,
        correlation_id: u64,
        pred: impl Fn(&BusCommandPayload) -> bool,
    ) -> dali2rust_contracts::msg::CommandEnvelope {
        for _ in 0..80 {
            let frame = self
                .cmd_tap_rx
                .recv_timeout(Duration::from_millis(100))
                .expect("command");
            if let BusFrame::Command(cmd) = frame {
                if cmd.meta.correlation_id == correlation_id && pred(&cmd.payload) {
                    return cmd.as_ref().clone();
                }
            }
        }
        panic!("no matching command for correlation {correlation_id}");
    }
}

#[test]
fn semantic_short_target_state_publishes_expanded_applied_event_without_runtime_update() {
    let harness = WorkerHarness::new(Arc::new(TestReadPort { enabled: true, ..Default::default() }), ControllerMode::NoAnswer);
    let short = 9u8;
    let corr = 33u64;
    harness.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliSetTargetStateCommand::for_short(0, short, &setpoint(55))));

    let applied_event = harness.recv_event_matching(corr, |payload| {
        matches!(payload, BusEventPayload::DaliTargetStateAppliedEvent(_))
    });
    let BusEventPayload::DaliTargetStateAppliedEvent(body) = &applied_event.payload else {
        panic!("expected applied event");
    };
    assert_eq!(body.scope, DaliTargetScope::Short);
    assert_eq!(body.short_address, Some(short));
    assert_eq!(body.virtual_lamp_id, None);
    assert_eq!(body.setpoint.level, 55);
    assert!(body.dapc_applied);
    let original = harness.recv_command_matching(corr, |payload| {
        matches!(payload, BusCommandPayload::DaliSetTargetStateCommand(_))
    });
    assert!(matches!(
        original.payload,
        BusCommandPayload::DaliSetTargetStateCommand(_)
    ));
    assert!(
        harness
            .cmd_tap_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err(),
        "expected no runtime update command from the worker"
    );
    assert_eq!(
        harness
            .sent_commands
            .lock()
            .expect("sent lock")
            .as_slice(),
        &[DaliCommand::Standard {
            address: DaliAddress::short(short).expect("short"),
            command: StandardCommand::DirectArcPower { level: 55 },
        }]
    );
    assert_eq!(
        harness
            .counters
            .semantic_set_target_state_handled
            .load(Ordering::Relaxed),
        1
    );
}

#[test]
fn adapter_disabled_wire_command_returns_named_refusal_confirmation() {
    let harness = WorkerHarness::new(Arc::new(TestReadPort::default()), ControllerMode::NoAnswer);
    let corr = 71u64;
    harness.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: 0x01, command: 0xFE, repeat_count: 0, raw_mode: false, raw_expects_backward: false }));

    let conf = harness.recv_confirmation_for(corr);
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    let error = conf.confirmation.error.as_ref().expect("product error");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.message.as_str(), "adapter_disabled");
    assert_eq!(harness.counters.execution_failed.load(Ordering::Relaxed), 1);
}

fn short_target() -> DaliProgramTarget {
    DaliProgramTarget::Short { short_address: 5 }
}

fn refused_operations() -> Vec<BusCommandPayload> {
    use dali2rust_contracts::msg as m;
    vec![
        m::DaliDiscoverDevicesCommand { mode: DiscoveryMode::RefreshKnown, registry_adapter_id: 0 }.into(),
        m::DaliReadAttributesCommand { registry_adapter_id: 0, short_address: 5, attribute_groups_mask: 1 << 1, memory_banks: MemoryBankReadPreset::None }.into(),
        m::DaliReadMemoryBankCommand { registry_adapter_id: 0, short_address: 5, bank: 1, start: 0, length: 1 }.into(),
        m::DaliWriteAttributesCommand { registry_adapter_id: 0, short_address: 5, fade_time_ms: Some(200), fade_rate: None, power_on_level: None, system_failure_level: None, extended_fade_time_ms: None, tc_coolest_mirek: None, tc_warmest_mirek: None, min_level: None, max_level: None, dimming_curve: None, signals_operation: true }.into(),
        m::DaliIdentifyDeviceCommand { registry_adapter_id: 0, short_address: 5, operation_key: Default::default() }.into(),
        m::DaliReplaceDeviceCommand { registry_adapter_id: 0, failed_short_address: 5, replacement_short_address: 6, restore_metadata_and_overrides: false, restore_attributes: false, restore_groups: false, restore_scenes: false, operation_key: Default::default() }.into(),
        m::DaliAddressingCommand { registry_adapter_id: 0, short_address: 5, new_short_address: 6, verify_after_program: true, operation_key: Default::default() }.into(),
        m::DaliProgramGroupMembershipCommand { registry_adapter_id: 0, target: short_target(), group_id: 1, action: GroupMembershipAction::Add }.into(),
        m::DaliProgramSceneCommand { registry_adapter_id: 0, target: short_target(), scene_id: 3, action: SceneProgramAction::Clear, target_state: None }.into(),
        m::Dali103ScanCommand { registry_adapter_id: 0 }.into(),
        m::Dali103CommissionCommand { registry_adapter_id: 0, include_addressed: false }.into(),
        m::Dali103InstanceConfigureCommand { registry_adapter_id: 0, short_address: 5, instance_number: 0, patch_mask: 1, event_scheme: 2, event_filter: [0; 3], event_priority: 4, instance_groups: [None; 3], timer_multipliers: [None; 4], instance_enabled: true }.into(),
        m::Dali103IdentifyCommand { registry_adapter_id: 0, short_address: 5 }.into(),
        m::Dali103FeedbackConfigureCommand { registry_adapter_id: 0, short_address: 5, instance_number: 0, patch_mask: 1, timing: 0, active_brightness: 0, active_colour: 0, inactive_brightness: 0, inactive_colour: 0, opcode_map: 0 }.into(),
    ]
}

fn refused_requests() -> Vec<BusCommandPayload> {
    use dali2rust_contracts::msg as m;
    vec![
        m::DaliCommandPayload { wire_address: 0x01, command: 0xFE, repeat_count: 0, raw_mode: false, raw_expects_backward: false }.into(),
        m::DaliCommissioningStepCommand { registry_adapter_id: 0, step: CommissioningStep::Terminate, scope: None, short_address: None, search_address: None }.into(),
        m::DaliSetTargetStateCommand::for_virtual_lamp(0, 4, &setpoint(40)).into(),
        m::DaliRecallSceneCommand { registry_adapter_id: 0, scope: DaliTargetScope::Broadcast, short_address: 0, group_id: 0, scene_id: 3 }.into(),
        m::DaliRecallLastActiveLevelCommand { registry_adapter_id: 0, scope: HclTargetScope::Broadcast, group_id: None }.into(),
        m::DaliStopFadeCommand { registry_adapter_id: 0, scope: DaliTargetScope::Broadcast, virtual_lamp_id: 0, short_address: 0, group_id: 0 }.into(),
        m::Dali103FeedbackDriveCommand { registry_adapter_id: 0, action: 0, short_address: Some(5), feature_number: None, feature_group: None, selected_group: 0, opcode_map: 0 }.into(),
        m::Dali103HandoverCommand { registry_adapter_id: 0, peer_short_address: 7 }.into(),
        m::Dali103ArbitrationProbeCommand { registry_adapter_id: 0 }.into(),
        m::DaliBusHealthProbeCommand { registry_adapter_id: 0 }.into(),
    ]
}

fn assert_nothing_reached_the_wire(harness: &WorkerHarness, kind: &str) {
    assert!(harness.sent_commands.lock().unwrap().is_empty(), "{kind} sent a 16-bit frame");
    assert!(harness.sent_frames24.lock().unwrap().is_empty(), "{kind} sent a 24-bit frame");
}

fn assert_adapter_disabled(code: ErrorCode, message: &str, kind: &str) {
    assert_eq!(code, ErrorCode::Conflict, "{kind}");
    assert_eq!(message, "adapter_disabled", "{kind}");
}

fn envelope_on_adapter_0(corr: u64, payload: BusCommandPayload) -> dali2rust_contracts::msg::CommandEnvelope {
    dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), payload)
}

#[test]
fn adapter_disabled_semantic_commands_publish_worker_failed_signal() {
    let disabled: Arc<dyn RegistryReadPort> = Arc::new(TestReadPort::default());
    for (corr, payload) in (81u64..).zip(refused_operations()) {
        let kind = payload.variant_name();
        let harness = WorkerHarness::new(Arc::clone(&disabled), ControllerMode::NoAnswer);
        harness.publish(envelope_on_adapter_0(corr, payload));
        let ev = harness.recv_event_matching(corr, |payload| {
            matches!(payload, BusEventPayload::OperationWorkerSignalEvent(_))
        });
        let BusEventPayload::OperationWorkerSignalEvent(body) = &ev.payload else {
            panic!("expected worker signal");
        };
        assert_eq!(body.signal, OperationWorkerSignal::WorkerFailed, "{kind}");
        let error = body.error.as_ref().expect("a refusal names its cause");
        assert_adapter_disabled(error.code, error.message.as_str(), kind);
        assert_nothing_reached_the_wire(&harness, kind);
    }
}

#[test]
fn adapter_disabled_requests_are_refused_by_confirmation_before_the_wire() {
    let disabled: Arc<dyn RegistryReadPort> = Arc::new(TestReadPort::default());
    for (corr, payload) in (121u64..).zip(refused_requests()) {
        let kind = payload.variant_name();
        let harness = WorkerHarness::new(Arc::clone(&disabled), ControllerMode::NoAnswer);
        harness.publish(envelope_on_adapter_0(corr, payload));
        let conf = harness.recv_confirmation_for(corr);
        assert_eq!(conf.status, DeliveryStatus::ExecutionFailed, "{kind}");
        let error = conf.confirmation.error.as_ref().expect("a refusal names its cause");
        assert_adapter_disabled(error.code, error.message.as_str(), kind);
        assert_nothing_reached_the_wire(&harness, kind);
    }
}

#[test]
fn every_command_the_dali_worker_handles_meets_the_adapter_gate() {
    let refused: std::collections::BTreeSet<&str> = refused_operations()
        .iter()
        .chain(refused_requests().iter())
        .map(BusCommandPayload::variant_name)
        .collect();
    let handled: std::collections::BTreeSet<&str> =
        dali2rust_dali_runtime::DALI_WORKER_HANDLED_COMMANDS.iter().copied().collect();
    assert_eq!(
        refused, handled,
        "every command the worker handles drives an adapter's wire, so each one has a \
         disabled-adapter refusal case here"
    );
}

#[test]
fn virtual_lamp_unbound_emits_failed_event_and_confirmation() {
    let harness = WorkerHarness::new(Arc::new(TestReadPort { enabled: true, ..Default::default() }), ControllerMode::NoAnswer);
    let corr = 91u64;
    harness.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliSetTargetStateCommand::for_virtual_lamp(0, 4, &setpoint(40))));

    let failed_event = harness.recv_event_matching(corr, |payload| {
        matches!(payload, BusEventPayload::DaliTargetStateFailedEvent(_))
    });
    assert!(matches!(
        failed_event.payload,
        BusEventPayload::DaliTargetStateFailedEvent(_)
    ));
    let conf = harness.recv_confirmation_for(corr);
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    let error = conf.confirmation.error.as_ref().expect("product error");
    assert_eq!(error.code, ErrorCode::VlUnbound);
    assert_eq!(error.message.as_str(), "vl_unbound");
}

#[test]
fn virtual_lamp_bound_target_state_publishes_expanded_applied_event_without_runtime_update() {
    let mut read_port = TestReadPort {
        enabled: true,
        ..Default::default()
    };
    read_port.bindings.insert((0, 4), 12);
    let harness = WorkerHarness::new(Arc::new(read_port), ControllerMode::NoAnswer);
    let corr = 92u64;
    harness.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliSetTargetStateCommand::for_virtual_lamp(0, 4, &setpoint(42))));

    let applied_event = harness.recv_event_matching(corr, |payload| {
        matches!(payload, BusEventPayload::DaliTargetStateAppliedEvent(_))
    });
    let BusEventPayload::DaliTargetStateAppliedEvent(body) = &applied_event.payload else {
        panic!("expected applied event");
    };
    assert_eq!(body.scope, DaliTargetScope::VirtualLamp);
    assert_eq!(body.virtual_lamp_id, Some(4));
    assert_eq!(body.short_address, Some(12));
    assert_eq!(body.setpoint.level, 42);
    assert!(body.dapc_applied);
    let original = harness.recv_command_matching(corr, |payload| {
        matches!(payload, BusCommandPayload::DaliSetTargetStateCommand(_))
    });
    assert!(matches!(
        original.payload,
        BusCommandPayload::DaliSetTargetStateCommand(_)
    ));
    assert!(
        harness
            .cmd_tap_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err(),
        "expected no runtime update command from the worker"
    );
    assert_eq!(
        harness
            .sent_commands
            .lock()
            .expect("sent lock")
            .as_slice(),
        &[DaliCommand::Standard {
            address: DaliAddress::short(12).expect("short"),
            command: StandardCommand::DirectArcPower { level: 42 },
        }]
    );
    assert_eq!(
        harness
            .counters
            .semantic_set_target_state_handled
            .load(Ordering::Relaxed),
        1
    );
}

#[test]
fn group_target_state_confirms_without_runtime_update() {
    let harness = WorkerHarness::new(Arc::new(TestReadPort { enabled: true, ..Default::default() }), ControllerMode::NoAnswer);
    let corr = 93u64;
    harness.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliSetTargetStateCommand::for_group(0, 3, &setpoint(88))));

    let original = harness.recv_command_matching(corr, |payload| match payload {
        BusCommandPayload::DaliSetTargetStateCommand(body) => body.scope == DaliTargetScope::Group,
        _ => false,
    });
    assert!(matches!(
        original.payload,
        BusCommandPayload::DaliSetTargetStateCommand(_)
    ));

    let conf = harness.recv_confirmation_for(corr);
    assert_eq!(conf.status, DeliveryStatus::Ok);
    let applied_event = harness.recv_event_matching(corr, |payload| {
        matches!(payload, BusEventPayload::DaliTargetStateAppliedEvent(_))
    });
    let BusEventPayload::DaliTargetStateAppliedEvent(body) = &applied_event.payload else {
        panic!("expected applied event");
    };
    assert_eq!(body.scope, DaliTargetScope::Group);
    assert_eq!(body.group_id, Some(3));
    assert_eq!(body.short_address, None);
    assert_eq!(body.setpoint.level, 88);
    assert!(body.dapc_applied);
    assert!(
        harness
            .cmd_tap_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err(),
        "expected no runtime fan-out commands for group target-state"
    );
    assert_eq!(
        harness
            .sent_commands
            .lock()
            .expect("sent lock")
            .as_slice(),
        &[DaliCommand::Standard {
            address: DaliAddress::group(3).expect("group"),
            command: StandardCommand::DirectArcPower { level: 88 },
        }]
    );
}

#[test]
fn group_membership_programming_virtual_lamp_publishes_result_event() {
    let mut read_port = TestReadPort {
        enabled: true,
        ..Default::default()
    };
    read_port.bindings.insert((0, 4), 12);
    let harness = WorkerHarness::new(Arc::new(read_port), ControllerMode::NoAnswer);
    let corr = 94u64;
    harness.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliProgramGroupMembershipCommand { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 4 }, group_id: 6, action: GroupMembershipAction::Add }));

    let event = harness.recv_event_matching(corr, |payload| {
        matches!(payload, BusEventPayload::DaliGroupMembershipProgrammedEvent(_))
    });
    let BusEventPayload::DaliGroupMembershipProgrammedEvent(body) = &event.payload else {
        panic!("expected group membership event");
    };
    assert_eq!(body.registry_adapter_id, 0);
    assert_eq!(body.target, DaliProgramTarget::VirtualLamp { virtual_lamp_id: 4 });
    assert_eq!(body.group_id, 6);
    assert_eq!(body.action, GroupMembershipAction::Add);
    assert_eq!(body.physical_short_address, Some(12));
    assert!(body.error.is_none());
    assert_eq!(body.membership, None);
    let conf = harness.recv_confirmation_for(corr);
    assert_eq!(conf.status, DeliveryStatus::Ok);
    let sent = harness
        .sent_commands
        .lock()
        .expect("sent lock");
    let drive = [
        DaliCommand::Standard {
            address: DaliAddress::short(12).expect("short"),
            command: StandardCommand::AddToGroup { group: 6 },
        },
        DaliCommand::Standard {
            address: DaliAddress::short(12).expect("short"),
            command: StandardCommand::QueryGroups0To7,
        },
        DaliCommand::Standard {
            address: DaliAddress::short(12).expect("short"),
            command: StandardCommand::QueryGroups8To15,
        },
    ];
    let drives = 1 + usize::from(dali2rust_dali_runtime::runtime::executor::PROGRAM_VERIFY_REPAIRS);
    let expected: Vec<_> = std::iter::repeat_with(|| drive.iter().cloned())
        .take(drives)
        .flatten()
        .collect();
    assert_eq!(
        sent.as_slice(),
        expected.as_slice(),
        "expected one drive plus the verify-repair budget of drives"
    );
    assert_eq!(
        harness
            .counters
            .semantic_program_group_membership_handled
            .load(Ordering::Relaxed),
        1
    );
}

#[test]
fn group_membership_programming_unbound_virtual_lamp_returns_vl_unbound() {
    let harness = WorkerHarness::new(Arc::new(TestReadPort { enabled: true, ..Default::default() }), ControllerMode::NoAnswer);
    let corr = 95u64;
    harness.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliProgramGroupMembershipCommand { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 9 }, group_id: 1, action: GroupMembershipAction::Remove }));

    let event = harness.recv_event_matching(corr, |payload| {
        matches!(payload, BusEventPayload::DaliGroupMembershipProgrammedEvent(_))
    });
    let BusEventPayload::DaliGroupMembershipProgrammedEvent(body) = &event.payload else {
        panic!("expected group membership event");
    };
    assert_eq!(body.physical_short_address, None);
    assert_eq!(body.error.as_ref().map(|e| e.code), Some(ErrorCode::VlUnbound));
    assert_eq!(
        body.error.as_ref().map(|e| e.message.as_str()),
        Some("vl_unbound")
    );
    let conf = harness.recv_confirmation_for(corr);
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    let error = conf.confirmation.error.as_ref().expect("product error");
    assert_eq!(error.code, ErrorCode::VlUnbound);
    assert_eq!(error.message.as_str(), "vl_unbound");
    assert!(
        harness
            .sent_commands
            .lock()
            .expect("sent lock")
            .is_empty()
    );
}

#[test]
fn refresh_known_discovery_success_and_failure_publish_expected_events() {
    let success_port = Arc::new(TestReadPort {
        enabled: true,
        known_shorts: vec![17],
        ..Default::default()
    });
    let success = WorkerHarness::new(success_port, ControllerMode::DiscoveryDt6 { short_address: 17 });
    success.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 101, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliDiscoverDevicesCommand { mode: DiscoveryMode::RefreshKnown, registry_adapter_id: 0 }));
    let progress = success.recv_event_matching(101, |payload| {
        matches!(payload, BusEventPayload::DaliDiscoveryProgressEvent(_))
    });
    let BusEventPayload::DaliDiscoveryProgressEvent(body) = &progress.payload else {
        panic!("expected discovery progress");
    };
    assert_eq!(body.short_address, 17);
    let completed = success.recv_event_matching(101, |payload| {
        matches!(payload, BusEventPayload::DaliDiscoveryCompletedEvent(_))
    });
    assert!(matches!(
        completed.payload,
        BusEventPayload::DaliDiscoveryCompletedEvent(_)
    ));

    let fail_port = Arc::new(TestReadPort {
        enabled: true,
        known_shorts: vec![17],
        ..Default::default()
    });
    let failure = WorkerHarness::new(fail_port, ControllerMode::Error);
    failure.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 102, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliDiscoverDevicesCommand { mode: DiscoveryMode::RefreshKnown, registry_adapter_id: 0 }));
    let failed = failure.recv_event_matching(102, |payload| {
        matches!(payload, BusEventPayload::DaliDiscoveryFailedEvent(_))
    });
    assert!(matches!(
        failed.payload,
        BusEventPayload::DaliDiscoveryFailedEvent(_)
    ));
    let worker_failed = failure.recv_event_matching(102, |payload| match payload {
        BusEventPayload::OperationWorkerSignalEvent(body) => {
            body.signal == OperationWorkerSignal::WorkerFailed
        }
        _ => false,
    });
    let BusEventPayload::OperationWorkerSignalEvent(body) = &worker_failed.payload else {
        panic!("expected worker failed signal");
    };
    assert_eq!(
        body.error.as_ref().map(|e| e.code),
        Some(ErrorCode::OperationFailed)
    );
}

#[test]
fn the_worker_stamps_an_applied_fact_from_the_shared_monotonic_scale() {
    let harness = WorkerHarness::new(
        Arc::new(TestReadPort { enabled: true, ..Default::default() }),
        ControllerMode::NoAnswer,
    );
    let before = dali2rust_bsp::monotonic_clock::observation_stamp_ms();
    let corr = 91u64;
    harness.publish(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliSetTargetStateCommand::for_short(0, 9, &setpoint(55))));

    let applied_event = harness.recv_event_matching(corr, |payload| {
        matches!(payload, BusEventPayload::DaliTargetStateAppliedEvent(_))
    });
    let BusEventPayload::DaliTargetStateAppliedEvent(body) = &applied_event.payload else {
        panic!("expected applied event");
    };
    let after = dali2rust_bsp::monotonic_clock::observation_stamp_ms();

    assert!(
        body.applied_at_mono_ms >= before && body.applied_at_mono_ms <= after,
        "producer stamp {} outside [{before}, {after}] — it must come from \
         the same process-wide scale the registry compares against",
        body.applied_at_mono_ms
    );
}

#[test]
fn a_passive_controller_refuses_wire_commands_with_a_named_cause() {
    let passive: Arc<dyn RegistryReadPort> = Arc::new(TestReadPort {
        enabled: true,
        passive: true,
        ..Default::default()
    });

    let harness = WorkerHarness::new(Arc::clone(&passive), ControllerMode::NoAnswer);
    harness.publish(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        181,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::Dali103ScanCommand { registry_adapter_id: 0 },
    ));
    let ev = harness.recv_event_matching(181, |payload| {
        matches!(payload, BusEventPayload::OperationWorkerSignalEvent(_))
    });
    let BusEventPayload::OperationWorkerSignalEvent(body) = &ev.payload else {
        panic!("expected worker signal");
    };
    assert_eq!(body.signal, OperationWorkerSignal::WorkerFailed);
    let error = body.error.as_ref().expect("a failure names its cause");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.message.as_str(), "controller_passive");
    assert!(
        harness.counters.tx_suppressed_passive.load(std::sync::atomic::Ordering::Relaxed) >= 1,
        "a passive controller that merely looks broken is this counter unread"
    );
    assert!(
        harness.sent_commands.lock().unwrap().is_empty(),
        "passive means NO forward frame, not a refused one after the fact"
    );
}

#[test]
fn a_select_feedback_drive_is_one_frame() {
    let harness = WorkerHarness::new(
        Arc::new(TestReadPort { enabled: true, ..Default::default() }),
        ControllerMode::NoAnswer,
    );
    let corr = 191;
    harness.publish(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        corr,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::Dali103FeedbackDriveCommand {
            registry_adapter_id: 0,
            action: 2,
            short_address: None,
            feature_number: None,
            feature_group: Some(7),
            selected_group: 4,
            opcode_map: 0,
        },
    ));
    let conf = harness.recv_confirmation_for(corr);
    assert_eq!(conf.status, DeliveryStatus::Ok, "a drive that went out confirms ok");
    assert_eq!(
        harness.sent_frames24.lock().unwrap().as_slice(),
        &[[0xFF, 0xA7, 0x24]],
        "broadcast, feature-at-instance-group 7, SELECT(4) — and nothing else"
    );
}

#[test]
fn a_passive_controller_refuses_a_feedback_drive_on_the_confirmation() {
    let harness = WorkerHarness::new(
        Arc::new(TestReadPort { enabled: true, passive: true, ..Default::default() }),
        ControllerMode::NoAnswer,
    );
    let corr = 193;
    harness.publish(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        corr,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::Dali103FeedbackDriveCommand {
            registry_adapter_id: 0,
            action: 0,
            short_address: Some(3),
            feature_number: Some(0),
            feature_group: None,
            selected_group: 0,
            opcode_map: 0,
        },
    ));
    let conf = harness.recv_confirmation_for(corr);
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    let error = conf.confirmation.error.as_ref().expect("a refusal names its cause");
    assert_eq!(error.message.as_str(), "controller_passive");
    assert!(harness.sent_frames24.lock().unwrap().is_empty());
}
