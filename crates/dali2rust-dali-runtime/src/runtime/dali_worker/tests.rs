use dali2rust_domain::dali::commands::{DaliCommand, DaliResponse, StandardCommand};
use dali2rust_domain::dali::controller::{DaliApplicationController, DaliProductController};
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::ses::DaliSession;
use dali2rust_domain::dali::types::DaliAddress;

struct MockController {
    last_command: Option<DaliCommand>,
    response: DaliResponse,
    session: DaliSession,
}

impl MockController {
    fn new(response: DaliResponse) -> Self {
        Self {
            last_command: None,
            response,
            session: DaliSession::new(),
        }
    }
}

impl DaliProductController for MockController {
    type Error = dali2rust_domain::dali::frame::FrameError;

    fn send_command(&mut self, cmd: &DaliCommand) -> Result<DaliResponse, Self::Error> {
        self.last_command = Some(*cmd);
        Ok(self.response)
    }

    fn session(&self) -> &DaliSession {
        &self.session
    }
}

impl DaliApplicationController for MockController {
    fn send_raw(
        &mut self,
        _frame: ForwardFrame,
        _expects_backward: bool,
    ) -> Result<DaliResponse, Self::Error> {
        Ok(DaliResponse::NoAnswer)
    }
}

#[test]
fn mock_controller_baseline() {
    let mut ctrl = MockController::new(DaliResponse::Answer(0x55));
    let cmd = DaliCommand::Standard {
        address: DaliAddress::short(1).unwrap(),
        command: StandardCommand::QueryStatus,
    };
    let resp = ctrl.send_command(&cmd).unwrap();
    assert_eq!(resp, DaliResponse::Answer(0x55));
    assert_eq!(ctrl.last_command, Some(cmd));
}

use super::DaliWorkerCounters;
use dali2rust_bus::{BusConfig, BusHost};
use dali2rust_contracts::msg::{
    DaliAttributeGroup, DaliReadAttributesCommand, LightSetpoint, MemoryBankReadPreset, PowerState,
};
use std::sync::atomic::Ordering;

#[test]
fn test_publish_read_attributes_evidence_publishes_runtime_update_and_event() {
    let config = BusConfig {
        commands_ingress: 1,
        events_ingress: 10,
        ..Default::default()
    };
    let (_host, publisher, ()) = BusHost::spawn(config, |_| {});

    let ar = DaliReadAttributesCommand {
        registry_adapter_id: 0,
        short_address: 5,
        attribute_groups_mask: 0,
        memory_banks: MemoryBankReadPreset::None,
    };

    let setpoint = LightSetpoint {
        power: PowerState::On,
        level: Some(254),
        ..Default::default()
    };

    let execution = crate::runtime::executor::AttributeReadExecution {
        runtime_setpoint: Some(setpoint),
        runtime_observation: Some(dali2rust_contracts::msg::RuntimeObservation::default()),
        random_address: None,
        has_common_102: false,
        has_dt8_color: false,
        dt8_supported: Some(false),
        has_groups: false,
        has_scenes: false,
        has_dt6_led: false,
        has_extended: false,
        dt8_color_mode: dali2rust_contracts::msg::ColorMode::None,
        dt8_xy_capable: false,
        dt8_tc_capable: false,
        dt8_rgb_capable: false,
        c102: Default::default(),
        groups_membership: None,
        scene_levels: None,
        dt8_rgbwaf_capable: false,
        dt8_color: Default::default(),
        dt6: None,
        extended_fade_time_ms: None,
        extended_version_number: None,
        extended_versions: [None; dali2rust_contracts::msg::MAX_EXTENDED_VERSIONS],
        has_scene_colours: false,
        scene_colours: None,
    };

    let counters = DaliWorkerCounters::default();
    let adapter_id = dali2rust_bus::BusId(1);
    let correlation_id = 12345;

    assert!(super::attribute_read::publish_read_attributes_evidence(
        &publisher,
        adapter_id,
        correlation_id,
        super::attribute_read::ReadProvenance {
            value_source: super::RuntimeSource::Api,
            started_mono_ms: 0,
        },
        &ar,
        &execution,
        &counters,
    ));
}

mod measured_colour {
    use dali2rust_contracts::msg::ColorMode;

    use crate::runtime::executor::AttributeReadExecution;

