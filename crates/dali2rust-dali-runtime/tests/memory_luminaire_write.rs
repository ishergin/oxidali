use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_contracts::msg::{ErrorCode, LuminaireNumbers as N};
use dali2rust_dali_runtime::runtime::controller::DaliController;
use dali2rust_dali_runtime::runtime::executor::{
    write_memory_luminaire, LuminaireWriteFailure, IDENTITY_CHANGED, IDENTITY_SUBJECT, IDENTITY_UNKNOWN,
    LOCK_SUBJECT, READBACK_SUBJECT, RELOCK_SUBJECT,
};
use dali2rust_domain::dali::banks::identity::{
    GTIN_LEN, GTIN_OFFSET, IDENTIFICATION_NUMBER_LEN, IDENTIFICATION_NUMBER_OFFSET,
};
use dali2rust_domain::dali::banks::DeviceIdentity;
use dali2rust_domain::dali::banks::luminaire_write::{
    number_patch, planned_writes, text_patch, FieldWrite, LuminaireField, BANK_UNLOCKED,
};
use dali2rust_domain::dali::banks::part251::{self, LuminaireFormat, LUMINAIRE_COLOUR, YEAR_OFFSET};
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
const REPLACEMENT_RANDOM_ADDRESS: u32 = 0x00_8765;
const IDENTITY_BANK: u8 = 0;
const PROFILE_BANK: u8 = 1;
const LOCK_OFFSET: u8 = 0x02;
const LAST_OFFSET_OFFSET: u8 = 0x00;
const FOUND_LOCK: u8 = 0x06;
const YEAR: u8 = 24;
const UNIT_BYTES: usize = 3;
const UNITS_PER_SESSION: usize = 2;
const MULTI_SESSION_TEXT: &str = "fixture lamp";
const LOCK_FRAMES_PER_SESSION: usize = 2;

#[derive(Clone)]
enum Interruption {
    Operator(Arc<WireActivity>),
    ForeignCommand(u16),
    LostAnswer,
}

#[derive(Debug, Clone, Copy)]
enum Trigger {
    Frame(u16),
    Nth(usize),
}

struct Interrupting {
    sim: SimDaliTransport,
    interruptions: Vec<Interruption>,
    trigger: Trigger,
    sent: usize,
}

impl Interrupting {
    fn new(sim: SimDaliTransport, interruptions: Vec<Interruption>, trigger: Trigger) -> Self {
        Self { sim, interruptions, trigger, sent: 0 }
    }

    fn after(&mut self, frame: u16) -> bool {
        self.sent += 1;
        let fires = match self.trigger {
            Trigger::Frame(at) => frame == at,
            Trigger::Nth(at) => self.sent == at,
        };
        if !fires {
            return false;
        }
        let mut lost = false;
        for interruption in &self.interruptions {
            match interruption {
                Interruption::Operator(gate) => gate.note_arrival(WirePriority::Interactive),
                Interruption::ForeignCommand(command) => {
                    self.sim.fleet_mut().exchange(*command, false);
                }
                Interruption::LostAnswer => lost = true,
            }
        }
        lost
    }

    fn heard<E>(&mut self, frame: u16, outcome: Result<TransferOutcome, E>) -> Result<TransferOutcome, E> {
        let lost = self.after(frame);
        outcome.map(|heard| if lost { TransferOutcome::NoAnswer } else { heard })
    }
}

impl DaliTransport for Interrupting {
    type Error = <SimDaliTransport as DaliTransport>::Error;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        let sent = self.sim.send_forward_frame(frame);
        let _ = self.after(frame);
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
        self.heard(frame, outcome)
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        let outcome = self.sim.exchange_frame_with_settle(frame, expects_backward, min_idle_us);
        self.heard(frame, outcome)
    }
}

fn special(command: SpecialCommand) -> u16 {
    DaliCommand::Special(command).to_forward_frame().raw()
}

fn query_dtr0() -> u16 {
    let address = DaliAddress::Short(SHORT);
    DaliCommand::Standard { address, command: StandardCommand::QueryContentDtr0 }.to_forward_frame().raw()
}

fn luminaire_gear(lock: u8, unstored: bool) -> SimDaliTransport {
    luminaire_gear_at(RANDOM_ADDRESS, lock, unstored)
}

