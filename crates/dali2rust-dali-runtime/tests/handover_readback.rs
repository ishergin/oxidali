use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_dali_runtime::runtime::controller::{DaliController, PHY_TICK_US, TX_ARM_LEAD_TICKS};
use dali2rust_dali_runtime::runtime::executor::arbitration::enable_peer_controller;
use dali2rust_dali_runtime::runtime::executor::SemanticDaliError;
use dali2rust_domain::dali::controller::{DaliApplicationController, WireHold};
use dali2rust_domain::dali::dev103::address::Device103Address;
use dali2rust_domain::dali::dev103::command::Device103Command;
use dali2rust_domain::dali::frame::HoldCause;
use dali2rust_domain::dali::ses::DaliPriority;
use dali2rust_platform::dali::TransferOutcome;
use dali2rust_test_support::StoppedClock;

const PEER: u8 = 7;
const YES: u8 = 0xFF;

fn at_peer(command: Device103Command) -> [u8; 3] {
    command.frame(Device103Address::Short(PEER)).as_bytes()
}

fn enable() -> [u8; 3] {
    at_peer(Device103Command::EnableApplicationController)
}

fn query() -> [u8; 3] {
    at_peer(Device103Command::QueryApplicationControlEnabled)
}

struct QuiescentOnceEnabled(Arc<Mutex<MockDaliTransport>>);

impl WireHold for QuiescentOnceEnabled {
    fn held(&self) -> Option<HoldCause> {
        let sent = self.0.lock().expect("bus").sent_frames24();
        sent.contains(&enable()).then_some(HoldCause::Quiescent)
    }
}

fn bus(answer: Option<u8>) -> Arc<Mutex<MockDaliTransport>> {
    let bus = MockDaliTransport::new();
    if let Some(answer) = answer {
        bus.script_frame24_answer(query(), answer);
    }
    Arc::new(Mutex::new(bus))
}

fn hand_over(bus: &Arc<Mutex<MockDaliTransport>>, hold: Arc<dyn WireHold>) -> Result<(), SemanticDaliError> {
    let mut controller = DaliController::new(Arc::clone(bus), Box::new(StoppedClock::at(0)));
    controller.with_wire_hold(hold, |c| enable_peer_controller(c, PEER))
}

fn first_pass_ends_with(bus: &Arc<Mutex<MockDaliTransport>>, query_outcome: TransferOutcome) {
    let bus = bus.lock().expect("bus");
    bus.script_frame24_outcome(TransferOutcome::NoAnswer);
    bus.script_frame24_outcome(TransferOutcome::NoAnswer);
    bus.script_frame24_outcome(query_outcome);
}

fn sent(bus: &Arc<Mutex<MockDaliTransport>>) -> Vec<[u8; 3]> {
    bus.lock().expect("bus").sent_frames24()
}

fn continues_the_transaction(bus: &Arc<Mutex<MockDaliTransport>>, frame: usize) -> bool {
    let settle_us = bus.lock().expect("bus").sent_frame24_settle_us()[frame];
    DaliPriority::Transaction.contains_settle_us(settle_us + u32::from(TX_ARM_LEAD_TICKS) * PHY_TICK_US)
}

struct Free;

impl WireHold for Free {
    fn held(&self) -> Option<HoldCause> {
        None
    }
}

// IEC 62386-103 §11.6.16
#[test]
fn a_peer_that_answers_after_its_enable_takes_the_bus() {
    let bus = bus(Some(YES));
    assert_eq!(hand_over(&bus, Arc::new(Free)), Ok(()));
    assert_eq!(sent(&bus), vec![enable(), enable(), query()]);
}

// IEC 62386-103 §11.6.16
#[test]
fn a_silent_peer_is_named_and_the_bus_stays_here() {
    let bus = bus(None);
    assert_eq!(hand_over(&bus, Arc::new(Free)), Err(SemanticDaliError::Conflict("peer_not_enabled")));
    assert_eq!(sent(&bus), vec![enable(), enable(), query()]);
}

// IEC 62386-103 §9.9.3
#[test]
fn a_hold_that_starts_after_the_enable_still_asks_the_peer() {
    let bus = bus(Some(YES));
    assert_eq!(hand_over(&bus, Arc::new(QuiescentOnceEnabled(Arc::clone(&bus)))), Ok(()));
    assert_eq!(sent(&bus), vec![enable(), enable(), query()]);
}

// IEC 62386-101 §9.1.4
#[test]
fn a_collided_query_is_asked_again_and_its_enable_pair_is_not() {
    let bus = bus(Some(YES));
    first_pass_ends_with(&bus, TransferOutcome::Collision);
    assert_eq!(hand_over(&bus, Arc::new(Free)), Ok(()));
    assert_eq!(sent(&bus), vec![enable(), enable(), query(), query()]);
    assert!(continues_the_transaction(&bus, 3), "the query is asked again at priority 1");
}

// IEC 62386-101 §8.2.5
#[test]
fn a_garbled_answer_names_an_address_clash_and_the_bus_stays_here() {
    let bus = bus(Some(YES));
    first_pass_ends_with(&bus, TransferOutcome::CorruptedInWindow);
    assert_eq!(hand_over(&bus, Arc::new(Free)), Err(SemanticDaliError::Conflict("peer_answer_contended")));
    assert_eq!(sent(&bus), vec![enable(), enable(), query()]);
}

// IEC 62386-103 §9.9.3
#[test]
fn a_hold_that_starts_after_the_enable_still_asks_again_after_a_collision() {
    let bus = bus(Some(YES));
    first_pass_ends_with(&bus, TransferOutcome::Collision);
    assert_eq!(hand_over(&bus, Arc::new(QuiescentOnceEnabled(Arc::clone(&bus)))), Ok(()));
    assert_eq!(sent(&bus), vec![enable(), enable(), query(), query()]);
}
