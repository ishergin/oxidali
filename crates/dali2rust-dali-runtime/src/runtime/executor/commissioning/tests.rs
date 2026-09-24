use super::*;
use crate::runtime::executor::discovery::DiscoveredDevice;
use dali2rust_contracts::msg::{ColorMode, DeviceType};
use dali2rust_domain::dali::commands::{DaliCommand, DaliResponse};
use dali2rust_domain::dali::controller::{DaliApplicationController, DaliProductController};
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::pres::special::QueryShortAddressAnswer;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::ses::DaliSession;
use dali2rust_domain::dali::types::DaliAddress;
use std::collections::VecDeque;

struct FakeController {
    session: DaliSession,
    commands: Vec<DaliCommand>,
    search_address: u32,
    random_address: Option<u32>,
    compare_script: VecDeque<Option<u8>>,
    violate_compare: bool,
    verify_reply: Option<u8>,
    violate_special_queries: bool,
    query_short_reply: Option<u8>,
    programmed_short: Option<u8>,
    detected_device: Option<DiscoveredDevice>,
    occupied_on_wire: Vec<u8>,
    unaddressed_reply: Option<DaliResponse>,
}

impl FakeController {
    fn with_random_address(random_address: Option<u32>) -> Self {
        Self {
            session: DaliSession::new(),
            commands: Vec::new(),
            search_address: 0,
            random_address,
            compare_script: VecDeque::new(),
            occupied_on_wire: Vec::new(),
            unaddressed_reply: None,
            violate_compare: false,
            verify_reply: Some(1),
            violate_special_queries: false,
            query_short_reply: None,
            programmed_short: None,
            detected_device: Some(DiscoveredDevice {
                short_address: 0,
                random_address: None,
                device_type: DeviceType::Dt6Led,
                color_mode: ColorMode::Brightness,
                dt8_xy_capable: false,
                dt8_tc_capable: false,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: None,
            }),
        }
    }

    fn with_compare_script(script: impl IntoIterator<Item = Option<u8>>) -> Self {
        let mut controller = Self::with_random_address(None);
        controller.compare_script = script.into_iter().collect();
        controller
    }

    fn special_command_log(&self) -> Vec<SpecialCommand> {
        self.commands
            .iter()
            .filter_map(|cmd| match cmd {
                DaliCommand::Special(special) => Some(*special),
                _ => None,
            })
            .collect()
    }

    fn compare_reply(&mut self) -> Option<u8> {
        self.compare_script.pop_front().unwrap_or_else(|| {
            self.random_address
                .filter(|addr| *addr <= self.search_address)
                .map(|_| 1)
        })
    }

    fn update_search_address(&mut self, command: SpecialCommand) {
        match command {
            SpecialCommand::SearchAddrH(value) => {
                self.search_address = (self.search_address & 0x0000_FFFF) | (u32::from(value) << 16);
            }
            SpecialCommand::SearchAddrM(value) => {
                self.search_address = (self.search_address & 0x00FF_00FF) | (u32::from(value) << 8);
            }
            SpecialCommand::SearchAddrL(value) => {
                self.search_address = (self.search_address & 0x00FF_FF00) | u32::from(value);
            }
            _ => {}
        }
    }
}

impl DaliProductController for FakeController {
    type Error = ();

