mod support;

use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_bus::{BusChannel, BusFrame, BusId, CorrelationIdAllocator};
use dali2rust_contracts::msg::{DaliWriteMemoryLuminaireCommand, OperationWorkerSignal, Origin};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{spawn_dali_worker, DaliRuntimeConfig, DaliWorkerCounters};
use dali2rust_contracts::msg::{ErrorCode, LuminaireNumbers as N};
use dali2rust_dali_runtime::runtime::controller::DaliController;
use dali2rust_dali_runtime::runtime::executor::{
    write_memory_luminaire, LuminaireWriteFailure, HEADER_SUBJECT, IDENTITY_CHANGED, IDENTITY_SUBJECT,
    LOCK_SUBJECT, READBACK_SUBJECT, RELOCK_SUBJECT,
};
use dali2rust_domain::dali::banks::identity::{IDENTIFICATION_NUMBER_OFFSET, LOGICAL_GEAR_INDEX_OFFSET};
use dali2rust_domain::dali::banks::{
    DeviceIdentity, IdentityRead, GTIN_OFFSET, IDENTITY_UNKNOWN, IDENTITY_UNPROVABLE,
};
use dali2rust_domain::dali::banks::luminaire_write::{
    number_patch, planned_writes, text_patch, FieldWrite, LuminaireField, BANK_UNLOCKED, TEXT_TERMINATOR,
};
use dali2rust_domain::dali::banks::part251::{self, LuminaireFormat, LUMINAIRE_COLOUR, YEAR_OFFSET};
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::opcode::READ_MEMORY_LOCATION_OPCODE;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_gear_model::GearSpec;
use dali2rust_platform::dali::{
    DaliTransport, TransferOutcome, WireActivity, WireLease, WirePriority, YieldGranularity,
};
use dali2rust_test_support::{publish_queued, StoppedClock};
use support::{next_signal, spawn_signal_bus, ReadPortDouble};

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
const SHORT_TEXT: &str = "ab";
const LIGHT_OUTPUT: u32 = 0x0A_0B_0C;
const WRITE_CORRELATION: u64 = 251;
const QUERY_SELECTOR: u8 = 0x01;
const LOGICAL_GEAR_UNITS: u8 = 2;
const SECOND_GEAR_UNIT: u8 = 1;
const ARM_ATTEMPTS: usize = 3;
const FOREIGN_OFFSET: u8 = 0x10;
const SHORT_BANK0_LAST: u8 = LOGICAL_GEAR_INDEX_OFFSET - 1;

#[derive(Clone)]
enum Interruption {
    Arrival(Arc<WireActivity>, WirePriority),
    ForeignCommand(u16),
    LostAnswer,
    LostFrame,
    BusBusy,
}

#[derive(Clone)]
enum Trigger {
    Frame(u16),
    Nth(usize),
    From(u16),
    After(u16),
    ReadAt(u8, u8),
    Script(u16, Vec<(u16, Interruption)>),
}

#[derive(Debug, Clone, Copy, Default)]
struct Pointer {
    bank: u8,
    offset: u8,
    reads: u8,
}

impl Pointer {
    fn follow(&mut self, frame: u16) -> Option<(u8, u8)> {
        let [opcode, value] = frame.to_be_bytes();
        let opcode_of = |command: SpecialCommand| special(command).to_be_bytes()[0];
        if opcode == opcode_of(SpecialCommand::Dtr1(0)) {
            self.bank = value;
        } else if opcode == opcode_of(SpecialCommand::Dtr0(0)) {
            *self = Self { offset: value, reads: 0, ..*self };
        } else if frame == read_location() {
            let at = self.offset.wrapping_add(self.reads);
            self.reads = self.reads.wrapping_add(1);
            return Some((self.bank, at));
        }
        None
    }
}

struct Interrupting {
    sim: SimDaliTransport,
    interruptions: Vec<Interruption>,
    trigger: Trigger,
    sent: usize,
    started: bool,
    pointer: Pointer,
}

type SimError = <SimDaliTransport as DaliTransport>::Error;

impl Interrupting {
    fn new(sim: SimDaliTransport, interruptions: Vec<Interruption>, trigger: Trigger) -> Self {
        Self { sim, interruptions, trigger, sent: 0, started: false, pointer: Pointer::default() }
    }

