use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_contracts::msg::MemoryBankReadPreset;
use dali2rust_dali_runtime::runtime::controller::DaliController;
use dali2rust_dali_runtime::runtime::executor::{execute_memory_bank_preset, SemanticDaliError};
use dali2rust_gear_model::GearSpec;
use dali2rust_test_support::StoppedClock;

const SHORT: u8 = 3;
const RANDOM_ADDRESS: u32 = 0x00_4321;
const IDENTITY_BANK: u8 = 0;
const RESERVED_OFFSET: usize = 0x01;
const PLACEHOLDER: u8 = 0xFF;

type BankReads = Vec<(u8, Vec<u8>)>;

fn identity_preset(holds_dtr0_on_unimplemented: bool) -> Result<BankReads, SemanticDaliError> {
    let mut spec = GearSpec::dt6(Some(SHORT), RANDOM_ADDRESS);
    spec.holds_dtr0_on_unimplemented = holds_dtr0_on_unimplemented;
    let bus = Arc::new(Mutex::new(SimDaliTransport::new(vec![spec])));
    let mut controller = DaliController::new(bus, Box::new(StoppedClock::at(0)));
    let mut reads = Vec::new();
    execute_memory_bank_preset(&mut controller, SHORT, MemoryBankReadPreset::Identity, |plan, execution| {
        reads.push((plan.bank, execution.bytes.clone()));
    })?;
    Ok(reads)
}

// IEC 62386-102 §9.10.4, Table 9
#[test]
fn a_gear_that_holds_dtr0_on_bank_0_location_0x01_still_yields_its_identity() {
    let reads = identity_preset(true).expect("a lagging DTR0 on the reserved location is not a lost read");

    let conformant = identity_preset(false).expect("a conformant gear yields its identity");
    assert_eq!(reads, conformant, "the quirk costs nothing a conformant gear would have answered");
    let (bank, bytes) = &reads[0];
    assert_eq!(*bank, IDENTITY_BANK);
    assert_eq!(bytes[RESERVED_OFFSET], PLACEHOLDER, "Table 9: 0x01 is reserved, answer NO");
}