    fn send_command(&mut self, cmd: &DaliCommand) -> Result<DaliResponse, Self::Error> {
        self.commands.push(*cmd);
        match *cmd {
            DaliCommand::Special(command) => {
                self.update_search_address(command);
                if self.violate_special_queries
                    && matches!(
                        command,
                        SpecialCommand::Compare
                            | SpecialCommand::VerifyShortAddress(_)
                            | SpecialCommand::QueryShortAddress
                    )
                {
                    return Ok(DaliResponse::Violation);
                }
                Ok(match command {
                    SpecialCommand::QueryShortAddress => self
                        .query_short_reply
                        .map(DaliResponse::Answer)
                        .unwrap_or(DaliResponse::NoAnswer),
                    SpecialCommand::Compare => match self.compare_reply() {
                        Some(_) if self.violate_compare => DaliResponse::Violation,
                        Some(value) => DaliResponse::Answer(value),
                        None => DaliResponse::NoAnswer,
                    },
                    SpecialCommand::VerifyShortAddress(_) => self
                        .verify_reply
                        .map(DaliResponse::Answer)
                        .unwrap_or(DaliResponse::NoAnswer),
                    SpecialCommand::ProgramShortAddress(value) => {
                        self.programmed_short = Some((value >> 1) & 0x3F);
                        DaliResponse::NoAnswer
                    }
                    _ => DaliResponse::NoAnswer,
                })
            }
            DaliCommand::Standard { address, command } => {
                if command == StandardCommand::QueryControlGearPresent {
                    let taken = matches!(address, DaliAddress::Short(s)
                        if self.occupied_on_wire.contains(&s));
                    return Ok(if taken {
                        DaliResponse::Answer(0xFF)
                    } else {
                        DaliResponse::NoAnswer
                    });
                }
                if command == StandardCommand::QueryMissingShortAddress {
                    return Ok(self.unaddressed_reply.unwrap_or(DaliResponse::NoAnswer));
                }
                let expected_short = self.programmed_short.unwrap_or_default();
                let Some(device) = self.detected_device else {
                    return Ok(DaliResponse::NoAnswer);
                };
                if address != DaliAddress::Short(expected_short) {
                    return Ok(DaliResponse::NoAnswer);
                }
                Ok(match command {
                    StandardCommand::QueryDeviceType => DaliResponse::Answer(match device.device_type {
                        DeviceType::Dt8Color => 8,
                        DeviceType::Dt6Led => 6,
                        _ => 0,
                    }),
                    StandardCommand::QueryNextDeviceType => DaliResponse::NoAnswer,
                    _ => DaliResponse::NoAnswer,
                })
            }
            DaliCommand::Extended { .. } => Ok(DaliResponse::NoAnswer),
            _ => Ok(DaliResponse::NoAnswer),
        }
    }

    fn session(&self) -> &DaliSession {
        &self.session
    }
}

impl DaliApplicationController for FakeController {
    fn send_raw(
        &mut self,
        _frame: ForwardFrame,
        _expects_backward: bool,
    ) -> Result<DaliResponse, Self::Error> {
        Ok(DaliResponse::NoAnswer)
    }
}

#[test]
fn a_violating_compare_answer_keeps_the_search_splitting() {
    let mut controller = FakeController::with_random_address(Some(0x00AB_CDEF));
    controller.violate_compare = true;

    let found = find_lowest_random_address(&mut controller).expect("search");

    assert_eq!(
        found,
        Some(0x00AB_CDEF),
        "a violating COMPARE is YES, so the search narrows to the gear"
    );
}

#[test]
fn begin_unaddressed_commissioning_sends_initialise_then_randomise() {
    let mut controller = FakeController::with_random_address(None);
    begin_unaddressed_commissioning(&mut controller).expect("begin");
    assert_eq!(
        controller.special_command_log(),
        vec![SpecialCommand::Initialise(0xFF), SpecialCommand::Randomise]
    );
}

#[test]
fn begin_unaddressed_commissioning_waits_out_the_randomise_settling() {
    let mut controller = FakeController::with_random_address(None);

    let started = std::time::Instant::now();
    begin_unaddressed_commissioning(&mut controller).expect("begin");
    let elapsed = started.elapsed();

    assert!(
        elapsed >= std::time::Duration::from_millis(RANDOMISE_SETTLE_MS),
        "returned after {elapsed:?}, while the gear is still allowed to be unready"
    );
}

#[test]
fn a_search_that_finds_nothing_while_a_gear_is_unaddressed_fails_the_run() {
    let mut controller = FakeController::with_compare_script([None]);
    controller.unaddressed_reply = Some(DaliResponse::Answer(0xFF));

    let error = commission_next_unaddressed(&mut controller, &[]).expect_err("disagreement");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed("commissioning_unaddressed_not_reached")
    );
}