    fn fires(&mut self, frame: u16) -> bool {
        self.sent += 1;
        let read = self.pointer.follow(frame);
        match &mut self.trigger {
            Trigger::Frame(at) => frame == *at,
            Trigger::Nth(at) => self.sent == *at,
            Trigger::From(at) => {
                self.started |= frame == *at;
                self.started
            }
            Trigger::After(at) => {
                let fires = self.started;
                self.started |= frame == *at;
                fires
            }
            Trigger::ReadAt(bank, offset) => read == Some((*bank, *offset)),
            Trigger::Script(after, steps) => {
                let armed = self.started;
                self.started |= frame == *after;
                let due = armed && steps.first().is_some_and(|(next, _)| *next == frame);
                if due {
                    self.interruptions = vec![steps.remove(0).1];
                }
                due
            }
        }
    }

    fn withheld(&self) -> Option<TransferOutcome> {
        self.interruptions.iter().find_map(|interruption| match interruption {
            Interruption::LostFrame => Some(TransferOutcome::NoAnswer),
            Interruption::BusBusy => Some(TransferOutcome::BusBusy),
            Interruption::Arrival(..) | Interruption::ForeignCommand(_) | Interruption::LostAnswer => None,
        })
    }

    fn interrupt(&mut self) -> bool {
        let mut lost = false;
        for interruption in &self.interruptions {
            match interruption {
                Interruption::Arrival(gate, priority) => gate.note_arrival(*priority),
                Interruption::ForeignCommand(command) => {
                    self.sim.fleet_mut().exchange(*command, false);
                }
                Interruption::LostAnswer => lost = true,
                Interruption::LostFrame | Interruption::BusBusy => {}
            }
        }
        lost
    }

    fn pass(
        &mut self,
        frame: u16,
        exchange: impl FnOnce(&mut SimDaliTransport) -> Result<TransferOutcome, SimError>,
    ) -> Result<TransferOutcome, SimError> {
        if !self.fires(frame) {
            return exchange(&mut self.sim);
        }
        let outcome = self.withheld().map_or_else(|| exchange(&mut self.sim), Ok);
        let lost = self.interrupt();
        outcome.map(|heard| if lost { TransferOutcome::NoAnswer } else { heard })
    }
}

impl DaliTransport for Interrupting {
    type Error = SimError;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        self.pass(frame, |sim| sim.send_forward_frame(frame).map(|()| TransferOutcome::NoAnswer)).map(|_| ())
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        self.sim.receive_backward_frame()
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        self.sim.is_bus_idle()
    }

    fn exchange_frame(&mut self, frame: u16, expects_backward: bool) -> Result<TransferOutcome, Self::Error> {
        self.pass(frame, |sim| sim.exchange_frame(frame, expects_backward))
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        self.pass(frame, |sim| sim.exchange_frame_with_settle(frame, expects_backward, min_idle_us))
    }
}

fn operator() -> Interruption {
    Interruption::Arrival(Arc::new(WireActivity::new()), WirePriority::Interactive)
}

fn special(command: SpecialCommand) -> u16 {
    DaliCommand::Special(command).to_forward_frame().raw()
}

fn read_location() -> u16 {
    let address = DaliAddress::Short(SHORT).encode_address_byte() | QUERY_SELECTOR;
    ForwardFrame::new(address, READ_MEMORY_LOCATION_OPCODE).raw()
}

fn query_dtr1() -> u16 {
    let address = DaliAddress::Short(SHORT);
    DaliCommand::Standard { address, command: StandardCommand::QueryContentDtr1 }.to_forward_frame().raw()
}

fn query_dtr0() -> u16 {
    let address = DaliAddress::Short(SHORT);
    DaliCommand::Standard { address, command: StandardCommand::QueryContentDtr0 }.to_forward_frame().raw()
}

fn foreign_pointer() -> Interruption {
    Interruption::ForeignCommand(special(SpecialCommand::Dtr0(FOREIGN_OFFSET)))
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

fn identity_of(sim: &SimDaliTransport) -> IdentityRead {
    let gear = &sim.fleet().gears()[0];
    let window: Vec<u8> = (GTIN_OFFSET..=LOGICAL_GEAR_INDEX_OFFSET)
        .map(|at| gear.memory_location(IDENTITY_BANK, at).expect("bank 0 holds the identity window"))
        .collect();
    IdentityRead::of_window(&window)
}

fn known(identity: IdentityRead) -> DeviceIdentity {
    identity.known().expect("the gear model carries a programmed identity")
}

fn run_write<T: DaliTransport + Send + 'static>(
    controller: &mut DaliController<T>,
    identity: IdentityRead,
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
    write_as(sim, identity, writes)
}