    fn dt8_read(mode: ColorMode) -> AttributeReadExecution {
        AttributeReadExecution {
            runtime_setpoint: None,
            runtime_observation: None,
            random_address: None,
            has_common_102: false,
            has_dt8_color: true,
            dt8_supported: Some(true),
            has_groups: false,
            has_scenes: false,
            has_dt6_led: false,
            has_extended: false,
            dt8_color_mode: mode,
            dt8_xy_capable: false,
            dt8_tc_capable: false,
            dt8_rgb_capable: false,
            c102: Default::default(),
            groups_membership: None,
            scene_levels: None,
            dt8_rgbwaf_capable: false,
            dt8_color: crate::runtime::executor::Dt8ColorFields {
                active_mode: mode,
                ..Default::default()
            },
            dt6: None,
            extended_fade_time_ms: None,
            extended_version_number: None,
            extended_versions: [None; dali2rust_contracts::msg::MAX_EXTENDED_VERSIONS],
            has_scene_colours: false,
            scene_colours: None,
        }
    }

    fn colour(
        execution: &AttributeReadExecution,
    ) -> Option<dali2rust_contracts::msg::ColorValue> {
        super::super::attribute_read::measured_colour(execution)
    }

    #[test]
    fn a_measured_tc_is_published_in_kelvin() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.dt8_color.value_2 = Some(250);
        let colour = colour(&execution).expect("a measured Tc is an observation");
        assert_eq!(colour.mode, ColorMode::Cct);
        assert_eq!(colour.color_temperature_kelvin, 4000);
    }

    #[test]
    fn an_unanswered_tc_publishes_nothing() {
        let execution = dt8_read(ColorMode::Cct);
        assert!(colour(&execution).is_none());
    }

    #[test]
    fn a_zero_mirek_is_not_a_temperature() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.dt8_color.value_2 = Some(0);
        assert!(colour(&execution).is_none());
    }

    #[test]
    fn xy_needs_both_halves() {
        let mut execution = dt8_read(ColorMode::Xy);
        execution.dt8_color.value_0 = Some(6553);
        assert!(colour(&execution).is_none(), "x alone is half a coordinate");
        execution.dt8_color.value_1 = Some(13107);
        let colour = colour(&execution).expect("both halves measured");
        assert_eq!((colour.mode, colour.x, colour.y), (ColorMode::Xy, 6553, 13107));
    }

    #[test]
    fn measured_rgb_dim_levels_are_published() {
        for mode in [ColorMode::Rgb, ColorMode::Rgbwaf] {
            let mut execution = dt8_read(mode);
            execution.dt8_color.rgb = Some((254, 10, 20));
            let colour = colour(&execution).expect("a measured triple is an observation");
            assert_eq!(
                (colour.mode, colour.r, colour.g, colour.b),
                (ColorMode::Rgb, 255, 55, 79),
                "three channels is the whole colour of a three-channel gear"
            );
        }
    }

    #[test]
    fn a_six_channel_colour_is_published_whole_or_not_at_all() {
        let mut execution = dt8_read(ColorMode::Rgbwaf);
        execution.dt8_rgbwaf_capable = true;
        execution.dt8_color.rgb = Some((254, 10, 20));
        assert!(
            colour(&execution).is_none(),
            "three measured channels beside three unread ones is not a colour"
        );

        execution.dt8_color.waf = Some((1, 2, 3));
        let colour = colour(&execution).expect("all six measured");
        assert_eq!(
            (colour.mode, colour.r, colour.w, colour.a, colour.f),
            (ColorMode::Rgbwaf, 255, 1, 18, 26),
            "all six channels decode, not just the first three"
        );
    }

    #[test]
    fn an_unanswered_rgb_triple_publishes_nothing() {
        for mode in [ColorMode::Rgb, ColorMode::Rgbwaf] {
            assert!(colour(&dt8_read(mode)).is_none(), "{mode:?}");
        }
    }

    #[test]
    fn a_gear_without_colour_reports_brightness() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.dt8_supported = Some(false);
        execution.has_dt8_color = false;
        let colour = colour(&execution).expect("brightness is an observation");
        assert_eq!(colour.mode, ColorMode::Brightness);
    }

    #[test]
    fn a_read_that_never_probed_publishes_nothing() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.dt8_supported = None;
        execution.has_dt8_color = false;
        execution.dt8_color.value_2 = Some(250);
        assert!(colour(&execution).is_none());
    }

    #[test]
    fn a_colour_capable_gear_this_read_skipped_publishes_nothing() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.has_dt8_color = false;
        execution.dt8_color.value_2 = Some(250);
        assert!(colour(&execution).is_none());
    }

    #[test]
    fn a_gear_that_named_no_active_mode_publishes_nothing() {
        for mode in [ColorMode::None, ColorMode::Unknown] {
            let mut execution = dt8_read(mode);
            execution.dt8_color.value_2 = Some(250);
            assert!(colour(&execution).is_none(), "{mode:?}");
        }
    }
}

