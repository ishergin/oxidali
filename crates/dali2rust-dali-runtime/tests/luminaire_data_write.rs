use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_contracts::msg::{ErrorCode, LuminaireNumbers as N};
use dali2rust_dali_runtime::runtime::controller::DaliController;
use dali2rust_dali_runtime::runtime::executor::{
    write_luminaire_data, LuminaireWriteFailure, LOCK_SUBJECT,
};
use dali2rust_domain::dali::banks::luminaire_write::{
    number_patch, planned_writes, text_patch, FieldWrite, LuminaireField, BANK_UNLOCKED,
};
use dali2rust_domain::dali::banks::part251::{LuminaireFormat, LUMINAIRE_COLOUR, YEAR_OFFSET};
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_gear_model::GearSpec;
use dali2rust_platform::dali::{
    DaliTransport, TransferOutcome, WireActivity, WireLease, WirePriority, YieldGranularity,
};
use dali2rust_test_support::StoppedClock;

const SHORT: u8 = 3;
const RANDOM_ADDRESS: u32 = 0x00_4321;
const PROFILE_BANK: u8 = 1;
const LOCK_OFFSET: u8 = 0x02;
const LAST_OFFSET_OFFSET: u8 = 0x00;
const FOUND_LOCK: u8 = 0x06;
const YEAR: u8 = 24;
const UNIT_BYTES: usize = 3;
const UNITS_PER_SESSION: usize = 2;
const MULTI_SESSION_TEXT: &str = "fixture lamp";
const LOCK_FRAMES_PER_SESSION: usize = 2;

enum Interruption {
    Operator(Arc<WireActivity>),
    ForeignCommand(u16),
}

struct Interrupting {
    sim: SimDaliTransport,
    interruption: Interruption,
    trigger: u16,
}

impl Interrupting {
    fn after(&mut self, frame: u16) {
        if frame != self.trigger {
            return;
        }
        match &self.interruption {
            Interruption::Operator(gate) => gate.note_arrival(WirePriority::Interactive),
            Interruption::ForeignCommand(command) => {
                self.sim.fleet_mut().exchange(*command, false);
            }
        }
    }
}

impl DaliTransport for Interrupting {
    type Error = <SimDaliTransport as DaliTransport>::Error;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        let sent = self.sim.send_forward_frame(frame);
        self.after(frame);
        sent
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        self.sim.receive_backward_frame()
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        self.sim.is_bus_idle()
    }

    fn exchange_frame(&mut self, frame: u16, expects_backward: bool) -> Result<TransferOutcome, Self::Error> {
        let outcome = self.sim.exchange_frame(frame, expects_backward);
        self.after(frame);
        outcome
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        let outcome = self.sim.exchange_frame_with_settle(frame, expects_backward, min_idle_us);
        self.after(frame);
        outcome
    }
}

fn special(command: SpecialCommand) -> u16 {
    DaliCommand::Special(command).to_forward_frame().raw()
}

fn luminaire_gear(lock: u8, unstored: bool) -> SimDaliTransport {
    let mut spec = GearSpec::dt6(Some(SHORT), RANDOM_ADDRESS);
    spec.luminaire_format = Some(LuminaireFormat::V3);
    let mut sim = SimDaliTransport::new(vec![spec]);
    let gear = &mut sim.fleet_mut().gears_mut()[0];
    gear.set_bank1_lock_byte(lock);
    gear.faults.memory_write_unstored = unstored;
    sim
}

struct Outcome {
    result: Result<(), LuminaireWriteFailure>,
    readback: Option<Vec<u8>>,
}

fn run_write<T: DaliTransport + Send + 'static>(
    controller: &mut DaliController<T>,
    writes: &[FieldWrite],
) -> Outcome {
    let mut readback = None;
    let result = write_luminaire_data(controller, SHORT, writes, |read| {
        readback = Some(read.bytes.clone());
    });
    Outcome { result, readback }
}

fn write(lock: u8, unstored: bool, writes: &[FieldWrite]) -> (Outcome, SimDaliTransport) {
    let bus = Arc::new(Mutex::new(luminaire_gear(lock, unstored)));
    let mut controller = DaliController::new(Arc::clone(&bus), Box::new(StoppedClock::at(0)));
    let outcome = run_write(&mut controller, writes);
    drop(controller);
    let sim = Arc::into_inner(bus).expect("one owner").into_inner().expect("bus");
    (outcome, sim)
}