fn write_as(sim: SimDaliTransport, identity: IdentityRead, writes: &[FieldWrite]) -> (Outcome, SimDaliTransport) {
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
    let mut units = vec![vec![TEXT_TERMINATOR]];
    units.extend(tail.chunks(UNIT_BYTES).map(<[u8]>::to_vec));
    units.push(vec![*head]);
    units
}

fn sessions_on_the_wire(stored: &[u8], lock: u8) -> Vec<Vec<u8>> {
    text_units_in_commit_order(stored)
        .chunks(UNITS_PER_SESSION)
        .map(|session| {
            let body = session.iter().flatten().copied();
            std::iter::once(BANK_UNLOCKED).chain(body).chain(std::iter::once(lock)).collect()
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
    assert_eq!(written_values(&sim), sessions_on_the_wire(&stored, FOUND_LOCK).concat());
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
    write_under_on(luminaire_gear(FOUND_LOCK, false), (granularity, trigger), interruptions, writes)
}

fn write_under_on(
    sim: SimDaliTransport,
    (granularity, trigger): (YieldGranularity, Trigger),
    interruptions: &[Interruption],
    writes: &[FieldWrite],
) -> (Outcome, SimDaliTransport) {
    let gate = interruptions
        .iter()
        .find_map(|interruption| match interruption {
            Interruption::Arrival(gate, _) => Some(Arc::clone(gate)),
            Interruption::ForeignCommand(_)
            | Interruption::LostAnswer
            | Interruption::LostFrame
            | Interruption::BusBusy => None,
        })
        .unwrap_or_else(|| Arc::new(WireActivity::new()));
    let identity = identity_of(&sim);
    let bus = Arc::new(Mutex::new(Interrupting::new(sim, interruptions.to_vec(), trigger)));
    let mut controller = DaliController::new(Arc::clone(&bus), Box::new(StoppedClock::at(0)));
    let lease = WireLease::new(gate, WirePriority::Attended, granularity);
    let outcome = controller.with_wire_lease(lease, |c| run_write(c, identity, writes));
    drop(controller);
    let interrupting = Arc::into_inner(bus).expect("one owner").into_inner().expect("bus");
    (outcome, interrupting.sim)
}

// IEC 62386-102 §9.10.2
#[test]
fn an_operator_arriving_mid_session_waits_for_the_lock_to_close() {
    let emptying = special(SpecialCommand::WriteMemoryLocation(0));
    let trigger = emptying;
    let operator = operator();
    let writes = text("luminaire_colour", MULTI_SESSION_TEXT);
    let (outcome, sim) = write_under(YieldGranularity::Step, Trigger::Frame(trigger), &[operator], &writes);

    let sim = &sim;
    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::Preempted, "luminaire_colour"),
        "the yield names the field it stopped in, not the lock it never opened"
    );
    let first_session = &sessions_on_the_wire(&stored_text(MULTI_SESSION_TEXT), FOUND_LOCK)[0];
    assert_eq!(&written_values(sim), first_session, "the started session runs to its relock");
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
    let end_of_first_tail_unit = MULTI_SESSION_TEXT.as_bytes()[UNIT_BYTES];
    let trigger = special(SpecialCommand::WriteMemoryLocation(end_of_first_tail_unit));
    let foreign = DaliCommand::Standard { address: DaliAddress::Broadcast, command: StandardCommand::QueryStatus };
    let interruption = Interruption::ForeignCommand(foreign.to_forward_frame().raw());
    let writes = text("luminaire_colour", MULTI_SESSION_TEXT);
    let (outcome, sim) = write_under(YieldGranularity::Step, Trigger::Frame(trigger), &[interruption], &writes);

    assert_eq!(shape(outcome.result), Ok(()), "a foreign frame between units clears writeEnableState");
    let next_unit = UNIT_BYTES + 1;
    assert_eq!(location(&sim, LUMINAIRE_COLOUR.offset + next_unit as u8), Some(MULTI_SESSION_TEXT.as_bytes()[next_unit]));
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
        written_values(&sim),
        [TEXT_TERMINATOR],
        "the terminator's unit shows DTR0 did not move; no tail byte is sent"
    );
    assert_old_empty_or_new(&sim, MULTI_SESSION_TEXT, "a gear holding DTR0");
}