#[test]
fn a_search_that_finds_nothing_with_nothing_unaddressed_ends_the_run() {
    let mut controller = FakeController::with_compare_script([None]);

    let found = commission_next_unaddressed(&mut controller, &[]).expect("end of run");

    assert_eq!(found, None);
    assert!(
        controller.commands.iter().any(|c| matches!(
            c,
            DaliCommand::Standard { command: StandardCommand::QueryMissingShortAddress, .. }
        )),
        "the premise was assumed rather than checked: {:?}",
        controller.commands
    );
}

#[test]
fn terminate_unaddressed_commissioning_sends_terminate() {
    let mut controller = FakeController::with_random_address(None);
    terminate_unaddressed_commissioning(&mut controller).expect("terminate");
    assert_eq!(controller.special_command_log(), vec![SpecialCommand::Terminate]);
}

#[test]
fn program_short_address_encoding_masks_and_shifts() {
    assert_eq!(encode_program_short_address(0), 0x01);
    assert_eq!(encode_program_short_address(1), 0x03);
    assert_eq!(encode_program_short_address(63), 0x7F);
}

#[test]
fn next_free_short_address_skips_used() {
    let mut c = FakeController::with_random_address(None);
    assert_eq!(next_free_short_address(&mut c, &[]), Ok(Some(0)));
    assert_eq!(next_free_short_address(&mut c, &[0]), Ok(Some(1)));
    assert_eq!(
        next_free_short_address(&mut c, &(0..=63).collect::<Vec<_>>()),
        Ok(None)
    );
}

#[test]
fn an_address_outside_the_six_bit_space_is_ignored_not_a_panic() {
    let mut c = FakeController::with_random_address(None);
    assert_eq!(next_free_short_address(&mut c, &[64, 255]), Ok(Some(0)));
    assert_eq!(next_free_short_address(&mut c, &[0, 64, 1, 255]), Ok(Some(2)));
}

#[test]
fn an_address_the_registry_forgot_is_still_taken_if_the_wire_says_so() {
    let mut c = FakeController::with_random_address(None);
    c.occupied_on_wire = vec![0, 1, 2];
    assert_eq!(next_free_short_address(&mut c, &[]), Ok(Some(3)));
}

#[test]
fn an_address_the_registry_knows_is_kept_even_when_the_wire_is_silent() {
    let mut c = FakeController::with_random_address(None);
    assert_eq!(next_free_short_address(&mut c, &[0, 1, 2]), Ok(Some(3)));
}

#[test]
fn commission_next_unaddressed_returns_none_when_compare_never_answers() {
    let mut controller = FakeController::with_random_address(None);
    assert_eq!(commission_next_unaddressed(&mut controller, &[]), Ok(None));
}

#[test]
fn commission_next_unaddressed_returns_no_free_short_address_when_all_occupied() {
    let mut controller = FakeController::with_random_address(Some(0));
    assert_eq!(
        commission_next_unaddressed(&mut controller, &(0..=63).collect::<Vec<_>>()),
        Err(SemanticDaliError::Conflict("no_free_short_address"))
    );
}

#[test]
fn commission_next_unaddressed_returns_verify_short_address_failed_on_missing_verify_reply() {
    let mut controller = FakeController::with_random_address(Some(0));
    controller.verify_reply = None;
    assert_eq!(
        commission_next_unaddressed(&mut controller, &[]),
        Err(SemanticDaliError::OperationFailed(
            "verify_short_address_failed",
        ))
    );
}

#[test]
fn commission_next_unaddressed_returns_programmed_device_not_present_when_detect_device_none() {
    let mut controller = FakeController::with_random_address(Some(0));
    controller.detected_device = None;
    assert_eq!(
        commission_next_unaddressed(&mut controller, &[]),
        Err(SemanticDaliError::OperationFailed(
            "programmed_device_not_present",
        ))
    );
}