fn year(value: u8) -> Vec<FieldWrite> {
    planned_writes(&number_patch(&[(N::PATCH_YEAR, u32::from(value))]), None)
}

fn text(name: &str, value: &str) -> Vec<FieldWrite> {
    let field = LuminaireField::named(name).expect(name);
    planned_writes(&N::default(), text_patch(field, value).as_ref())
}

fn written_values(sim: &SimDaliTransport) -> Vec<u8> {
    let opcode = special(SpecialCommand::WriteMemoryLocation(0)) >> 8;
    sim.sent_frames()
        .into_iter()
        .filter(|frame| frame >> 8 == opcode)
        .map(|frame| frame.to_be_bytes()[1])
        .collect()
}

fn location(sim: &SimDaliTransport, offset: u8) -> Option<u8> {
    sim.fleet().gears()[0].memory_location(PROFILE_BANK, offset)
}

fn failure(code: ErrorCode, subject: &str) -> Result<(), (ErrorCode, String)> {
    Err((code, subject.to_owned()))
}

fn shape(result: Result<(), LuminaireWriteFailure>) -> Result<(), (ErrorCode, String)> {
    result.map_err(|failure| (failure.code, failure.subject.to_owned()))
}

fn sessions_on_the_wire(bytes: &[u8], lock: u8) -> Vec<u8> {
    let units: Vec<&[u8]> = bytes.chunks(UNIT_BYTES).collect();
    units
        .chunks(UNITS_PER_SESSION)
        .flat_map(|session| {
            let body = session.iter().flat_map(|unit| unit.iter().copied());
            std::iter::once(BANK_UNLOCKED).chain(body).chain(std::iter::once(lock))
        })
        .collect()
}

// IEC 62386-102 §9.10.2, §9.10.5
#[test]
fn a_locked_bank_is_opened_written_closed_and_read_back_whole() {
    let (outcome, sim) = write(FOUND_LOCK, false, &year(YEAR));

    assert_eq!(shape(outcome.result), Ok(()));
    assert_eq!(written_values(&sim), vec![BANK_UNLOCKED, YEAR, FOUND_LOCK], "open, write, close");
    assert_eq!(location(&sim, YEAR_OFFSET), Some(YEAR));
    assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK), "the gear is left as found");
    let last = location(&sim, LAST_OFFSET_OFFSET).expect("bank 1 is implemented");
    let readback = outcome.readback.expect("the bank is read back");
    assert_eq!(readback.len(), usize::from(last) + 1, "the read-back covers the whole bank");
    assert_eq!(readback[usize::from(YEAR_OFFSET)], YEAR);
}

// IEC 62386-102 §9.10.2, Table 8
#[test]
fn an_open_bank_is_written_without_touching_its_lock() {
    let (outcome, sim) = write(BANK_UNLOCKED, false, &year(YEAR));

    assert_eq!(shape(outcome.result), Ok(()));
    assert_eq!(written_values(&sim), vec![YEAR], "no lock byte is written");
    assert_eq!(location(&sim, LOCK_OFFSET), Some(BANK_UNLOCKED));
}

// IEC 62386-102 §9.10.2, Table 8
#[test]
fn a_lock_that_does_not_open_fails_on_the_lock_and_writes_no_field() {
    let (outcome, sim) = write(FOUND_LOCK, true, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::VerifyFailed, LOCK_SUBJECT));
    assert_eq!(
        written_values(&sim),
        vec![BANK_UNLOCKED, FOUND_LOCK],
        "the unlock is proved by reading 0x02 before any field byte, and the lock is put back"
    );
}

// IEC 62386-102 §9.10.5
#[test]
fn a_byte_the_gear_confirms_but_does_not_keep_fails_its_field() {
    let (outcome, sim) = write(BANK_UNLOCKED, true, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::VerifyFailed, "year"));
    assert_eq!(written_values(&sim), vec![YEAR], "the gear answered the write itself");
    assert_ne!(location(&sim, YEAR_OFFSET), Some(YEAR));
}

// DiiA 251 §9.2.3
#[test]
fn a_field_the_gears_format_lacks_is_refused_before_any_write() {
    let (outcome, sim) = write(FOUND_LOCK, false, &text("oem_name", "ACME"));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::Conflict, "bank1_field_not_in_format")
    );
    assert!(written_values(&sim).is_empty(), "the header refused it before the wire");
    assert_eq!(outcome.readback, None, "nothing was written, so nothing is read back");
}