fn is_write_location(frame: u16) -> bool {
    frame >> 8 == special(SpecialCommand::WriteMemoryLocation(0)) >> 8
}

fn assert_old_empty_or_new(sim: &SimDaliTransport, new: &str, context: &str) {
    assert_reads_old_empty_or_new(sim, &colour_on_the_gear(&luminaire_gear(FOUND_LOCK, false)), new, context);
}

fn assert_reads_old_empty_or_new(sim: &SimDaliTransport, old: &[u8], new: &str, context: &str) {
    let seen = colour_on_the_gear(sim);
    assert!(
        seen == old || seen.is_empty() || seen == new.as_bytes(),
        "{context}: {:?}",
        String::from_utf8_lossy(&seen)
    );
}

fn at_each_write_location(new: &str, interruption: impl Fn() -> Interruption) -> Vec<(usize, SimDaliTransport)> {
    at_each_write_location_on(|| luminaire_gear(FOUND_LOCK, false), new, interruption)
}

fn at_each_write_location_on(
    gear: impl Fn() -> SimDaliTransport,
    new: &str,
    interruption: impl Fn() -> Interruption,
) -> Vec<(usize, SimDaliTransport)> {
    let writes = text("luminaire_colour", new);
    let (_, calm) = write_on(gear(), &writes);
    let frames = calm.sent_frames();
    let writes_at = frames.iter().enumerate().filter(|(_, frame)| is_write_location(**frame));
    writes_at
        .map(|(index, _)| {
            let trigger = Trigger::Nth(index + 1);
            (index + 1, write_under_on(gear(), (YieldGranularity::Step, trigger), &[interruption()], &writes).1)
        })
        .collect()
}

fn another_text_of_its_length(old: &[u8]) -> String {
    old.iter().map(|byte| if *byte == b'a' { 'b' } else { 'a' }).collect()
}

fn gear_with_an_emptied_colour() -> SimDaliTransport {
    let mut sim = luminaire_gear(FOUND_LOCK, false);
    sim.fleet_mut().gears_mut()[0].set_bank1_location(LUMINAIRE_COLOUR.offset, TEXT_TERMINATOR);
    sim
}

// DiiA 251 §9.2.5
#[test]
fn a_text_stopped_after_any_write_reads_old_empty_or_new() {
    for (nth, sim) in at_each_write_location(MULTI_SESSION_TEXT, operator) {
        assert_old_empty_or_new(&sim, MULTI_SESSION_TEXT, &format!("stopped after frame {nth}"));
        assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK), "frame {nth}");
    }
}

// DiiA 251 §9.2.5
#[test]
fn a_text_as_long_as_the_old_one_reads_old_empty_or_new_after_any_write() {
    let old = colour_on_the_gear(&luminaire_gear(FOUND_LOCK, false));
    let new = another_text_of_its_length(&old);
    assert_eq!(new.len(), old.len());
    for (nth, sim) in at_each_write_location(&new, operator) {
        assert_old_empty_or_new(&sim, &new, &format!("stopped after frame {nth}"));
        assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK), "frame {nth}");
    }
}

// DiiA 251 §9.2.5
#[test]
fn a_text_over_a_field_whose_first_byte_is_already_zero_reads_empty_or_new_after_any_write() {
    let old = colour_on_the_gear(&gear_with_an_emptied_colour());
    assert!(old.is_empty(), "the old text ends at its first byte");
    for (nth, sim) in at_each_write_location_on(gear_with_an_emptied_colour, MULTI_SESSION_TEXT, operator) {
        assert_reads_old_empty_or_new(&sim, &old, MULTI_SESSION_TEXT, &format!("stopped after frame {nth}"));
        assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK), "frame {nth}");
    }
}

// IEC 62386-102 §9.10.5
// DiiA 251 §9.2.5
#[test]
fn a_text_write_location_the_gear_never_heard_leaves_old_empty_or_new() {
    for (nth, sim) in at_each_write_location(SHORT_TEXT, || Interruption::LostFrame) {
        assert_old_empty_or_new(&sim, SHORT_TEXT, &format!("frame {nth} lost"));
    }
}