fn luminaire_gear_at(random_address: u32, lock: u8, unstored: bool) -> SimDaliTransport {
    let mut spec = GearSpec::dt6(Some(SHORT), random_address);
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

fn number_at(sim: &SimDaliTransport, offset: u8, len: usize) -> u64 {
    (offset..offset + len as u8)
        .map(|at| sim.fleet().gears()[0].memory_location(IDENTITY_BANK, at).expect("bank 0 is implemented"))
        .fold(0, |number, byte| number << 8 | u64::from(byte))
}

fn identity_of(sim: &SimDaliTransport) -> DeviceIdentity {
    DeviceIdentity {
        gtin: number_at(sim, GTIN_OFFSET, GTIN_LEN),
        identification_number: number_at(sim, IDENTIFICATION_NUMBER_OFFSET, IDENTIFICATION_NUMBER_LEN),
    }
}

fn run_write<T: DaliTransport + Send + 'static>(
    controller: &mut DaliController<T>,
    identity: Option<DeviceIdentity>,
    writes: &[FieldWrite],
) -> Outcome {
    let mut readback = None;
    let result = write_memory_luminaire(controller, (SHORT, identity), writes, |read| {
        readback = Some(read.bytes.clone());
    });
    Outcome { result, readback }
}

fn write(lock: u8, unstored: bool, writes: &[FieldWrite]) -> (Outcome, SimDaliTransport) {
    write_on(luminaire_gear(lock, unstored), writes)
}

fn write_on(sim: SimDaliTransport, writes: &[FieldWrite]) -> (Outcome, SimDaliTransport) {
    let identity = identity_of(&sim);
    write_as(sim, Some(identity), writes)
}

fn write_as(sim: SimDaliTransport, identity: Option<DeviceIdentity>, writes: &[FieldWrite]) -> (Outcome, SimDaliTransport) {
    let bus = Arc::new(Mutex::new(sim));
    let mut controller = DaliController::new(Arc::clone(&bus), Box::new(StoppedClock::at(0)));
    let outcome = run_write(&mut controller, identity, writes);
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

fn text_units_in_commit_order(stored: &[u8]) -> Vec<Vec<u8>> {
    let (head, tail) = stored.split_first().expect("a text writes at least its terminator");
    let emptied: Vec<u8> = std::iter::once(0).chain(tail.iter().copied()).collect();
    let mut units: Vec<Vec<u8>> = emptied.chunks(UNIT_BYTES).map(<[u8]>::to_vec).collect();
    units.push(vec![*head]);
    units
}

fn sessions_on_the_wire(stored: &[u8], lock: u8) -> Vec<u8> {
    text_units_in_commit_order(stored)
        .chunks(UNITS_PER_SESSION)
        .flat_map(|session| {
            let body = session.iter().flatten().copied();
            std::iter::once(BANK_UNLOCKED).chain(body).chain(std::iter::once(lock))
        })
        .collect()
}

fn stored_text(text: &str) -> Vec<u8> {
    text.bytes().chain(std::iter::once(0)).collect()
}

fn colour_on_the_gear(sim: &SimDaliTransport) -> Vec<u8> {
    let last = location(sim, LAST_OFFSET_OFFSET).expect("bank 1 is implemented");
    let bank: Vec<u8> = (0..=last).map(|offset| location(sim, offset).unwrap_or(0)).collect();
    part251::text_bytes(&bank, LUMINAIRE_COLOUR).expect("the region fits the bank").to_vec()
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

fn write_under(
    granularity: YieldGranularity,
    trigger: Trigger,
    interruptions: &[Interruption],
    writes: &[FieldWrite],
) -> (Outcome, SimDaliTransport) {
    let gate = interruptions
        .iter()
        .find_map(|interruption| match interruption {
            Interruption::Operator(gate) => Some(Arc::clone(gate)),
            Interruption::ForeignCommand(_) | Interruption::LostAnswer => None,
        })
        .unwrap_or_else(|| Arc::new(WireActivity::new()));
    let sim = luminaire_gear(FOUND_LOCK, false);
    let identity = identity_of(&sim);
    let bus = Arc::new(Mutex::new(Interrupting::new(sim, interruptions.to_vec(), trigger)));
    let mut controller = DaliController::new(Arc::clone(&bus), Box::new(StoppedClock::at(0)));
    let lease = WireLease::new(gate, WirePriority::Attended, granularity);
    let outcome = controller.with_wire_lease(lease, |c| run_write(c, Some(identity), writes));
    drop(controller);
    let interrupting = Arc::into_inner(bus).expect("one owner").into_inner().expect("bus");
    (outcome, interrupting.sim)
}

// IEC 62386-102 §9.10.2
#[test]
fn an_operator_arriving_mid_session_waits_for_the_lock_to_close() {
    let emptying = special(SpecialCommand::WriteMemoryLocation(0));
    let trigger = emptying;
    let operator = Interruption::Operator(Arc::new(WireActivity::new()));
    let writes = text("luminaire_colour", MULTI_SESSION_TEXT);
    let (outcome, sim) = write_under(YieldGranularity::Step, Trigger::Frame(trigger), &[operator], &writes);

    let sim = &sim;
    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::Preempted, "luminaire_colour"),
        "the yield names the field it stopped in, not the lock it never opened"
    );
    let session_len = UNIT_BYTES * UNITS_PER_SESSION + LOCK_FRAMES_PER_SESSION;
    let first_session = &sessions_on_the_wire(&stored_text(MULTI_SESSION_TEXT), FOUND_LOCK)[..session_len];
    assert_eq!(written_values(sim), first_session, "the started session runs to its relock");
    assert_eq!(location(sim, LOCK_OFFSET), Some(FOUND_LOCK));
    assert!(colour_on_the_gear(sim).is_empty(), "the stopped text reads empty, not new head on old tail");
    let frames = sim.sent_frames();
    let relock = special(SpecialCommand::WriteMemoryLocation(FOUND_LOCK));
    assert_eq!(&frames[frames.len() - 2..], [relock, query_dtr0()], "nothing follows the relock unit");
    assert_eq!(outcome.readback, None, "a yielded write leaves the read-back to the next run");
}