#[test]
fn commission_next_unaddressed_retry_budget_returns_bus_contended_after_repeated_verify_failures() {
    let mut controller = FakeController::with_random_address(Some(0));
    controller.verify_reply = None;

    assert_eq!(
        commission_next_unaddressed_with_retry_budget(&mut controller, &[], 2),
        Err(SemanticDaliError::OperationFailed("bus_contended"))
    );

    let programmed_attempts = controller
        .special_command_log()
        .into_iter()
        .filter(|command| matches!(command, SpecialCommand::ProgramShortAddress(_)))
        .count();
    assert_eq!(programmed_attempts, 3);
}

#[test]
fn find_lowest_random_address_binary_searches_to_expected_value() {
    let mut controller = FakeController::with_random_address(Some(0x0012_3456));
    assert_eq!(
        find_lowest_random_address(&mut controller),
        Ok(Some(0x0012_3456))
    );
}

#[test]
fn find_lowest_random_address_returns_commissioning_search_inconsistent_when_final_compare_fails() {
    let mut controller = FakeController::with_compare_script(
        (0..25)
            .map(|_| Some(1))
            .chain(std::iter::once(None)),
    );
    assert_eq!(
        find_lowest_random_address(&mut controller),
        Err(SemanticDaliError::OperationFailed(
            "commissioning_search_inconsistent",
        ))
    );
}

#[test]
fn every_reply_bearing_expert_step_reads_a_violation_as_an_answer() {
    for step in [
        CommissioningStep::Compare,
        CommissioningStep::VerifyShortAddress,
        CommissioningStep::QueryShortAddress,
    ] {
        let mut controller = FakeController::with_random_address(None);
        controller.violate_special_queries = true;
        let outcome = run_commissioning_step(&mut controller, step, None, Some(3), None)
            .expect("a violation is an answer, never a transport failure");
        assert!(
            outcome.backward_violation,
            "{step:?}: §8.2.5 makes a violating frame a backward frame"
        );
        assert_eq!(
            outcome.backward_frame, None,
            "{step:?}: a violating frame carries no content"
        );
    }
}

#[test]
fn query_short_address_reports_mask_as_a_byte_not_as_address_sixty_three() {
    let mut controller = FakeController::with_random_address(None);
    controller.query_short_reply = Some(0xFF);
    let outcome = run_commissioning_step(
        &mut controller,
        CommissioningStep::QueryShortAddress,
        None,
        None,
        None,
    )
    .expect("MASK is an answer");
    assert_eq!(outcome.backward_frame, Some(0xFF));
    assert!(!outcome.backward_violation);
    assert_eq!(
        QueryShortAddressAnswer::decode(outcome.backward_frame, outcome.backward_violation),
        QueryShortAddressAnswer::Unaddressed,
        "a gear with no short address is not the gear at address 63"
    );
}

#[test]
fn an_unaddressed_gear_and_several_gear_are_different_answers() {
    let mut unaddressed = FakeController::with_random_address(None);
    unaddressed.query_short_reply = Some(0xFF);
    let a = run_commissioning_step(
        &mut unaddressed,
        CommissioningStep::QueryShortAddress,
        None,
        None,
        None,
    )
    .expect("MASK is an answer");

    let mut several = FakeController::with_random_address(None);
    several.violate_special_queries = true;
    let b = run_commissioning_step(
        &mut several,
        CommissioningStep::QueryShortAddress,
        None,
        None,
        None,
    )
    .expect("a violation is an answer");

    assert_ne!(a, b, "MASK and a violating frame must not collapse");
    assert_eq!(
        QueryShortAddressAnswer::decode(a.backward_frame, a.backward_violation).wire_name(),
        "unaddressed"
    );
    assert_eq!(
        QueryShortAddressAnswer::decode(b.backward_frame, b.backward_violation).wire_name(),
        "multiple"
    );
}