// IEC 62386-102 §9.10.5
// DiiA 251 §9.2.5
#[test]
fn a_text_write_location_whose_answer_is_lost_leaves_old_empty_or_new() {
    for (nth, sim) in at_each_write_location(SHORT_TEXT, || Interruption::LostAnswer) {
        assert_old_empty_or_new(&sim, SHORT_TEXT, &format!("answer to frame {nth} lost"));
    }
}

fn light_output_on_the_gear(sim: &SimDaliTransport) -> Vec<Option<u8>> {
    let field = LuminaireField::named("nominal_light_output_lm").expect("number field");
    (field.offset..=field.last_offset()).map(|offset| location(sim, offset)).collect()
}

// IEC 62386-102 §9.10.5
#[test]
fn a_number_whose_first_byte_the_gear_never_heard_is_left_as_found() {
    let writes = planned_writes(&number_patch(&[(N::PATCH_NOMINAL_LIGHT_OUTPUT_LM, LIGHT_OUTPUT)]), None);
    let first_byte = special(SpecialCommand::WriteMemoryLocation(LIGHT_OUTPUT.to_be_bytes()[1]));
    let (outcome, sim) = write_under(YieldGranularity::Step, Trigger::Frame(first_byte), &[Interruption::LostFrame], &writes);

    let found = light_output_on_the_gear(&luminaire_gear(FOUND_LOCK, false));
    assert_ne!(found[0], Some(LIGHT_OUTPUT.to_be_bytes()[2]), "the shifted byte would show");
    assert_eq!(light_output_on_the_gear(&sim), found, "no byte of the unit lands one location early");
    assert_eq!(shape(outcome.result), failure(ErrorCode::VerifyUnanswered, "nominal_light_output_lm"));
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
        let operator = operator();
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
    let operator = operator();
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
    let (outcome, sim) = write_as(luminaire_gear(FOUND_LOCK, false), IdentityRead::Unread, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Conflict, IDENTITY_UNKNOWN));
    assert!(sim.sent_frames().is_empty(), "nothing is sent to a gear nobody can tell from its replacement");
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn a_gear_whose_identity_reads_mask_is_refused_before_any_frame() {
    let (outcome, sim) = write_as(luminaire_gear(FOUND_LOCK, false), IdentityRead::Unprovable, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Conflict, IDENTITY_UNPROVABLE));
    assert!(sim.sent_frames().is_empty(), "an unprogrammed identity tells no gear from another");
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn a_gear_whose_identity_now_reads_mask_is_another_device() {
    let read_then = identity_of(&luminaire_gear(FOUND_LOCK, false));
    let mut sim = luminaire_gear(FOUND_LOCK, false);
    sim.fleet_mut().gears_mut()[0].clear_identification_number();
    assert_eq!(identity_of(&sim), IdentityRead::Unprovable);
    let (outcome, sim) = write_as(sim, read_then, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Conflict, IDENTITY_CHANGED));
    assert!(wrote_nothing(&sim));
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn a_gear_replaced_since_the_registry_read_it_is_refused_before_any_write() {
    let read_then = identity_of(&luminaire_gear(FOUND_LOCK, false));
    let replacement = luminaire_gear_at(REPLACEMENT_RANDOM_ADDRESS, FOUND_LOCK, false);
    assert_ne!(identity_of(&replacement), read_then);
    let (outcome, sim) = write_as(replacement, read_then, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Conflict, IDENTITY_CHANGED));
    assert!(wrote_nothing(&sim), "no ENABLE WRITE MEMORY or WRITE MEMORY LOCATION reaches the other gear");
    assert_eq!(outcome.readback, None);
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn a_gear_replaced_by_one_whose_bank_0_ends_before_the_index_is_another_device() {
    let read_then = identity_of(&luminaire_gear(FOUND_LOCK, false));
    let mut replacement = luminaire_gear_at(REPLACEMENT_RANDOM_ADDRESS, FOUND_LOCK, false);
    replacement.fleet_mut().gears_mut()[0].set_bank0_last_offset(SHORT_BANK0_LAST);
    let (outcome, sim) = write_as(replacement, read_then, &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::Conflict, IDENTITY_CHANGED),
        "its bank 0 declares an end no re-read gets past, so it is not the gear the registry read"
    );
    assert!(wrote_nothing(&sim));
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn another_logical_unit_of_the_same_bus_unit_is_another_device() {
    let read_then = identity_of(&luminaire_gear(FOUND_LOCK, false));
    let mut sim = luminaire_gear(FOUND_LOCK, false);
    sim.fleet_mut().gears_mut()[0].set_logical_gear_unit(LOGICAL_GEAR_UNITS, SECOND_GEAR_UNIT);
    let (then, now) = (known(read_then), known(identity_of(&sim)));
    assert_eq!(
        (now.gtin, now.identification_number),
        (then.gtin, then.identification_number),
        "every logical unit carries the bus unit's GTIN and number"
    );
    let (outcome, sim) = write_as(sim, read_then, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Conflict, IDENTITY_CHANGED));
    assert!(wrote_nothing(&sim));
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
    let operator = operator();
    let (outcome, sim) = write_under(YieldGranularity::Frame, Trigger::Nth(1), &[operator], &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::Preempted, IDENTITY_SUBJECT));
    assert!(wrote_nothing(&sim));
}