// IEC 62386-102 §9.10.5
#[test]
fn a_foreign_command_between_units_costs_nothing_because_each_unit_enables_writing() {
    let end_of_first_unit = MULTI_SESSION_TEXT.as_bytes()[UNIT_BYTES - 1];
    let trigger = special(SpecialCommand::WriteMemoryLocation(end_of_first_unit));
    let foreign = DaliCommand::Standard { address: DaliAddress::Broadcast, command: StandardCommand::QueryStatus };
    let interruption = Interruption::ForeignCommand(foreign.to_forward_frame().raw());
    let writes = text("luminaire_colour", MULTI_SESSION_TEXT);
    let (outcome, sim) = write_under(YieldGranularity::Step, Trigger::Frame(trigger), &[interruption], &writes);

    assert_eq!(shape(outcome.result), Ok(()), "a foreign frame between units clears writeEnableState");
    assert_eq!(location(&sim, LUMINAIRE_COLOUR.offset + UNIT_BYTES as u8), Some(MULTI_SESSION_TEXT.as_bytes()[UNIT_BYTES]));
}

// IEC 62386-102 §9.10.5
#[test]
fn a_gear_that_does_not_advance_dtr0_fails_the_unit_before_the_next_one_writes() {
    let mut sim = luminaire_gear(BANK_UNLOCKED, false);
    sim.fleet_mut().gears_mut()[0].faults.memory_write_holds_dtr0 = true;
    let writes = text("luminaire_colour", MULTI_SESSION_TEXT);
    let (outcome, sim) = write_on(sim, &writes);

    assert_eq!(shape(outcome.result), failure(ErrorCode::VerifyFailed, "luminaire_colour"));
    assert_eq!(
        written_values(&sim).len(),
        UNIT_BYTES,
        "the first unit's DTR0 shows the bytes landed on one location; no second unit is sent"
    );
}

fn is_write_location(frame: u16) -> bool {
    frame >> 8 == special(SpecialCommand::WriteMemoryLocation(0)) >> 8
}

// DiiA 251 §9.2.5
#[test]
fn a_text_stopped_after_any_write_reads_old_empty_or_new() {
    let writes = text("luminaire_colour", MULTI_SESSION_TEXT);
    let (_, calm) = write(FOUND_LOCK, false, &writes);
    let old = colour_on_the_gear(&luminaire_gear(FOUND_LOCK, false));
    let new = MULTI_SESSION_TEXT.as_bytes();
    let frames = calm.sent_frames();
    let writes_at = frames.iter().enumerate().filter(|(_, frame)| is_write_location(**frame));
    for (index, _) in writes_at {
        let operator = Interruption::Operator(Arc::new(WireActivity::new()));
        let (_, sim) = write_under(YieldGranularity::Step, Trigger::Nth(index + 1), &[operator], &writes);
        let seen = colour_on_the_gear(&sim);
        assert!(
            seen == old || seen.is_empty() || seen == new,
            "stopped after frame {}: {:?}",
            index + 1,
            String::from_utf8_lossy(&seen)
        );
        assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK), "frame {}", index + 1);
    }
}

fn position_of(frames: &[u16], frame: u16, last: bool) -> usize {
    let mut positions = frames.iter().enumerate().filter(|(_, sent)| **sent == frame).map(|(index, _)| index + 1);
    let found = if last { positions.next_back() } else { positions.next() };
    found.expect("the frame is on the wire")
}