#[test]
fn every_contract_attribute_group_survives_the_mask_round_trip() {
    for group in DaliAttributeGroup::ALL {
        assert_eq!(
            super::attribute_read::decode_attribute_groups_mask(group.mask_bit()),
            vec![group],
            "{group:?} lost by the mask decode"
        );
    }
    let all_mask = DaliAttributeGroup::ALL
        .iter()
        .fold(0u8, |mask, group| mask | group.mask_bit());
    assert_eq!(
        super::attribute_read::decode_attribute_groups_mask(all_mask).len(),
        DaliAttributeGroup::ALL.len()
    );
}

#[test]
fn sequence_incomplete_never_pollutes_the_transport_abort_counter() {
    let counters = super::DaliWorkerCounters::default();
    super::attribute_read::count_read_abort(
        crate::runtime::executor::SemanticDaliError::OperationFailed(
            crate::runtime::executor::discovery::DEVICE_TYPE_ENUM_INCOMPLETE,
        ),
        &counters,
    );
    assert_eq!(
        counters.read_attributes_sequence_incomplete.load(Ordering::Relaxed),
        1
    );
    assert_eq!(counters.read_attributes_transport_aborts.load(Ordering::Relaxed), 0);
}

mod publish_accounting {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use dali2rust_bus::{BusConfig, BusHost, BusId};
    use dali2rust_contracts::msg::{DaliDiscoveryCompletedEvent, Origin};
    use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

    use super::super::publish::publish_event_typed;
    use super::super::DaliWorkerCounters;

    const OVERFLOW_DEADLINE: Duration = Duration::from_secs(5);

    #[test]
    fn a_best_effort_event_lost_to_a_full_ingress_is_no_required_delivery_failure() {
        let (host, publisher, ()) = BusHost::spawn(
            BusConfig {
                events_ingress: 1,
                ..BusConfig::default()
            },
            |_| {},
        );
        let counters = DaliWorkerCounters::default();
        let deadline = Instant::now() + OVERFLOW_DEADLINE;
        while host.counters_snapshot().events.ingress_overflow == 0 {
            assert!(
                Instant::now() < deadline,
                "inconclusive: the events ingress never overflowed, so no best-effort \
                 event was lost and nothing here was tested"
            );
            let completed = dali2rust_contracts::bus::event_envelope(
                SOURCE_ID_UNSPECIFIED,
                1,
                BusId::default().0,
                Some(Origin::Internal),
                DaliDiscoveryCompletedEvent { registry_adapter_id: 0 },
            );
            publish_event_typed(&publisher, completed);
        }
        assert_eq!(
            counters.event_publish_failed.load(Ordering::Relaxed),
            0,
            "a best-effort drop is the bus's ingress_overflow; event_publish_failed \
             counts only a required publish that exhausted its budget (ADR-021)"
        );
    }
}

mod memory_bank_loss {
    use std::sync::{Arc, Mutex};

    use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
    use dali2rust_bus::{BusConfig, BusHost, BusId};
    use dali2rust_contracts::msg::{
        AttributeGroupReadOutcome, DaliReadAttributesCommand, ErrorCode, MemoryBankReadPreset,
    };
    use dali2rust_test_support::hold_the_events_ingress_full;

    use std::sync::atomic::Ordering;

    use super::super::attribute_read::read_attributes_memory_bank_stage;
    use super::super::memory_bank::{publish_memory_bank_execution, BankRead};
    use super::super::DaliWorkerCounters;
    use crate::runtime::clock::StdClock;
    use crate::runtime::controller::DaliController;
    use crate::runtime::executor::{AttributeReadOutcomes, MemoryReadExecution};

    const DEMO_SHORT: u8 = 0;
    const SWEEP_CORRELATION: u64 = 4_100;
    const EVENTS_INGRESS: usize = 4;
    const CHUNK_LOST: &str = "memory_bank_chunk_lost";
    const THREE_CHUNKS_OF_BYTES: usize = 60;
    const READ_CORRELATION: u64 = 4_101;
    const SWEEP: DaliReadAttributesCommand = DaliReadAttributesCommand {
        registry_adapter_id: 0,
        short_address: DEMO_SHORT,
        attribute_groups_mask: 0,
        memory_banks: MemoryBankReadPreset::Identity,
    };