fn lost_answers(trigger: Trigger, writes: &[FieldWrite]) -> (Outcome, SimDaliTransport) {
    write_under(YieldGranularity::Step, trigger, &[Interruption::LostAnswer], writes)
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn an_identity_window_that_ends_early_proves_nothing() {
    let cut = Trigger::ReadAt(IDENTITY_BANK, IDENTIFICATION_NUMBER_OFFSET);
    let (outcome, sim) = lost_answers(cut, &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::DeviceAbsent, IDENTITY_SUBJECT),
        "a GTIN without its identification number is not another device"
    );
    assert!(wrote_nothing(&sim));
}

// IEC 62386-102 §9.10.6
#[test]
fn a_gear_silent_from_the_identity_read_is_absent_not_unverified() {
    let silent = Trigger::From(special(SpecialCommand::Dtr1(IDENTITY_BANK)));
    let (outcome, sim) = lost_answers(silent, &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::DeviceAbsent, IDENTITY_SUBJECT),
        "nothing was written, so there is nothing to verify"
    );
    assert!(wrote_nothing(&sim));
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §9.10.4
#[test]
fn an_identity_read_whose_pointer_a_foreign_frame_moves_is_contended_not_absent() {
    let proof = Trigger::Frame(query_dtr1());
    let (outcome, sim) = write_under(YieldGranularity::Step, proof, &[foreign_pointer()], &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::ReadContended, IDENTITY_SUBJECT),
        "the gear answered, with a pointer another master set"
    );
    assert!(wrote_nothing(&sim));
}

// DiiA 252 §9.2.2
// IEC 62386-102 §9.10.4
#[test]
fn a_bank_1_header_silent_inside_its_format_id_is_absent_not_contended() {
    let inside = Trigger::ReadAt(PROFILE_BANK, part251::CONTENT_FORMAT_ID_OFFSET + 1);
    let (outcome, sim) = lost_answers(inside, &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::DeviceAbsent, HEADER_SUBJECT),
        "a gear falling silent inside a latched value is silence, not another answer"
    );
    assert!(wrote_nothing(&sim));
}

// DiiA 252 §9.2.2
// IEC 62386-101 §8.2.5
#[test]
fn a_bank_1_header_whose_format_id_lost_the_bus_inside_it_is_contended() {
    let inside = Trigger::ReadAt(PROFILE_BANK, part251::CONTENT_FORMAT_ID_OFFSET + 1);
    let (outcome, sim) = write_under(YieldGranularity::Step, inside, &[Interruption::BusBusy], &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::ReadContended, HEADER_SUBJECT));
    assert!(wrote_nothing(&sim));
}

// IEC 62386-102 §9.10.4
// DiiA 251 §9.2.3
#[test]
fn a_bank_1_header_that_ends_early_proves_nothing() {
    let cut = Trigger::ReadAt(PROFILE_BANK, part251::CONTENT_FORMAT_ID_OFFSET);
    let (outcome, sim) = lost_answers(cut, &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::DeviceAbsent, HEADER_SUBJECT),
        "a header cut before its format is not a format refusal"
    );
    assert!(wrote_nothing(&sim));
}