// IEC 62386-102 §9.10.2
#[test]
fn a_frame_lease_never_leaves_the_bank_open_whichever_frame_the_operator_follows() {
    let writes = year(YEAR);
    let (_, calm) = write(FOUND_LOCK, false, &writes);
    let frames = calm.sent_frames();
    let opened = position_of(&frames, special(SpecialCommand::WriteMemoryLocation(BANK_UNLOCKED)), false);
    let closed = position_of(&frames, special(SpecialCommand::WriteMemoryLocation(FOUND_LOCK)), true);
    assert!(opened < closed);
    for nth in opened..=closed {
        let operator = Interruption::Operator(Arc::new(WireActivity::new()));
        let (outcome, sim) = write_under(YieldGranularity::Frame, Trigger::Nth(nth), &[operator], &writes);
        assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK), "operator after frame {nth}");
        assert_eq!(location(&sim, YEAR_OFFSET), Some(YEAR), "the started session finishes, frame {nth}");
        assert_eq!(
            shape(outcome.result),
            failure(ErrorCode::Preempted, READBACK_SUBJECT),
            "the frame lease yields at the read-back, frame {nth}"
        );
    }
}

// IEC 62386-102 §9.10.5
#[test]
fn a_restore_whose_answer_is_lost_is_proved_by_the_read_back() {
    let relock = special(SpecialCommand::WriteMemoryLocation(FOUND_LOCK));
    let (outcome, sim) = write_under(YieldGranularity::Step, Trigger::Frame(relock), &[Interruption::LostAnswer], &year(YEAR));

    assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK), "the gear kept the restore it did not answer");
    assert_eq!(shape(outcome.result), Ok(()), "the read-back, not the lost answer, says what is written");
}

// IEC 62386-102 §9.10.5
#[test]
fn a_field_whose_answer_is_lost_is_proved_by_the_read_back() {
    let field_byte = special(SpecialCommand::WriteMemoryLocation(YEAR));
    let (outcome, sim) = write_under(YieldGranularity::Step, Trigger::Frame(field_byte), &[Interruption::LostAnswer], &year(YEAR));

    assert_eq!(location(&sim, YEAR_OFFSET), Some(YEAR));
    assert_eq!(shape(outcome.result), Ok(()));
}

// IEC 62386-102 §9.10.2
#[test]
fn a_lost_restore_the_read_back_cannot_check_says_the_bank_may_be_open() {
    let relock = special(SpecialCommand::WriteMemoryLocation(FOUND_LOCK));
    let operator = Interruption::Operator(Arc::new(WireActivity::new()));
    let both = [Interruption::LostAnswer, operator];
    let (outcome, _) = write_under(YieldGranularity::Frame, Trigger::Frame(relock), &both, &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::VerifyUnanswered, RELOCK_SUBJECT),
        "the read-back yielded, so nothing shows the lock went back"
    );
}

fn wrote_nothing(sim: &SimDaliTransport) -> bool {
    let enable = standard(StandardCommand::EnableWriteMemory);
    written_values(sim).is_empty() && !sim.sent_frames().contains(&enable)
}

fn standard(command: StandardCommand) -> u16 {
    DaliCommand::Standard { address: DaliAddress::Short(SHORT), command }.to_forward_frame().raw()
}

// IEC 62386-102 §9.10.6
#[test]
fn a_gear_the_registry_cannot_identify_is_refused_before_any_frame() {
    let (outcome, sim) = write_as(luminaire_gear(FOUND_LOCK, false), None, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Conflict, IDENTITY_UNKNOWN));
    assert!(sim.sent_frames().is_empty(), "nothing is sent to a gear nobody can tell from its replacement");
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn a_gear_replaced_since_the_registry_read_it_is_refused_before_any_write() {
    let read_then = identity_of(&luminaire_gear(FOUND_LOCK, false));
    let replacement = luminaire_gear_at(REPLACEMENT_RANDOM_ADDRESS, FOUND_LOCK, false);
    assert_ne!(identity_of(&replacement), read_then);
    let (outcome, sim) = write_as(replacement, Some(read_then), &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Conflict, IDENTITY_CHANGED));
    assert!(wrote_nothing(&sim), "no ENABLE WRITE MEMORY or WRITE MEMORY LOCATION reaches the other gear");
    assert_eq!(outcome.readback, None);
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn the_identity_is_read_before_the_bank_1_header() {
    let (outcome, sim) = write(FOUND_LOCK, false, &year(YEAR));

    assert_eq!(shape(outcome.result), Ok(()));
    let frames = sim.sent_frames();
    let identity_bank = position_of(&frames, special(SpecialCommand::Dtr1(IDENTITY_BANK)), false);
    let header_bank = position_of(&frames, special(SpecialCommand::Dtr1(PROFILE_BANK)), false);
    assert!(identity_bank < header_bank, "bank 0 is armed before bank 1");
}

// IEC 62386-102 §9.10.6
#[test]
fn an_identity_read_that_yields_stops_the_write_before_any_write() {
    let operator = Interruption::Operator(Arc::new(WireActivity::new()));
    let (outcome, sim) = write_under(YieldGranularity::Frame, Trigger::Nth(1), &[operator], &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Preempted, IDENTITY_SUBJECT));
    assert!(wrote_nothing(&sim));
}