    struct Swept {
        result: Result<(), (ErrorCode, &'static str)>,
        memory_banks: AttributeGroupReadOutcome,
        lost_facts: (u32, u32),
    }

    fn sweep_bank_zero(hold_the_ingress: bool) -> Swept {
        let config = BusConfig {
            events_ingress: EVENTS_INGRESS,
            ..BusConfig::default()
        };
        let (_host, publisher, ()) = BusHost::spawn(config, |_| {});
        let transport = Arc::new(Mutex::new(SimDaliTransport::demo_bus()));
        let mut controller = DaliController::new(transport, Box::new(StdClock::new()));
        let counters = DaliWorkerCounters::default();
        let mut outcomes = AttributeReadOutcomes::for_request(&[], true);
        let _ingress = hold_the_ingress.then(|| hold_the_events_ingress_full(&publisher));
        let result = read_attributes_memory_bank_stage(
            &mut controller,
            &publisher,
            BusId::default(),
            SWEEP_CORRELATION,
            &SWEEP,
            &mut outcomes,
            &counters,
        );
        let lost_facts = (
            counters.evidence_publish_failed.load(Ordering::Relaxed),
            counters.execution_failed.load(Ordering::Relaxed),
        );
        Swept { result, memory_banks: outcomes.memory_banks, lost_facts }
    }

    #[test]
    fn a_refused_chunk_of_a_bank_sweep_fails_the_attribute_read() {
        let open = sweep_bank_zero(false);
        assert_eq!(
            (open.result, open.memory_banks),
            (Ok(()), AttributeGroupReadOutcome::Success),
            "control: with the events ingress open the sweep of bank 0 succeeds"
        );
        let held = sweep_bank_zero(true);
        assert_eq!(
            held.result,
            Err((ErrorCode::CommandsIngressOverload, CHUNK_LOST)),
            "the full ingress refused the swept bank's chunk, yet the attribute read went on \
             as if the registry had heard it"
        );
        assert_eq!(
            (held.memory_banks, held.lost_facts),
            (AttributeGroupReadOutcome::TransportAbort, (1, 1)),
            "the swept bank ran on the wire and lost its chunk: the section is aborted, not \
             unattempted, and the loss counts as one lost fact (evidence, execution)"
        );
    }

    #[test]
    fn a_refused_chunk_ends_the_read_and_aborts_it() {
        let config = BusConfig {
            events_ingress: EVENTS_INGRESS,
            ..BusConfig::default()
        };
        let (_host, publisher, ()) = BusHost::spawn(config, |_| {});
        let counters = DaliWorkerCounters::default();
        let execution = MemoryReadExecution {
            bytes: vec![0; THREE_CHUNKS_OF_BYTES],
            indicator_unread: false,
            error: None,
        };
        let _ingress = hold_the_events_ingress_full(&publisher);
        let read = BankRead {
            publisher: &publisher,
            adapter_id: BusId::default(),
            correlation_id: READ_CORRELATION,
            registry_adapter_id: 0,
            short_address: DEMO_SHORT,
            counters: &counters,
        };
        let delivered = publish_memory_bank_execution(&read, 0, 0, &execution);
        assert!(!delivered, "the full ingress refused the first chunk, yet the read was delivered");
        assert_eq!(
            counters.memory_bank_chunk_events_published.load(Ordering::Relaxed),
            0,
            "no chunk reached the bus"
        );
        assert_eq!(
            counters.event_publish_failed.load(Ordering::Relaxed),
            2,
            "a refused first chunk is followed by exactly one more publish, the aborted event: \
             no later chunk of the read is tried and the read is not left open"
        );
    }
}

mod memory_bank_contention {
    use std::convert::Infallible;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
    use dali2rust_bus::{BusConfig, BusHost, BusId};
    use dali2rust_contracts::msg::{
        AttributeGroupReadOutcome, DaliReadAttributesCommand, MemoryBankReadPreset,
    };
    use dali2rust_domain::dali::banks::part251::LuminaireFormat;
    use dali2rust_domain::dali::banks::{BANK_LUMINAIRE_INFO, CONTENT_FORMAT_ID_OFFSET};
    use dali2rust_domain::dali::frame::ForwardFrame;
    use dali2rust_domain::dali::pres::command::DaliCommand;
    use dali2rust_domain::dali::pres::opcode::READ_MEMORY_LOCATION_OPCODE;
    use dali2rust_domain::dali::pres::special::SpecialCommand;
    use dali2rust_domain::dali::types::DaliAddress;
    use dali2rust_gear_model::GearSpec;
    use dali2rust_platform::dali::{DaliTransport, TransferOutcome};
    use dali2rust_test_support::StoppedClock;