// IEC 62386-102 §9.10.4
#[test]
fn a_gear_silent_from_the_bank_1_header_names_the_header() {
    let silent = Trigger::From(special(SpecialCommand::Dtr1(PROFILE_BANK)));
    let (outcome, sim) = lost_answers(silent, &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::DeviceAbsent, HEADER_SUBJECT));
    assert!(wrote_nothing(&sim));
}

// IEC 62386-102 §9.10.4, §9.10.5
#[test]
fn a_read_back_that_ends_early_proves_nothing() {
    let cut = Trigger::ReadAt(PROFILE_BANK, YEAR_OFFSET);
    let (outcome, sim) = lost_answers(cut, &year(YEAR));

    assert_eq!(location(&sim, YEAR_OFFSET), Some(YEAR), "the write landed; only its proof is missing");
    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::VerifyUnanswered, READBACK_SUBJECT),
        "a read-back that stops before the field is not a field the gear refused"
    );
}

// IEC 62386-102 §9.10.2, Table 8
#[test]
fn a_lock_whose_pointer_was_never_proved_is_neither_restored_nor_reported_open() {
    let silent = Trigger::From(special(SpecialCommand::Dtr0(LOCK_OFFSET)));
    let (outcome, sim) = lost_answers(silent, &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::DeviceAbsent, LOCK_SUBJECT),
        "no ENABLE WRITE MEMORY reached the gear, so its lock is as found"
    );
    assert!(wrote_nothing(&sim));
    assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK));
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §9.10.2, Table 8
#[test]
fn a_lock_open_the_bus_never_let_through_is_neither_restored_nor_reported_open() {
    let busy = Trigger::From(special(SpecialCommand::Dtr0(LOCK_OFFSET)));
    let (outcome, sim) = write_under(YieldGranularity::Step, busy, &[Interruption::BusBusy], &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::ReadContended, LOCK_SUBJECT),
        "the open stopped before its write, so nothing is open and nothing was verified"
    );
    assert!(wrote_nothing(&sim));
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §9.10.2, Table 8
#[test]
fn a_lock_open_whose_pointer_a_foreign_frame_moves_is_contended_and_touches_nothing() {
    let moved = Trigger::From(special(SpecialCommand::Dtr0(LOCK_OFFSET)));
    let (outcome, sim) = write_under(YieldGranularity::Step, moved, &[foreign_pointer()], &year(YEAR));

    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::ReadContended, LOCK_SUBJECT),
        "a pointer another master moved is the bus, not a failed operation"
    );
    assert!(wrote_nothing(&sim));
    assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK));
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §9.10.2, Table 8
#[test]
fn a_lock_open_that_reached_the_gear_and_then_lost_the_bus_says_the_bank_may_be_open() {
    let busy = Trigger::After(special(SpecialCommand::WriteMemoryLocation(BANK_UNLOCKED)));
    let (outcome, sim) = write_under(YieldGranularity::Step, busy, &[Interruption::BusBusy], &year(YEAR));

    assert_eq!(location(&sim, LOCK_OFFSET), Some(BANK_UNLOCKED), "the gear did open");
    assert_eq!(
        shape(outcome.result),
        failure(ErrorCode::VerifyContended, RELOCK_SUBJECT),
        "nothing proves the lock went back"
    );
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §9.10.2, Table 8
#[test]
fn a_lock_open_that_reached_its_write_is_restored_even_when_its_retries_never_hold_again() {
    let unlock = special(SpecialCommand::WriteMemoryLocation(BANK_UNLOCKED));
    let mut steps = vec![(query_dtr0(), Interruption::BusBusy)];
    steps.extend(std::iter::repeat_n((query_dtr1(), Interruption::LostAnswer), ARM_ATTEMPTS));
    let (outcome, sim) = write_under(YieldGranularity::Step, Trigger::Script(unlock, steps), &[], &year(YEAR));

    assert_eq!(written_values(&sim), [BANK_UNLOCKED, FOUND_LOCK], "the gear opened, so it is closed");
    assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK));
    assert_eq!(shape(outcome.result), failure(ErrorCode::VerifyUnanswered, LOCK_SUBJECT));
}