// DiiA 251 §9.2.3
#[test]
fn a_text_spanning_several_sessions_opens_and_closes_the_lock_in_each() {
    let (outcome, sim) = write(FOUND_LOCK, false, &text("luminaire_colour", MULTI_SESSION_TEXT));

    let mut stored = MULTI_SESSION_TEXT.as_bytes().to_vec();
    stored.push(0);
    assert_eq!(shape(outcome.result), Ok(()));
    assert_eq!(written_values(&sim), sessions_on_the_wire(&stored, FOUND_LOCK));
    let held: Vec<Option<u8>> = (0..stored.len())
        .map(|index| location(&sim, LUMINAIRE_COLOUR.offset + index as u8))
        .collect();
    assert_eq!(held, stored.iter().copied().map(Some).collect::<Vec<_>>());
    assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK));
}

fn write_interrupted(trigger: u16, interruption: Interruption, writes: &[FieldWrite]) -> (Outcome, SimDaliTransport) {
    let gate = match &interruption {
        Interruption::Operator(gate) => Arc::clone(gate),
        Interruption::ForeignCommand(_) => Arc::new(WireActivity::new()),
    };
    let sim = luminaire_gear(FOUND_LOCK, false);
    let bus = Arc::new(Mutex::new(Interrupting { sim, interruption, trigger }));
    let mut controller = DaliController::new(Arc::clone(&bus), Box::new(StoppedClock::at(0)));
    let lease = WireLease::new(gate, WirePriority::Attended, YieldGranularity::Step);
    let outcome = controller.with_wire_lease(lease, |c| run_write(c, writes));
    drop(controller);
    let interrupting = Arc::into_inner(bus).expect("one owner").into_inner().expect("bus");
    (outcome, interrupting.sim)
}

// IEC 62386-102 §9.10.2; ADR-009
#[test]
fn an_operator_arriving_mid_session_waits_for_the_lock_to_close() {
    let first = MULTI_SESSION_TEXT.as_bytes()[0];
    let trigger = special(SpecialCommand::WriteMemoryLocation(first));
    let operator = Interruption::Operator(Arc::new(WireActivity::new()));
    let writes = text("luminaire_colour", MULTI_SESSION_TEXT);
    let (outcome, sim) = write_interrupted(trigger, operator, &writes);

    let sim = &sim;
    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::Preempted, "luminaire_colour"),
        "the yield names the field it stopped in, not the lock it never opened"
    );
    let session_len = UNIT_BYTES * UNITS_PER_SESSION + LOCK_FRAMES_PER_SESSION;
    let first_session = &sessions_on_the_wire(MULTI_SESSION_TEXT.as_bytes(), FOUND_LOCK)[..session_len];
    assert_eq!(written_values(sim), first_session, "the started session runs to its relock");
    assert_eq!(location(sim, LOCK_OFFSET), Some(FOUND_LOCK));
    assert_eq!(sim.sent_frames().last(), Some(&special(SpecialCommand::WriteMemoryLocation(FOUND_LOCK))));
    assert_eq!(outcome.readback, None, "a yielded write leaves the read-back to the next run");
}

// IEC 62386-102 §9.10.5
#[test]
fn a_foreign_command_between_units_costs_nothing_because_each_unit_enables_writing() {
    let end_of_first_unit = MULTI_SESSION_TEXT.as_bytes()[UNIT_BYTES - 1];
    let trigger = special(SpecialCommand::WriteMemoryLocation(end_of_first_unit));
    let foreign = DaliCommand::Standard { address: DaliAddress::Broadcast, command: StandardCommand::QueryStatus };
    let interruption = Interruption::ForeignCommand(foreign.to_forward_frame().raw());
    let (outcome, sim) = write_interrupted(trigger, interruption, &text("luminaire_colour", MULTI_SESSION_TEXT));

    assert_eq!(shape(outcome.result), Ok(()), "a foreign frame between units clears writeEnableState");
    assert_eq!(location(&sim, LUMINAIRE_COLOUR.offset + UNIT_BYTES as u8), Some(MULTI_SESSION_TEXT.as_bytes()[UNIT_BYTES]));
}