#[test]
fn a_detect_miss_after_withdraw_never_programs_a_second_gear() {
    let mut controller = FakeController::with_random_address(Some(0));
    controller.detected_device = None;

    assert_eq!(
        commission_next_unaddressed_with_retry_budget(&mut controller, &[], 2),
        Err(SemanticDaliError::OperationFailed(
            "programmed_device_not_present"
        )),
        "the failure is reported as itself, not laundered into bus_contended"
    );

    let log = controller.special_command_log();
    let programmed = log
        .iter()
        .filter(|command| matches!(command, SpecialCommand::ProgramShortAddress(_)))
        .count();
    assert_eq!(
        programmed, 1,
        "one gear was withdrawn, so exactly one address may be handed out"
    );
    let withdrawn = log
        .iter()
        .filter(|command| matches!(command, SpecialCommand::Withdraw))
        .count();
    assert_eq!(withdrawn, 1, "a second WITHDRAW means a second gear was taken");
}

#[test]
fn a_verify_miss_before_withdraw_still_repeats_the_whole_attempt() {
    let mut controller = FakeController::with_random_address(Some(0));
    controller.verify_reply = None;
    let _ = commission_next_unaddressed_with_retry_budget(&mut controller, &[], 2);
    let withdrawn = controller
        .special_command_log()
        .into_iter()
        .filter(|command| matches!(command, SpecialCommand::Withdraw))
        .count();
    assert_eq!(
        withdrawn, 0,
        "verify failed, so WITHDRAW never went out and nothing left the search"
    );
}

mod address_change {
    use crate::runtime::executor::commissioning::change_short_address;
    use crate::runtime::executor::test_helpers::shared::{
        assert_script_consumed, setup_controller, short_address,
    };
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_domain::dali::commands::DaliCommand;
    use dali2rust_domain::dali::pres::special::SpecialCommand;
    use dali2rust_domain::dali::pres::standard::StandardCommand;

    const FROM: u8 = 5;
    const TO: u8 = 9;
    const ENCODED: u8 = (TO << 1) | 1;

    fn special_frame(cmd: SpecialCommand) -> u16 {
        DaliCommand::Special(cmd).to_forward_frame().raw()
    }

    fn standard_frame(short: u8, command: StandardCommand) -> u16 {
        DaliCommand::Standard {
            address: short_address(short),
            command,
        }
        .to_forward_frame()
        .raw()
    }

    fn expect_arm(mock: &MockDaliTransport, armed: Option<u8>) {
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(ENCODED)));
        mock.expect_forward_frame_with_backward(
            standard_frame(FROM, StandardCommand::QueryContentDtr0),
            armed,
        );
    }

    fn expect_write(mock: &MockDaliTransport) {
        let frame = standard_frame(FROM, StandardCommand::SetShortAddress);
        mock.expect_forward_frame(frame);
        mock.expect_forward_frame(frame);
    }

    #[test]
    fn a_proved_operand_is_written_after_exactly_one_readback() {
        let mock = MockDaliTransport::new();
        expect_arm(&mock, Some(ENCODED));
        expect_write(&mock);
        let (transport, mut controller) = setup_controller(mock);
        change_short_address(&mut controller, FROM, TO, false).expect("the move must succeed");
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_lost_arm_is_repaired_rather_than_acted_on() {
        let mock = MockDaliTransport::new();
        expect_arm(&mock, Some(0x2A));
        expect_arm(&mock, Some(ENCODED));
        expect_write(&mock);
        let (transport, mut controller) = setup_controller(mock);
        change_short_address(&mut controller, FROM, TO, false).expect("the retry must repair it");
        assert_script_consumed(&transport);
    }

    #[test]
    fn an_unprovable_operand_never_reaches_set_short_address() {
        let mock = MockDaliTransport::new();
        for _ in 0..3 {
            expect_arm(&mock, None);
        }
        let (transport, mut controller) = setup_controller(mock);
        let outcome = change_short_address(&mut controller, FROM, TO, false);
        assert!(outcome.is_err(), "an unproved operand must not be acted on");
        let write = standard_frame(FROM, StandardCommand::SetShortAddress);
        let sent = transport.lock().expect("mock lock").sent_frames();
        assert!(
            !sent.contains(&write),
            "the gear must stay where it is when the operand cannot be proved: {sent:?}"
        );
        assert_script_consumed(&transport);
    }
}