// IEC 62386-102 §9.10.2, §9.10.5
#[test]
fn a_lock_opened_without_an_answer_is_still_restored() {
    let unlock = special(SpecialCommand::WriteMemoryLocation(BANK_UNLOCKED));
    let (outcome, sim) = lost_answers(Trigger::Frame(unlock), &year(YEAR));

    assert_eq!(shape(outcome.result), failure(ErrorCode::VerifyUnanswered, LOCK_SUBJECT));
    assert_eq!(written_values(&sim), [BANK_UNLOCKED, FOUND_LOCK], "the gear may have opened, so it is closed");
    assert_eq!(location(&sim, LOCK_OFFSET), Some(FOUND_LOCK));
}

fn worker_command(text: &str) -> BusFrame {
    let field = LuminaireField::named("luminaire_colour").expect("text field");
    let command = DaliWriteMemoryLuminaireCommand {
        registry_adapter_id: 0,
        short_address: SHORT,
        numbers: N::default(),
        text: text_patch(field, text),
    };
    let envelope = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        WRITE_CORRELATION,
        BusId::default().0,
        Some(Origin::Api),
        command,
    );
    BusFrame::command(envelope)
}

fn write_by_the_worker(
    transport: Arc<Mutex<Interrupting>>,
    gate: Arc<WireActivity>,
    identity: IdentityRead,
) -> Result<(), (ErrorCode, String)> {
    let (host, publisher, worker_commands, signals) = spawn_signal_bus();
    let worker = spawn_dali_worker(
        worker_commands,
        DaliController::new(transport, Box::new(StoppedClock::at(0))),
        DaliRuntimeConfig::default(),
        Arc::new(ReadPortDouble { identity, ..ReadPortDouble::default() }),
        publisher.clone(),
        BusId::default(),
        Arc::new(DaliWorkerCounters::default()),
        gate,
        Arc::new(CorrelationIdAllocator::new()),
    );
    publish_queued(&publisher, BusChannel::Commands, worker_command(MULTI_SESSION_TEXT));
    let started = next_signal(&signals, WRITE_CORRELATION).map(|signal| signal.signal);
    assert_eq!(started, Some(OperationWorkerSignal::WorkerStarted));
    let ended = next_signal(&signals, WRITE_CORRELATION).expect("the write ends");
    drop((host, publisher));
    worker.join().expect("the worker stops with its bus");
    ended.error.map_or(Ok(()), |error| Err((error.code, error.message.as_str().to_owned())))
}

fn through_the_worker(arrival: WirePriority) -> (Result<(), (ErrorCode, String)>, SimDaliTransport) {
    let gate = Arc::new(WireActivity::new());
    let sim = luminaire_gear(FOUND_LOCK, false);
    let identity = identity_of(&sim);
    let emptying = Trigger::Frame(special(SpecialCommand::WriteMemoryLocation(TEXT_TERMINATOR)));
    let interruption = Interruption::Arrival(Arc::clone(&gate), arrival);
    let transport = Arc::new(Mutex::new(Interrupting::new(sim, vec![interruption], emptying)));
    let result = write_by_the_worker(Arc::clone(&transport), gate, identity);
    let interrupting = Arc::into_inner(transport).expect("one owner").into_inner().expect("bus");
    (result, interrupting.sim)
}

fn assert_yielded_after_the_first_session(result: Result<(), (ErrorCode, String)>, sim: &SimDaliTransport) {
    assert_eq!(result, failure(ErrorCode::Preempted, "luminaire_colour"));
    let first_session = &sessions_on_the_wire(&stored_text(MULTI_SESSION_TEXT), FOUND_LOCK)[0];
    assert_eq!(&written_values(sim), first_session, "the started session runs to its relock, and no other starts");
    assert_eq!(location(sim, LOCK_OFFSET), Some(FOUND_LOCK));
    assert!(colour_on_the_gear(sim).is_empty());
}

// IEC 62386-102 §9.10.2
#[test]
fn the_priority_table_lets_an_operator_in_only_after_the_lock_is_restored() {
    let (result, sim) = through_the_worker(WirePriority::Interactive);

    assert_yielded_after_the_first_session(result, &sim);
}

// IEC 62386-103 §9.13.1
#[test]
fn home_assistant_hcl_and_background_work_wait_for_the_whole_write() {
    for arrival in [WirePriority::Attended, WirePriority::Unattended] {
        let (result, sim) = through_the_worker(arrival);

        assert_eq!(result, Ok(()), "{arrival:?} work does not cut into a write it queues behind");
        assert_eq!(colour_on_the_gear(&sim), MULTI_SESSION_TEXT.as_bytes(), "{arrival:?}");
    }
}
