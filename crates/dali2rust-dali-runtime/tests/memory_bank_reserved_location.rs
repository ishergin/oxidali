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
const PROFILE_BANK: u8 = 1;
const RESERVED_OFFSET: usize = 0x01;
const PLACEHOLDER: u8 = 0xFF;

#[derive(Debug, PartialEq, Eq)]
struct BankRead {
    bank: u8,
    bytes: Vec<u8>,
    indicator_unread: bool,
}

#[derive(Clone, Copy)]
struct Quirks {
    holds_dtr0: bool,
    indicator_missing: bool,
    metered: bool,
}

fn preset(preset: MemoryBankReadPreset, quirks: Quirks) -> Result<Vec<BankRead>, SemanticDaliError> {
    let mut spec = GearSpec::dt6(Some(SHORT), RANDOM_ADDRESS).with_metering(quirks.metered, false);
    spec.holds_dtr0_on_unimplemented = quirks.holds_dtr0;
    spec.indicator_byte_unimplemented = quirks.indicator_missing;
    let bus = Arc::new(Mutex::new(SimDaliTransport::new(vec![spec])));
    let mut controller = DaliController::new(bus, Box::new(StoppedClock::at(0)));
    let mut reads = Vec::new();
    execute_memory_bank_preset(&mut controller, SHORT, preset, |plan, execution| {
        reads.push(BankRead {
            bank: plan.bank,
            bytes: execution.bytes.clone(),
            indicator_unread: execution.indicator_unread,
        });
    })?;
    Ok(reads)
}

const fn quirky(indicator_missing: bool, metered: bool) -> Quirks {
    Quirks { holds_dtr0: true, indicator_missing, metered }
}

const fn conformant(indicator_missing: bool, metered: bool) -> Quirks {
    Quirks { holds_dtr0: false, indicator_missing, metered }
}

// IEC 62386-102 §9.10.4, Table 9
#[test]
fn a_gear_that_holds_dtr0_on_bank_0_location_0x01_still_yields_its_identity() {
    let reads = preset(MemoryBankReadPreset::Identity, quirky(false, false))
        .expect("a lagging DTR0 on the reserved location is not a lost read");

    let expected = preset(MemoryBankReadPreset::Identity, conformant(false, false))
        .expect("a conformant gear yields its identity");
    assert_eq!(reads, expected, "the quirk costs nothing a conformant gear would have answered");
    let identity = &reads[0];
    assert_eq!(identity.bank, IDENTITY_BANK);
    assert_eq!(identity.bytes[RESERVED_OFFSET], PLACEHOLDER, "Table 9: 0x01 is reserved, answer NO");
    assert!(!identity.indicator_unread);
}

// IEC 62386-102 §9.10.2, Table 10
#[test]
fn a_gear_that_holds_dtr0_on_a_missing_indicator_byte_yields_bank_1_with_0x01_unread() {
    let reads = preset(MemoryBankReadPreset::Identity, quirky(true, false))
        .expect("the indicator byte the gear lacks costs no bank");

    let profile = reads.iter().find(|read| read.bank == PROFILE_BANK).expect("bank 1 is read");
    assert!(profile.indicator_unread, "two lags at 0x01 leave it unread, never a value");
    let expected = preset(MemoryBankReadPreset::Identity, conformant(true, false)).expect("conformant");
    assert_eq!(reads, expected, "a NO the pointer passed is no value either");
}

// DiiA 252 §9.2.1
#[test]
fn a_gear_that_holds_dtr0_on_a_missing_indicator_byte_still_yields_its_metered_banks() {
    for metered_preset in [MemoryBankReadPreset::Power, MemoryBankReadPreset::Energy] {
        let reads = preset(metered_preset, quirky(true, true))
            .expect("the bank header read passes 0x01 without failing");

        let expected = preset(metered_preset, conformant(true, true)).expect("conformant");
        assert!(!reads.is_empty(), "{metered_preset:?} read its banks");
        assert_eq!(reads, expected, "{metered_preset:?}");
    }
}