    use super::super::attribute_read::read_attributes_memory_bank_stage;
    use super::super::DaliWorkerCounters;
    use crate::runtime::controller::DaliController;
    use crate::runtime::executor::AttributeReadOutcomes;

    const SHORT: u8 = 3;
    const RANDOM_ADDRESS: u32 = 0x00_4321;
    const SWEEP_CORRELATION: u64 = 4_200;
    const FOREIGN_OFFSET: u8 = 0x10;
    const QUERY_SELECTOR: u8 = 0x01;
    const INSIDE_THE_FORMAT_ID: (u8, u8) = (BANK_LUMINAIRE_INFO, CONTENT_FORMAT_ID_OFFSET + 1);
    const SWEEP: DaliReadAttributesCommand = DaliReadAttributesCommand {
        registry_adapter_id: 0,
        short_address: SHORT,
        attribute_groups_mask: 0,
        memory_banks: MemoryBankReadPreset::Identity,
    };

    #[derive(Clone, Copy)]
    enum Disturbance {
        BusBusy,
        ForeignPointer,
    }

    struct Disturbed {
        sim: SimDaliTransport,
        disturbance: Disturbance,
    }

    fn read_location() -> u16 {
        let address = DaliAddress::Short(SHORT).encode_address_byte() | QUERY_SELECTOR;
        ForwardFrame::new(address, READ_MEMORY_LOCATION_OPCODE).raw()
    }

    fn foreign_pointer() -> u16 {
        DaliCommand::Special(SpecialCommand::Dtr0(FOREIGN_OFFSET)).to_forward_frame().raw()
    }

    impl DaliTransport for Disturbed {
        type Error = Infallible;

        fn send_forward_frame(&mut self, frame: u16) -> Result<(), Infallible> {
            self.sim.send_forward_frame(frame)
        }

        fn receive_backward_frame(&mut self) -> Result<Option<u8>, Infallible> {
            self.sim.receive_backward_frame()
        }

        fn is_bus_idle(&self) -> Result<bool, Infallible> {
            self.sim.is_bus_idle()
        }

        fn exchange_frame(&mut self, frame: u16, expects_backward: bool) -> Result<TransferOutcome, Infallible> {
            if frame != read_location() || self.sim.fleet().memory_pointer() != INSIDE_THE_FORMAT_ID {
                return self.sim.exchange_frame(frame, expects_backward);
            }
            match self.disturbance {
                Disturbance::BusBusy => Ok(TransferOutcome::BusBusy),
                Disturbance::ForeignPointer => {
                    let answered = self.sim.exchange_frame(frame, expects_backward);
                    self.sim.fleet_mut().exchange(foreign_pointer(), false);
                    answered
                }
            }
        }
    }

    fn sweep(disturbance: Disturbance) -> (AttributeGroupReadOutcome, [u32; 2]) {
        let (_host, publisher, ()) = BusHost::spawn(BusConfig::default(), |_| {});
        let mut spec = GearSpec::dt6(Some(SHORT), RANDOM_ADDRESS);
        spec.luminaire_format = Some(LuminaireFormat::V3);
        let sim = SimDaliTransport::new(vec![spec]);
        let transport = Arc::new(Mutex::new(Disturbed { sim, disturbance }));
        let mut controller = DaliController::new(transport, Box::new(StoppedClock::at(0)));
        let counters = DaliWorkerCounters::default();
        let mut outcomes = AttributeReadOutcomes::for_request(&[], true);
        let result = read_attributes_memory_bank_stage(
            &mut controller,
            &publisher,
            BusId::default(),
            SWEEP_CORRELATION,
            &SWEEP,
            &mut outcomes,
            &counters,
        );
        assert!(result.is_err(), "inconclusive: the disturbance never reached the sweep");
        let aborts = [&counters.read_attributes_contended_aborts, &counters.read_attributes_transport_aborts];
        (outcomes.memory_banks, aborts.map(|counter| counter.load(Ordering::Relaxed)))
    }

    const CONTENDED: (AttributeGroupReadOutcome, [u32; 2]) = (AttributeGroupReadOutcome::ContendedAbort, [1, 0]);

