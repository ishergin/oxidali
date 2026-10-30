use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_contracts::msg::CommissioningStep;
use dali2rust_dali_runtime::runtime::controller::DaliController;
use dali2rust_dali_runtime::runtime::executor::{
    begin_unaddressed_commissioning, commission_next_unaddressed_with_retry_budget, run_commissioning_step,
    SemanticDaliError,
};
use dali2rust_domain::dali::controller::{DaliApplicationController, WireHold};
use dali2rust_domain::dali::frame::HoldCause;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_gear_model::GearSpec;
use dali2rust_test_support::StoppedClock;

const RANDOM_ADDRESS: u32 = 0x00_4321;
const FIRST_FREE_SHORT: u8 = 0;
const NO_RETRIES: u8 = 0;
const RETRIES: u8 = 2;
const SEARCH_ADDRESS: u32 = 0x12_34_56;
const QUIESCENT: SemanticDaliError = SemanticDaliError::Conflict("controller_quiescent");

fn special(command: SpecialCommand) -> u16 {
    DaliCommand::Special(command).to_forward_frame().raw()
}

fn program_short() -> u16 {
    special(SpecialCommand::ProgramShortAddress((FIRST_FREE_SHORT << 1) | 1))
}

struct QuiescentOnceSent(Arc<Mutex<SimDaliTransport>>, u16);

impl WireHold for QuiescentOnceSent {
    fn held(&self) -> Option<HoldCause> {
        let sent = self.0.lock().expect("bus").sent_frames();
        sent.contains(&self.1).then_some(HoldCause::Quiescent)
    }
}

struct QuiescentForOneFrameOnceSent {
    bus: Arc<Mutex<SimDaliTransport>>,
    marker: u16,
    refused: AtomicBool,
}

impl WireHold for QuiescentForOneFrameOnceSent {
    fn held(&self) -> Option<HoldCause> {
        let sent = self.bus.lock().expect("bus").sent_frames().contains(&self.marker);
        (sent && !self.refused.swap(true, Ordering::SeqCst)).then_some(HoldCause::Quiescent)
    }
}

fn unaddressed_bus() -> Arc<Mutex<SimDaliTransport>> {
    Arc::new(Mutex::new(SimDaliTransport::new(vec![GearSpec::dt6(None, RANDOM_ADDRESS)])))
}

fn under<R>(
    bus: &Arc<Mutex<SimDaliTransport>>,
    hold: Arc<dyn WireHold>,
    run: impl FnOnce(&mut DaliController<SimDaliTransport>) -> R,
) -> (R, Vec<u16>) {
    let mut controller = DaliController::new(Arc::clone(bus), Box::new(StoppedClock::at(0)));
    let result = controller.with_wire_hold(hold, run);
    let sent = bus.lock().expect("bus").sent_frames();
    (result, sent)
}

fn held_once_sent<R>(
    bus: &Arc<Mutex<SimDaliTransport>>,
    marker: u16,
    run: impl FnOnce(&mut DaliController<SimDaliTransport>) -> R,
) -> (R, Vec<u16>) {
    under(bus, Arc::new(QuiescentOnceSent(Arc::clone(bus), marker)), run)
}

fn held_for_one_frame_once_sent<R>(
    bus: &Arc<Mutex<SimDaliTransport>>,
    marker: u16,
    run: impl FnOnce(&mut DaliController<SimDaliTransport>) -> R,
) -> (R, Vec<u16>) {
    let hold = QuiescentForOneFrameOnceSent { bus: Arc::clone(bus), marker, refused: AtomicBool::new(false) };
    under(bus, Arc::new(hold), run)
}

fn after(sent: &[u16], marker: u16) -> Vec<u16> {
    let at = sent.iter().position(|&frame| frame == marker).expect("the marker frame was sent");
    sent[at + 1..].to_vec()
}

// IEC 62386-102 §9.14.3
// IEC 62386-103 §9.9.3
#[test]
fn a_hold_after_program_short_address_lets_the_gear_be_verified_and_withdrawn() {
    let bus = unaddressed_bus();
    let (result, sent) = held_once_sent(&bus, program_short(), |c| {
        begin_unaddressed_commissioning(c)?;
        commission_next_unaddressed_with_retry_budget(c, &[], NO_RETRIES)
    });

    let verify = special(SpecialCommand::VerifyShortAddress((FIRST_FREE_SHORT << 1) | 1));
    assert_eq!(after(&sent, program_short()), vec![verify, special(SpecialCommand::Withdraw)]);
    assert_eq!(result.map(|found| found.map(|d| d.short_address)), Err(QUIESCENT));
}

// IEC 62386-103 §9.9.3
#[test]
fn a_held_search_is_not_retried_and_names_its_cause() {
    let bus = unaddressed_bus();
    let randomise = special(SpecialCommand::Randomise);
    let (result, sent) = held_for_one_frame_once_sent(&bus, randomise, |c| {
        begin_unaddressed_commissioning(c)?;
        commission_next_unaddressed_with_retry_budget(c, &[], RETRIES)
    });

    assert_eq!(after(&sent, randomise), vec![randomise], "only the second half of the started pair");
    assert_eq!(result.map(|found| found.is_some()), Err(QUIESCENT));
}

// IEC 62386-103 §9.9.3
#[test]
fn a_held_detection_of_the_programmed_gear_is_not_retried_and_names_its_cause() {
    let bus = unaddressed_bus();
    let withdraw = special(SpecialCommand::Withdraw);
    let (result, sent) = held_for_one_frame_once_sent(&bus, withdraw, |c| {
        begin_unaddressed_commissioning(c)?;
        commission_next_unaddressed_with_retry_budget(c, &[], RETRIES)
    });

    assert_eq!(after(&sent, withdraw), Vec::<u16>::new(), "no detection query follows the hold");
    assert_eq!(result.map(|found| found.is_some()), Err(QUIESCENT));
}

// IEC 62386-102 §11.5.4, §11.5.5, §11.5.6
#[test]
fn an_expert_search_address_step_sets_all_three_bytes_once_started() {
    let bus = unaddressed_bus();
    let high = special(SpecialCommand::SearchAddrH(0x12));
    let (result, sent) = held_once_sent(&bus, high, |c| {
        run_commissioning_step(c, CommissioningStep::SearchAddress, None, None, Some(SEARCH_ADDRESS)).map(drop)
    });

    let rest = vec![special(SpecialCommand::SearchAddrM(0x34)), special(SpecialCommand::SearchAddrL(0x56))];
    assert_eq!(after(&sent, high), rest);
    assert_eq!(result, Ok(()));
}