    // DiiA 252 §9.2.2
    // IEC 62386-101 §8.2.5
    #[test]
    fn a_latched_value_the_busy_bus_breaks_after_its_first_byte_is_a_contended_sweep() {
        assert_eq!(sweep(Disturbance::BusBusy), CONTENDED, "the gear answered; the wire never let the rest through");
    }

    // IEC 62386-102 §9.10.4
    #[test]
    fn a_sweep_whose_pointer_a_foreign_frame_moves_is_contended() {
        assert_eq!(sweep(Disturbance::ForeignPointer), CONTENDED, "another master moved the pointer");
    }
}

mod answer_riders {
    use std::time::Duration;

    use dali2rust_bus::{BusConfig, BusFrame, BusHost};
    use dali2rust_contracts::msg::{DeliveryStatus, ErrorCode};

    use super::super::publish::Answer;
    use super::super::DaliWorkerCounters;

    const CARRIER: u64 = 81;
    const RIDERS: [u64; 2] = [82, 83];
    const ANSWER_DEADLINE: Duration = Duration::from_secs(1);

    type Answered = (u64, DeliveryStatus, Option<(ErrorCode, String)>);

    #[test]
    fn a_failed_answer_reaches_the_carrier_and_every_request_it_carries_with_one_cause() {
        let (_host, publisher, confirmations) =
            BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_confirmations(8));
        let counters = DaliWorkerCounters::default();
        let answer = Answer { correlation_id: CARRIER, riders: &RIDERS };
        answer.failed(&publisher, &counters, (ErrorCode::Conflict, "controller_passive"));
        let mut answered: Vec<Answered> = (0..=RIDERS.len())
            .map(|_| {
                let Ok(BusFrame::Confirmation(conf)) = confirmations.recv_timeout(ANSWER_DEADLINE) else {
                    panic!("a refused request went unanswered");
                };
                let error = conf.confirmation.error.as_ref().map(|e| (e.code, e.message.as_str().to_owned()));
                (conf.meta.correlation_id, conf.status, error)
            })
            .collect();
        answered.sort_by_key(|answer| answer.0);
        let refusal = Some((ErrorCode::Conflict, "controller_passive".to_owned()));
        assert_eq!(
            answered,
            [CARRIER, RIDERS[0], RIDERS[1]]
                .map(|correlation_id| (correlation_id, DeliveryStatus::ExecutionFailed, refusal.clone()))
                .to_vec()
        );
        assert_eq!(counters.execution_failed.load(std::sync::atomic::Ordering::Relaxed), 3);
    }
}

mod lost_fact_riders {
    use dali2rust_bus::{BusConfig, BusHost, BusId};
    use dali2rust_contracts::msg::{DaliTargetScope, LightSetpoint, PowerState, RuntimeSource};
    use dali2rust_test_support::hold_the_events_ingress_full;

    use super::super::publish::Answer;
    use super::super::target_state::{close_target_state_applied, target_state_applied_event, Provenance, Reply};
    use super::super::DaliWorkerCounters;

    const EVENTS_INGRESS: usize = 4;
    const CARRIER: u64 = 91;
    const RIDERS: [u64; 2] = [92, 93];
    const SHORT: u8 = 9;

    #[test]
    fn a_lost_applied_fact_fails_the_carrier_and_every_request_it_carries() {
        let config = BusConfig { events_ingress: EVENTS_INGRESS, ..BusConfig::default() };
        let (_host, publisher, ()) = BusHost::spawn(config, |_| {});
        let counters = DaliWorkerCounters::default();
        let setpoint = LightSetpoint { power: PowerState::On, level: Some(1), color: None };
        let provenance = Provenance { source: RuntimeSource::Api, hold_hcl: true };
        let applied = target_state_applied_event(0, DaliTargetScope::Short, None, Some(SHORT), None, &setpoint, provenance);
        let _ingress = hold_the_events_ingress_full(&publisher);
        let answer = Answer { correlation_id: CARRIER, riders: &RIDERS };
        let reply = Reply { publisher: &publisher, adapter_id: BusId::default(), answer, provenance, counters: &counters };
        assert!(
            !close_target_state_applied(&reply, applied),
            "inconclusive: the held events ingress took the applied fact"
        );
        assert_eq!(
            counters.execution_failed.load(std::sync::atomic::Ordering::Relaxed),
            1 + 2,
            "the carrier and both requests it carries are each answered as failed"
        );
    }
}
