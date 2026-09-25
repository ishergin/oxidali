use dali2rust_domain::dali::commands::DaliCommand;
use dali2rust_domain::dali::devices::dt8_color::{
    colour_value_is_wide, Dt8Command, COLOUR_STATUS_TC_OUT_OF_RANGE, COLOUR_TYPE_BYTE_TC,
    COLOUR_VALUE_NUMBER_OF_PRIMARIES, COLOUR_VALUE_RED, COLOUR_VALUE_REPORT_COLOUR_TYPE,
    COLOUR_VALUE_REPORT_TC, COLOUR_VALUE_TC, COLOUR_VALUE_TC_COOLEST,
    COLOUR_VALUE_TC_PHYSICAL_WARMEST, COLOUR_VALUE_TC_WARMEST, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE,
    COLOUR_VALUE_TEMPORARY_TC, COLOUR_VALUE_X, COLOUR_VALUE_Y, TC_LIMIT_SELECTOR_COOLEST,
    TC_LIMIT_SELECTOR_WARMEST,
};
use dali2rust_domain::dali::devices::dt6_led::Dt6Command;
use dali2rust_domain::dali::devices::DeviceType;
use dali2rust_domain::dali::pres::opcode::READ_MEMORY_LOCATION_OPCODE;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::banks::part251::LuminaireFormat;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_platform::dali::TransferOutcome;

use crate::fleet::{demo_random_address, encode_short_reply, GearFleet, DEFAULT_RESERVED_SHORT_ADDRESSES};
use crate::gear::{
    GearSpec, BANK0_BASE_LAST_OFFSET, BANK0_LAST_BANK, BANK1_UNLOCKED, DALI_YES, DT8_STATUS_TC_ACTIVE,
    MEMORY_BANK_IDENTITY, MEMORY_BANK_PROFILE, STATUS_GEAR_FAILURE, STATUS_LAMP_FAILURE,
    STATUS_LAMP_ON, STATUS_LIMIT_ERROR, STATUS_POWER_CYCLE_SEEN,
};

fn frame(command: DaliCommand) -> u16 {
    let f = command.to_forward_frame();
    (u16::from(f.address_byte()) << 8) | u16::from(f.command_byte())
}

fn short(addr: u8) -> DaliAddress {
    DaliAddress::short(addr).unwrap()
}

fn standard(addr: u8, command: StandardCommand) -> u16 {
    frame(DaliCommand::Standard {
        address: short(addr),
        command,
    })
}

fn special(command: SpecialCommand) -> u16 {
    frame(DaliCommand::Special(command))
}

fn dt8(addr: u8, command: Dt8Command) -> u16 {
    (u16::from(short(addr).encode_address_byte() | 0x01) << 8) | u16::from(command.opcode())
}

fn exchange(fleet: &mut GearFleet, f: u16, backward: bool) -> TransferOutcome {
    fleet.exchange(f, backward)
}

fn config(fleet: &mut GearFleet, f: u16) {
    assert!(
        fleet.frame_pairs_now(f),
        "frame {f:#06X} is not a send-twice command — sending it twice is two commands"
    );
    exchange(fleet, f, false);
    exchange(fleet, f, false);
}

fn enable_dt8(fleet: &mut GearFleet) {
    exchange(
        fleet,
        special(SpecialCommand::EnableDeviceType(DeviceType::Color.code())),
        false,
    );
}

#[test]
fn dapc_sets_level_and_actual_level_query_reads_it_back() {
    let mut fleet = GearFleet::demo_bus();
    let dapc = standard(0, StandardCommand::DirectArcPower { level: 120 });
    exchange(&mut fleet, dapc, false);
    let reply = exchange(&mut fleet, standard(0, StandardCommand::QueryActualLevel), true);
    assert_eq!(reply, TransferOutcome::Answer(120));
}

#[test]
fn control_gear_present_answers_only_for_addressed_gear() {
    let mut fleet = GearFleet::demo_bus();
    let present = exchange(
        &mut fleet,
        standard(0, StandardCommand::QueryControlGearPresent),
        true,
    );
    assert_eq!(present, TransferOutcome::Answer(DALI_YES));
    let absent = exchange(
        &mut fleet,
        standard(40, StandardCommand::QueryControlGearPresent),
        true,
    );
    assert_eq!(absent, TransferOutcome::NoAnswer);
}

#[test]
fn group_membership_programs_and_reads_back() {
    let mut fleet = GearFleet::demo_bus();
    config(&mut fleet, standard(1, StandardCommand::AddToGroup { group: 3 }));
    let low = exchange(
        &mut fleet,
        standard(1, StandardCommand::QueryGroups0To7),
        true,
    );
    assert_eq!(low, TransferOutcome::Answer(1 << 3));
    config(
        &mut fleet,
        standard(1, StandardCommand::RemoveFromGroup { group: 3 }),
    );
    let cleared = exchange(
        &mut fleet,
        standard(1, StandardCommand::QueryGroups0To7),
        true,
    );
    assert_eq!(cleared, TransferOutcome::Answer(0));
}

#[test]
fn scene_program_recall_and_query() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(90)), false);
    config(&mut fleet, standard(2, StandardCommand::SetScene { scene: 5 }));
    let stored = exchange(
        &mut fleet,
        standard(2, StandardCommand::QuerySceneLevel { scene: 5 }),
        true,
    );
    assert_eq!(stored, TransferOutcome::Answer(90));
    exchange(
        &mut fleet,
        standard(2, StandardCommand::GoToScene { scene: 5 }),
        false,
    );
    let level = exchange(
        &mut fleet,
        standard(2, StandardCommand::QueryActualLevel),
        true,
    );
    assert_eq!(level, TransferOutcome::Answer(90));
}

#[test]
fn group_recall_moves_members_only_and_mask_member_stays() {
    let mut fleet = GearFleet::demo_bus();
    let group_frame = |command: StandardCommand| {
        frame(DaliCommand::Standard {
            address: DaliAddress::group(7).unwrap(),
            command,
        })
    };
    config(&mut fleet, standard(1, StandardCommand::AddToGroup { group: 7 }));
    config(&mut fleet, standard(2, StandardCommand::AddToGroup { group: 7 }));
    exchange(&mut fleet, special(SpecialCommand::Dtr0(90)), false);
    config(&mut fleet, standard(1, StandardCommand::SetScene { scene: 5 }));
    exchange(&mut fleet, special(SpecialCommand::Dtr0(70)), false);
    config(&mut fleet, standard(3, StandardCommand::SetScene { scene: 5 }));
    exchange(
        &mut fleet,
        frame(DaliCommand::Standard {
            address: DaliAddress::Broadcast,
            command: StandardCommand::DirectArcPower { level: 40 },
        }),
        false,
    );

    exchange(&mut fleet, group_frame(StandardCommand::GoToScene { scene: 5 }), false);

    let level = |fleet: &mut GearFleet, short: u8| {
        exchange(fleet, standard(short, StandardCommand::QueryActualLevel), true)
    };
    assert_eq!(level(&mut fleet, 1), TransferOutcome::Answer(90), "member with a slot moves");
    assert_eq!(
        level(&mut fleet, 2),
        TransferOutcome::Answer(40),
        "member whose sceneX is MASK is not touched (§9.19)"
    );
    assert_eq!(
        level(&mut fleet, 3),
        TransferOutcome::Answer(40),
        "a non-member holds its level though its slot is programmed"
    );
}

#[test]
fn min_level_write_clamps_to_roof_and_flags_limit_error_on_a_lit_lamp() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    config(&mut fleet, standard(1, StandardCommand::SetMaxLevel));
    exchange(
        &mut fleet,
        standard(1, StandardCommand::DirectArcPower { level: 60 }),
        false,
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr0(220)), false);
    config(&mut fleet, standard(1, StandardCommand::SetMinLevel));

    let min = exchange(&mut fleet, standard(1, StandardCommand::QueryMinLevel), true);
    assert_eq!(min, TransferOutcome::Answer(200), "accepted, not requested");
    let level = exchange(&mut fleet, standard(1, StandardCommand::QueryActualLevel), true);
    assert_eq!(level, TransferOutcome::Answer(200), "lit lamp pulled to the new floor");
    let status = exchange(&mut fleet, standard(1, StandardCommand::QueryStatus), true);
    let TransferOutcome::Answer(status) = status else {
        panic!("status must answer");
    };
    assert_ne!(status & 0x08, 0, "limitError (bit 3) reports the pull");

    exchange(
        &mut fleet,
        standard(1, StandardCommand::DirectArcPower { level: 200 }),
        false,
    );
    let status = exchange(&mut fleet, standard(1, StandardCommand::QueryStatus), true);
    let TransferOutcome::Answer(status) = status else {
        panic!("status must answer");
    };
    assert_eq!(status & 0x08, 0, "an in-range DAPC resets limitError");
}

#[test]
fn min_level_write_clamps_to_the_physical_minimum() {
    let mut spec = GearSpec::dt6(Some(1), 0x00AB_CDEF);
    spec.phm = 10;
    let mut fleet = GearFleet::new(vec![spec], 0, 7);
    assert_eq!(
        exchange(
            &mut fleet,
            standard(1, StandardCommand::QueryPhysicalMinimum),
            true
        ),
        TransferOutcome::Answer(10)
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr0(5)), false);
    config(&mut fleet, standard(1, StandardCommand::SetMinLevel));
    assert_eq!(
        exchange(&mut fleet, standard(1, StandardCommand::QueryMinLevel), true),
        TransferOutcome::Answer(10),
        "the PHM floor, not the request"
    );
}

#[test]
fn max_level_write_clamps_to_the_floor_and_pulls_a_lit_lamp_down() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(100)), false);
    config(&mut fleet, standard(1, StandardCommand::SetMinLevel));
    exchange(
        &mut fleet,
        standard(1, StandardCommand::DirectArcPower { level: 220 }),
        false,
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr0(50)), false);
    config(&mut fleet, standard(1, StandardCommand::SetMaxLevel));
    assert_eq!(
        exchange(&mut fleet, standard(1, StandardCommand::QueryMaxLevel), true),
        TransferOutcome::Answer(100),
        "minLevel >= DTR0 -> minLevel"
    );
    assert_eq!(
        exchange(&mut fleet, standard(1, StandardCommand::QueryActualLevel), true),
        TransferOutcome::Answer(100),
        "lit lamp pulled to the new roof"
    );
    let TransferOutcome::Answer(status) =
        exchange(&mut fleet, standard(1, StandardCommand::QueryStatus), true)
    else {
        panic!("status must answer");
    };
    assert_ne!(status & 0x08, 0, "limitError reports the pull-down");
    exchange(&mut fleet, special(SpecialCommand::Dtr0(255)), false);
    config(&mut fleet, standard(1, StandardCommand::SetMaxLevel));
    assert_eq!(
        exchange(&mut fleet, standard(1, StandardCommand::QueryMaxLevel), true),
        TransferOutcome::Answer(0xFE),
        "MASK -> 0xFE"
    );
}

#[test]
fn limit_error_reports_a_bounds_clamped_dapc_and_ignores_mask() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    config(&mut fleet, standard(1, StandardCommand::SetMaxLevel));
    exchange(
        &mut fleet,
        standard(1, StandardCommand::DirectArcPower { level: 250 }),
        false,
    );
    let TransferOutcome::Answer(status) =
        exchange(&mut fleet, standard(1, StandardCommand::QueryStatus), true)
    else {
        panic!("status must answer");
    };
    assert_ne!(status & 0x08, 0, "a clamped DAPC raises limitError");
    exchange(
        &mut fleet,
        standard(1, StandardCommand::DirectArcPower { level: 255 }),
        false,
    );
    let TransferOutcome::Answer(status) =
        exchange(&mut fleet, standard(1, StandardCommand::QueryStatus), true)
    else {
        panic!("status must answer");
    };
    assert_ne!(status & 0x08, 0, "DAPC(MASK) leaves limitError untouched");
}

#[test]
fn a_special_frame_cancels_the_device_type_walk() {
    let mut fleet = multi_type_fleet();
    assert_eq!(
        exchange(&mut fleet, standard(8, StandardCommand::QueryDeviceType), true),
        TransferOutcome::Answer(255)
    );
    assert_eq!(
        exchange(
            &mut fleet,
            standard(8, StandardCommand::QueryNextDeviceType),
            true
        ),
        TransferOutcome::Answer(6)
    );
    exchange(&mut fleet, special(SpecialCommand::EnableDeviceType(8)), false);
    assert_eq!(
        exchange(
            &mut fleet,
            standard(8, StandardCommand::QueryNextDeviceType),
            true
        ),
        TransferOutcome::NoAnswer,
        "a heard special breaks the directly-preceded chain"
    );
}

#[test]
fn a_send_twice_first_half_cancels_the_device_type_walk() {
    let mut fleet = multi_type_fleet();
    assert_eq!(
        exchange(&mut fleet, standard(8, StandardCommand::QueryDeviceType), true),
        TransferOutcome::Answer(255)
    );
    assert_eq!(
        exchange(
            &mut fleet,
            standard(8, StandardCommand::QueryNextDeviceType),
            true
        ),
        TransferOutcome::Answer(6)
    );
    exchange(&mut fleet, standard(8, StandardCommand::Reset), false);
    assert_eq!(
        exchange(
            &mut fleet,
            standard(8, StandardCommand::QueryNextDeviceType),
            true
        ),
        TransferOutcome::NoAnswer,
        "a held first instance still cancels the walk"
    );
}

fn multi_type_fleet() -> GearFleet {
    let mut spec = GearSpec::dt8(Some(8), 0x00AB_CDEF, (true, false, false), (153, 370));
    spec.device_types = vec![6, 8];
    GearFleet::new(vec![spec], 0, 7)
}

#[test]
fn a_single_type_gear_answers_no_to_query_next() {
    let mut fleet = GearFleet::demo_bus();
    let first = exchange(&mut fleet, standard(1, StandardCommand::QueryDeviceType), true);
    assert_eq!(first, TransferOutcome::Answer(6));
    let next = exchange(
        &mut fleet,
        standard(1, StandardCommand::QueryNextDeviceType),
        true,
    );
    assert_eq!(next, TransferOutcome::NoAnswer, "no walk was armed (§11.5.13)");
}

#[test]
fn an_interleaved_command_cancels_the_device_type_walk() {
    let mut fleet = multi_type_fleet();
    let first = exchange(&mut fleet, standard(8, StandardCommand::QueryDeviceType), true);
    assert_eq!(first, TransferOutcome::Answer(255), "multi-type advertises MASK");
    assert_eq!(
        exchange(&mut fleet, standard(8, StandardCommand::QueryNextDeviceType), true),
        TransferOutcome::Answer(6)
    );
    let _ = exchange(&mut fleet, standard(8, StandardCommand::QueryActualLevel), true);
    assert_eq!(
        exchange(&mut fleet, standard(8, StandardCommand::QueryNextDeviceType), true),
        TransferOutcome::NoAnswer,
        "cancelled walk answers NO, never 254"
    );

    assert_eq!(
        exchange(&mut fleet, standard(8, StandardCommand::QueryDeviceType), true),
        TransferOutcome::Answer(255)
    );
    for expected in [6u8, 8] {
        assert_eq!(
            exchange(&mut fleet, standard(8, StandardCommand::QueryNextDeviceType), true),
            TransferOutcome::Answer(expected)
        );
    }
    assert_eq!(
        exchange(&mut fleet, standard(8, StandardCommand::QueryNextDeviceType), true),
        TransferOutcome::Answer(254),
        "the honest end of a complete walk"
    );
}

#[test]
fn dt8_gear_advertises_type_and_cct_write_updates_colour_status() {
    let mut fleet = GearFleet::demo_bus();
    let dt = exchange(
        &mut fleet,
        standard(4, StandardCommand::QueryDeviceType),
        true,
    );
    assert_eq!(dt, TransferOutcome::Answer(DeviceType::Color.code()));

    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x50)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x01)), false);
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(4, Dt8Command::SetTemporaryColourTemperature),
        false,
    );

    enable_dt8(&mut fleet);
    let staged_only = exchange(&mut fleet, dt8(4, Dt8Command::QueryColourStatus), true);
    assert_eq!(
        staged_only,
        TransferOutcome::Answer(0),
        "Tc staged, not active"
    );

    enable_dt8(&mut fleet);
    exchange(&mut fleet, dt8(4, Dt8Command::Activate), false);

    enable_dt8(&mut fleet);
    let status = exchange(&mut fleet, dt8(4, Dt8Command::QueryColourStatus), true);
    assert_eq!(status, TransferOutcome::Answer(DT8_STATUS_TC_ACTIVE));
}

#[test]
fn memory_bank_identity_reads_sequentially_with_dtr0_autoincrement() {
    let mut fleet = GearFleet::demo_bus();
    exchange(
        &mut fleet,
        special(SpecialCommand::Dtr1(MEMORY_BANK_IDENTITY)),
        false,
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0)), false);
    let read_frame = (u16::from(short(0).encode_address_byte() | 0x01) << 8)
        | u16::from(READ_MEMORY_LOCATION_OPCODE);
    let TransferOutcome::Answer(last_location) = exchange(&mut fleet, read_frame, true) else {
        panic!("offset 0x00 must answer");
    };
    assert_eq!(last_location, BANK0_BASE_LAST_OFFSET);
    assert_eq!(
        exchange(&mut fleet, read_frame, true),
        TransferOutcome::NoAnswer,
        "Table 9 reserves offset 0x01"
    );
    let mut bytes = Vec::new();
    while let TransferOutcome::Answer(v) = exchange(&mut fleet, read_frame, true) {
        bytes.push(v);
    }
    assert_eq!(
        bytes.len(),
        usize::from(BANK0_BASE_LAST_OFFSET) - 1,
        "reads stop past the last location"
    );
    assert_eq!(bytes[0], BANK0_LAST_BANK);
    let gtin = bytes[1..7]
        .iter()
        .fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
    assert_eq!(gtin & 0x00FF_FFFF, u64::from(demo_random_address(0)));
    assert_eq!(bytes[7], 1, "firmware major");
}

#[test]
fn metering_banks_answer_only_on_gear_that_declared_the_types() {
    use dali2rust_domain::dali::banks::{
        bank_last_offset, read_value, BankValue, ValueWidth, BANK_ACTIVE_ENERGY,
        BANK_CONTROL_GEAR_DIAGNOSTICS, BANK_LOADSIDE_ENERGY, DEVICE_TYPE_DIAGNOSTICS,
        DEVICE_TYPE_ENERGY,
    };
    let mut fleet = GearFleet::demo_bus();
    let metered = 8u8;
    let plain = 0u8;

    let declared = fleet
        .gears()
        .iter()
        .find(|g| g.spec.short_address == Some(metered))
        .expect("the metering fixture")
        .spec
        .device_types
        .clone();
    assert!(declared.contains(&DEVICE_TYPE_ENERGY));
    assert!(declared.contains(&DEVICE_TYPE_DIAGNOSTICS));

    let read_from = |fleet: &mut GearFleet, addr: u8, bank: u8, offset: u8, len: usize| {
        exchange(fleet, special(SpecialCommand::Dtr1(bank)), false);
        exchange(fleet, special(SpecialCommand::Dtr0(offset)), false);
        let frame = (u16::from(short(addr).encode_address_byte() | 0x01) << 8)
            | u16::from(READ_MEMORY_LOCATION_OPCODE);
        let mut bytes = Vec::new();
        for _ in 0..len {
            match exchange(fleet, frame, true) {
                TransferOutcome::Answer(v) => bytes.push(v),
                _ => break,
            }
        }
        bytes
    };

    let meta = read_from(&mut fleet, metered, BANK_ACTIVE_ENERGY, 0x00, 1);
    assert_eq!(meta.first().copied(), bank_last_offset(BANK_ACTIVE_ENERGY));
    let gear_meta = read_from(&mut fleet, metered, BANK_CONTROL_GEAR_DIAGNOSTICS, 0x00, 1);
    assert_eq!(
        gear_meta.first().copied(),
        bank_last_offset(BANK_CONTROL_GEAR_DIAGNOSTICS)
    );

    let energy = read_from(&mut fleet, metered, BANK_ACTIVE_ENERGY, 0x05, 6);
    assert_eq!(energy.len(), 6);
    assert!(matches!(
        read_value(&energy, 0, ValueWidth::U48),
        Some(BankValue::Reading(_))
    ));

    let loadside_power = read_from(&mut fleet, metered, BANK_LOADSIDE_ENERGY, 0x0C, 4);
    assert_eq!(
        read_value(&loadside_power, 0, ValueWidth::U32),
        Some(BankValue::TemporarilyUnavailable)
    );
    let loadside_energy = read_from(&mut fleet, metered, BANK_LOADSIDE_ENERGY, 0x05, 6);
    assert_eq!(
        read_value(&loadside_energy, 0, ValueWidth::U48),
        Some(BankValue::Reading(0)),
        "a real zero is a value, not a sentinel"
    );

    assert!(read_from(&mut fleet, plain, BANK_ACTIVE_ENERGY, 0x00, 1).is_empty());
    assert!(read_from(&mut fleet, plain, BANK_CONTROL_GEAR_DIAGNOSTICS, 0x00, 1).is_empty());
}

#[test]
fn the_last_accessible_bank_byte_follows_the_metering_banks() {
    let mut fleet = GearFleet::demo_bus();
    let read_last_bank = |fleet: &mut GearFleet, addr: u8| {
        exchange(fleet, special(SpecialCommand::Dtr1(MEMORY_BANK_IDENTITY)), false);
        exchange(fleet, special(SpecialCommand::Dtr0(2)), false);
        let frame = (u16::from(short(addr).encode_address_byte() | 0x01) << 8)
            | u16::from(READ_MEMORY_LOCATION_OPCODE);
        exchange(fleet, frame, true)
    };
    assert_eq!(read_last_bank(&mut fleet, 0), TransferOutcome::Answer(BANK0_LAST_BANK));
    assert_eq!(read_last_bank(&mut fleet, 8), TransferOutcome::Answer(207));
}

#[test]
fn memory_bank_profile_lock_byte_and_unknown_bank() {
    let mut fleet = GearFleet::demo_bus();
    let read_frame = (u16::from(short(0).encode_address_byte() | 0x01) << 8)
        | u16::from(READ_MEMORY_LOCATION_OPCODE);
    exchange(
        &mut fleet,
        special(SpecialCommand::Dtr1(MEMORY_BANK_PROFILE)),
        false,
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr0(2)), false);
    assert_eq!(
        exchange(&mut fleet, read_frame, true),
        TransferOutcome::Answer(BANK1_UNLOCKED)
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr1(2)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0)), false);
    assert_eq!(
        exchange(&mut fleet, read_frame, true),
        TransferOutcome::NoAnswer
    );
}

fn commission_one(fleet: &mut GearFleet, scope: u8, program_to: u8) -> u32 {
    config(fleet, special(SpecialCommand::Initialise(scope)));
    config(fleet, special(SpecialCommand::Randomise));

    let mut low = 0u32;
    let mut high = 0x00FF_FFFF;
    while low < high {
        let mid = low + (high - low) / 2;
        set_search(fleet, mid);
        match exchange(fleet, special(SpecialCommand::Compare), true) {
            TransferOutcome::Answer(_) => high = mid,
            _ => low = mid + 1,
        }
    }
    set_search(fleet, low);
    let encoded = encode_short_reply(program_to);
    exchange(
        fleet,
        special(SpecialCommand::ProgramShortAddress(encoded)),
        false,
    );
    low
}

fn set_search(fleet: &mut GearFleet, value: u32) {
    exchange(
        fleet,
        special(SpecialCommand::SearchAddrH((value >> 16) as u8)),
        false,
    );
    exchange(
        fleet,
        special(SpecialCommand::SearchAddrM((value >> 8) as u8)),
        false,
    );
    exchange(
        fleet,
        special(SpecialCommand::SearchAddrL(value as u8)),
        false,
    );
}

#[test]
fn commissioning_binary_search_finds_and_programs_unaddressed_gear() {
    let mut fleet = GearFleet::demo_bus();
    let found = commission_one(&mut fleet, 0xFF, 6);

    let encoded = encode_short_reply(6);
    let verified = exchange(
        &mut fleet,
        special(SpecialCommand::VerifyShortAddress(encoded)),
        true,
    );
    assert_eq!(verified, TransferOutcome::Answer(DALI_YES));
    exchange(&mut fleet, special(SpecialCommand::Withdraw), false);
    exchange(&mut fleet, special(SpecialCommand::Terminate), false);

    let present = exchange(
        &mut fleet,
        standard(6, StandardCommand::QueryControlGearPresent),
        true,
    );
    assert_eq!(present, TransferOutcome::Answer(DALI_YES));

    let gear = fleet
        .gears()
        .iter()
        .find(|g| g.spec.short_address == Some(6))
        .expect("programmed gear");
    assert_eq!(gear.spec.random_address, found);
}

#[test]
fn randomise_redraws_the_address_instead_of_replaying_the_seed() {
    let mut fleet = GearFleet::demo_bus();
    let before: Vec<u32> = fleet.gears().iter().map(|g| g.spec.random_address).collect();
    config(&mut fleet, special(SpecialCommand::Initialise(0x00)));
    config(&mut fleet, special(SpecialCommand::Randomise));
    let after: Vec<u32> = fleet.gears().iter().map(|g| g.spec.random_address).collect();

    assert_ne!(before, after, "RANDOMISE must redraw");
    assert!(after.iter().all(|a| *a <= 0x00FF_FFFF), "24 bits");
    let mut sorted = after.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), after.len(), "addresses must stay distinct");
}

#[test]
fn randomise_leaves_gear_outside_the_initialise_scope_alone() {
    let mut fleet = GearFleet::demo_bus();
    let untouched = fleet.gears()[0].spec.random_address;
    config(&mut fleet, special(SpecialCommand::Initialise(0xFF)));
    config(&mut fleet, special(SpecialCommand::Randomise));
    assert_eq!(fleet.gears()[0].spec.random_address, untouched);
}

#[test]
fn two_gear_answering_one_query_violate_whatever_they_meant_to_say() {
    let mut fleet = GearFleet::demo_bus();
    for addr in [0u8, 1] {
        config(
            &mut fleet,
            standard(addr, StandardCommand::AddToGroup { group: 2 }),
        );
    }
    let group_frame = frame(DaliCommand::Standard {
        address: DaliAddress::group(2).unwrap(),
        command: StandardCommand::QueryActualLevel,
    });
    assert_eq!(
        exchange(&mut fleet, group_frame, true),
        TransferOutcome::CorruptedInWindow
    );

    exchange(
        &mut fleet,
        standard(0, StandardCommand::DirectArcPower { level: 100 }),
        false,
    );
    assert_eq!(
        exchange(&mut fleet, group_frame, true),
        TransferOutcome::CorruptedInWindow,
        "§8.2.5: overlapping answers are a backward frame with unusable content"
    );
}

#[test]
fn compare_with_many_responders_violates_and_still_means_yes() {
    let mut fleet = GearFleet::demo_bus();
    config(&mut fleet, special(SpecialCommand::Initialise(0x00)));
    set_search(&mut fleet, 0x00FF_FFFF);
    assert_eq!(
        exchange(&mut fleet, special(SpecialCommand::Compare), true),
        TransferOutcome::CorruptedInWindow
    );
}

#[test]
fn reserved_short_address_is_cleared_at_construction() {
    let specs = vec![
        GearSpec::dt6(Some(2), 0x11_1111),
        GearSpec::dt6(Some(19), 0x22_2222),
    ];
    let fleet = GearFleet::new(specs, DEFAULT_RESERVED_SHORT_ADDRESSES, 1);
    assert_eq!(fleet.stats().reserved_conflicts, 1);
    assert_eq!(fleet.gears()[0].spec.short_address, None, "2 is reserved");
    assert_eq!(fleet.gears()[1].spec.short_address, Some(19));
}

#[test]
fn commissioning_cannot_program_a_reserved_short_address() {
    let specs = vec![GearSpec::dt6(None, 0x33_3333)];
    let mut fleet = GearFleet::new(specs, DEFAULT_RESERVED_SHORT_ADDRESSES, 1);
    commission_one(&mut fleet, 0xFF, 1);
    assert_eq!(fleet.stats().refused_programs, 1);
    assert_eq!(
        fleet.gears()[0].spec.short_address,
        None,
        "a real fixture owns short address 1 — the fleet must not claim it"
    );
}

#[test]
fn a_disabled_gear_is_electrically_absent() {
    let mut fleet = GearFleet::demo_bus();
    fleet.gears_mut()[0].enabled = false;
    assert_eq!(
        exchange(
            &mut fleet,
            standard(0, StandardCommand::QueryControlGearPresent),
            true
        ),
        TransferOutcome::NoAnswer
    );
    let broadcast = frame(DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::RecallMaxLevel,
    });
    exchange(&mut fleet, broadcast, false);
    assert_eq!(fleet.gears()[0].level, 0);
    assert!(fleet.gears()[1].level > 0, "enabled gear still responds");
}

#[test]
fn injected_drop_withholds_the_answer_without_faking_one() {
    let mut fleet = GearFleet::demo_bus();
    fleet.gears_mut()[0].faults.drop_answer_permille = 1000;
    assert_eq!(
        exchange(
            &mut fleet,
            standard(0, StandardCommand::QueryControlGearPresent),
            true
        ),
        TransferOutcome::NoAnswer
    );
    assert_eq!(fleet.stats().dropped_answers, 1);
}

#[test]
fn forced_status_bits_reach_the_status_byte() {
    let mut fleet = GearFleet::demo_bus();
    fleet.gears_mut()[0].faults.forced_status = 0x08;
    let TransferOutcome::Answer(status) =
        exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true)
    else {
        panic!("status must answer");
    };
    assert_eq!(status & 0x08, 0x08);
}

#[test]
fn power_cycle_bit_clears_on_the_first_arc_power_command() {
    let mut fleet = GearFleet::demo_bus();
    let TransferOutcome::Answer(fresh) =
        exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true)
    else {
        panic!("status must answer");
    };
    assert_eq!(fresh & 0x80, 0x80, "power cycle seen after boot");

    exchange(
        &mut fleet,
        standard(0, StandardCommand::DirectArcPower { level: 50 }),
        false,
    );
    let TransferOutcome::Answer(after) =
        exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true)
    else {
        panic!("status must answer");
    };
    assert_eq!(after & 0x80, 0, "cleared once the gear has driven the lamp");
    assert_eq!(after & STATUS_LAMP_ON, STATUS_LAMP_ON);
}

#[test]
fn the_version_query_answers_the_same_encoded_byte_as_memory_bank_0() {
    let mut fleet = GearFleet::demo_bus();
    let TransferOutcome::Answer(queried) =
        exchange(&mut fleet, standard(0, StandardCommand::QueryVersionNumber), true)
    else {
        panic!("version must answer");
    };
    assert_eq!(queried, 0x08, "IEC 62386-102 §4.2: version 2.0");
    assert_eq!(
        fleet.gears()[0].memory_location(MEMORY_BANK_IDENTITY, 0x16),
        Some(queried),
        "§11.5.10 makes bank 0 location 0x16 the same value"
    );
}

#[test]
fn lamp_on_is_false_during_startup_and_on_lamp_failure() {
    let mut fleet = GearFleet::demo_bus();
    let drive = |fleet: &mut GearFleet| {
        exchange(
            fleet,
            standard(0, StandardCommand::DirectArcPower { level: 200 }),
            false,
        );
        let TransferOutcome::Answer(status) =
            exchange(fleet, standard(0, StandardCommand::QueryStatus), true)
        else {
            panic!("status must answer");
        };
        status
    };

    assert_eq!(drive(&mut fleet) & STATUS_LAMP_ON, STATUS_LAMP_ON, "lit");

    fleet.gears_mut()[0].lamp_starting = true;
    let starting = drive(&mut fleet);
    assert_eq!(starting & STATUS_LAMP_ON, 0, "driven, not yet producing light");
    fleet.gears_mut()[0].lamp_starting = false;

    fleet.gears_mut()[0].lamp_failure = true;
    let failed = drive(&mut fleet);
    assert_eq!(failed & STATUS_LAMP_ON, 0, "total lamp failure");
    assert_eq!(
        failed & STATUS_LAMP_FAILURE,
        STATUS_LAMP_FAILURE,
        "the pair the old model could emit: lampFailure with lampOn beside it"
    );
}

#[test]
fn reset_state_bit_clears_on_the_first_config_write_and_returns_on_reset() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    config(&mut fleet, standard(0, StandardCommand::SetMaxLevel));
    let TransferOutcome::Answer(configured) =
        exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true)
    else {
        panic!("status must answer");
    };
    assert_eq!(configured & 0x20, 0, "no longer in reset state");

    config(&mut fleet, standard(0, StandardCommand::Reset));
    let TransferOutcome::Answer(after_reset) =
        exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true)
    else {
        panic!("status must answer");
    };
    assert_eq!(after_reset & 0x20, 0x20, "reset restores the bit");
}

#[test]
fn reset_keeps_the_bench_switches_it_does_not_own() {
    let mut fleet = GearFleet::demo_bus();
    fleet.gears_mut()[0].faults.drop_answer_permille = 250;
    config(&mut fleet, standard(0, StandardCommand::Reset));
    assert_eq!(
        fleet.gears()[0].faults.drop_answer_permille,
        250,
        "an injected fault is a bench setting, not gear state"
    );
    assert!(fleet.gears()[0].enabled);
}

#[test]
fn a_dapc_frame_commits_the_staged_colour() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0xC8)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x00)), false);
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(5, Dt8Command::SetTemporaryColourTemperature),
        false,
    );

    enable_dt8(&mut fleet);
    assert_eq!(
        exchange(&mut fleet, dt8(5, Dt8Command::QueryColourStatus), true),
        TransferOutcome::Answer(0),
        "staged only — nothing active yet (ISSUE-12)"
    );

    let dapc = (u16::from(short(5).encode_address_byte()) << 8) | 120;
    exchange(&mut fleet, dapc, false);

    enable_dt8(&mut fleet);
    assert_eq!(
        exchange(&mut fleet, dt8(5, Dt8Command::QueryColourStatus), true),
        TransferOutcome::Answer(DT8_STATUS_TC_ACTIVE),
        "the arc-power change commits it"
    );
    let gear = &fleet.gears()[5];
    assert_eq!(gear.color_ct, 200, "5000 K in mirek");
    assert_eq!(gear.level, 120);
}

#[test]
fn a_masked_dapc_activates_the_staged_colour_only_while_the_bit_is_set_issue128() {
    let mut fleet = GearFleet::demo_bus();
    write_gear_features(&mut fleet, 0x00);
    stage_cct(&mut fleet);
    let level = fleet.gears()[5].level;
    let masked = (u16::from(short(5).encode_address_byte()) << 8) | 0xFF;

    exchange(&mut fleet, masked, false);
    assert_eq!(
        colour_status(&mut fleet),
        TransferOutcome::Answer(0),
        "Table 5: MASK with the bit clear changes no colour"
    );
    assert_eq!(fleet.gears()[5].pending_ct, 200, "Table 5: no change in temporaries");

    write_gear_features(&mut fleet, 0x01);
    exchange(&mut fleet, masked, false);
    assert_eq!(
        colour_status(&mut fleet),
        TransferOutcome::Answer(DT8_STATUS_TC_ACTIVE),
        "Table 5: an arc-power command, MASK included, activates while the bit is set"
    );
    assert_eq!(fleet.gears()[5].color_ct, 200);
    assert_eq!(fleet.gears()[5].level, level, "MASK never moves the level");
}

fn stage_cct(fleet: &mut GearFleet) {
    exchange(fleet, special(SpecialCommand::Dtr0(0xC8)), false);
    exchange(fleet, special(SpecialCommand::Dtr1(0x00)), false);
    enable_dt8(fleet);
    exchange(
        fleet,
        dt8(5, Dt8Command::SetTemporaryColourTemperature),
        false,
    );
}

fn write_gear_features(fleet: &mut GearFleet, operand: u8) {
    exchange(fleet, special(SpecialCommand::Dtr0(operand)), false);
    enable_dt8(fleet);
    config(fleet, dt8(5, Dt8Command::StoreGearFeaturesStatus));
}

fn read_gear_features(fleet: &mut GearFleet) -> TransferOutcome {
    enable_dt8(fleet);
    exchange(fleet, dt8(5, Dt8Command::QueryGearFeaturesStatus), true)
}

fn colour_status(fleet: &mut GearFleet) -> TransferOutcome {
    enable_dt8(fleet);
    exchange(fleet, dt8(5, Dt8Command::QueryColourStatus), true)
}

#[test]
fn automatic_activation_is_set_at_power_up_and_restored_by_reset() {
    let mut fleet = GearFleet::demo_bus();
    assert_eq!(read_gear_features(&mut fleet), TransferOutcome::Answer(0x01));

    write_gear_features(&mut fleet, 0x00);
    assert_eq!(read_gear_features(&mut fleet), TransferOutcome::Answer(0x00));

    config(&mut fleet, standard(5, StandardCommand::Reset));
    assert_eq!(
        read_gear_features(&mut fleet),
        TransferOutcome::Answer(0x01),
        "RESET restores the default of a volatile byte"
    );
}

#[test]
fn a_dapc_does_not_commit_colour_while_automatic_activation_is_clear() {
    let mut fleet = GearFleet::demo_bus();
    write_gear_features(&mut fleet, 0x00);
    stage_cct(&mut fleet);

    let dapc = (u16::from(short(5).encode_address_byte()) << 8) | 120;
    exchange(&mut fleet, dapc, false);

    assert_eq!(
        colour_status(&mut fleet),
        TransferOutcome::Answer(0),
        "Table 5: no colour change while the bit is clear"
    );
    let gear = &fleet.gears()[5];
    assert_eq!(gear.level, 120, "the level still moves");
    assert_eq!(
        gear.pending_ct, 200,
        "Table 5: no change in temporaries — the staged value survives the DAPC"
    );

    write_gear_features(&mut fleet, 0x01);
    exchange(&mut fleet, dapc, false);
    assert_eq!(
        colour_status(&mut fleet),
        TransferOutcome::Answer(DT8_STATUS_TC_ACTIVE),
        "with the bit set the same DAPC commits what was still staged"
    );
    assert_eq!(fleet.gears()[5].color_ct, 200);
}

#[test]
fn an_explicit_activate_ignores_the_automatic_activation_bit() {
    let mut fleet = GearFleet::demo_bus();
    write_gear_features(&mut fleet, 0x00);
    stage_cct(&mut fleet);

    enable_dt8(&mut fleet);
    exchange(&mut fleet, dt8(5, Dt8Command::Activate), false);

    assert_eq!(
        colour_status(&mut fleet),
        TransferOutcome::Answer(DT8_STATUS_TC_ACTIVE)
    );
    assert_eq!(fleet.gears()[5].color_ct, 200);
}

#[test]
fn a_single_store_gear_features_does_not_change_the_byte() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x00)), false);
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(5, Dt8Command::StoreGearFeaturesStatus),
        false,
    );

    assert_eq!(
        read_gear_features(&mut fleet),
        TransferOutcome::Answer(0x01),
        "an unpaired configuration command is discarded"
    );
    assert_eq!(fleet.stats().send_twice_pairs_executed, 0);
}

#[test]
fn echoing_the_queried_byte_into_the_store_operand_is_counted() {
    let mut fleet = GearFleet::demo_bus();
    write_gear_features(&mut fleet, 0x41);

    assert_eq!(
        read_gear_features(&mut fleet),
        TransferOutcome::Answer(0x01),
        "only bit 0 is writable; the reserved bits are clamped away"
    );
    assert_eq!(fleet.stats().dt8_gear_features_reserved_bits, 1);

    write_gear_features(&mut fleet, 0x01);
    assert_eq!(
        fleet.stats().dt8_gear_features_reserved_bits,
        1,
        "a legal operand must not move the counter"
    );
}

#[test]
fn overlapping_answers_keep_the_bytes_so_a_driver_can_merge_them_on_the_wire() {
    let mut fleet = GearFleet::demo_bus();
    for addr in [0u8, 1] {
        config(
            &mut fleet,
            standard(addr, StandardCommand::AddToGroup { group: 2 }),
        );
    }
    exchange(
        &mut fleet,
        standard(0, StandardCommand::DirectArcPower { level: 100 }),
        false,
    );
    let group_frame = frame(DaliCommand::Standard {
        address: DaliAddress::group(2).unwrap(),
        command: StandardCommand::QueryActualLevel,
    });
    assert_eq!(
        exchange(&mut fleet, group_frame, true),
        TransferOutcome::CorruptedInWindow
    );
    let mut answers = fleet.last_answers().to_vec();
    answers.sort_unstable();
    assert_eq!(answers, vec![0, 100]);

    exchange(
        &mut fleet,
        standard(0, StandardCommand::RecallMaxLevel),
        false,
    );
    assert!(fleet.last_answers().is_empty());
}

#[test]
fn dtr_content_queries_stay_silent_for_addresses_the_fleet_does_not_own() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x55)), false);

    let own = exchange(&mut fleet, standard(4, StandardCommand::QueryContentDtr0), true);
    assert_eq!(own, TransferOutcome::Answer(0x55), "an addressed gear answers");

    let foreign = exchange(&mut fleet, standard(60, StandardCommand::QueryContentDtr0), true);
    assert_eq!(
        foreign,
        TransferOutcome::NoAnswer,
        "an address the fleet does not own is another master's readback — jamming it is the bug"
    );

    let idx = fleet
        .gears_mut()
        .iter()
        .position(|g| g.spec.short_address == Some(4))
        .expect("demo bus has short 4");
    fleet.gears_mut()[idx].enabled = false;
    let disabled = exchange(&mut fleet, standard(4, StandardCommand::QueryContentDtr0), true);
    assert_eq!(disabled, TransferOutcome::NoAnswer, "disable must silence DTR answers too");
}

#[test]
fn expects_backward_agrees_with_what_the_fleet_actually_answers() {
    let queries = [
        standard(0, StandardCommand::QueryStatus),
        standard(0, StandardCommand::QueryActualLevel),
        standard(0, StandardCommand::QueryControlGearPresent),
        standard(0, StandardCommand::QueryGroups0To7),
        standard(0, StandardCommand::QuerySceneLevel { scene: 3 }),
        standard(0, StandardCommand::QueryDeviceType),
        standard(0, StandardCommand::QueryRandomAddressH),
        standard(0, StandardCommand::QueryContentDtr0),
        special(SpecialCommand::Compare),
        special(SpecialCommand::VerifyShortAddress(encode_short_reply(0))),
        special(SpecialCommand::QueryShortAddress),
    ];
    let silent = [
        standard(0, StandardCommand::DirectArcPower { level: 10 }),
        standard(0, StandardCommand::Off),
        standard(0, StandardCommand::RecallMaxLevel),
        standard(0, StandardCommand::AddToGroup { group: 1 }),
        standard(0, StandardCommand::SetScene { scene: 1 }),
        standard(0, StandardCommand::Reset),
        special(SpecialCommand::Dtr0(1)),
        special(SpecialCommand::Initialise(0x00)),
        special(SpecialCommand::Randomise),
        special(SpecialCommand::Withdraw),
        special(SpecialCommand::Terminate),
        special(SpecialCommand::EnableDeviceType(8)),
    ];

    for frame in queries {
        let mut fleet = GearFleet::demo_bus();
        config(&mut fleet, special(SpecialCommand::Initialise(0x00)));
        let target = fleet.gears()[0].spec.random_address;
        set_search(&mut fleet, target);
        assert!(
            fleet.expects_backward(frame),
            "frame {frame:#06x} answers but was not predicted to"
        );
        let outcome = exchange(&mut fleet, frame, true);
        assert_ne!(
            outcome,
            TransferOutcome::NoAnswer,
            "frame {frame:#06x} was predicted to answer"
        );
    }

    for frame in silent {
        let fleet = GearFleet::demo_bus();
        assert!(
            !fleet.expects_backward(frame),
            "frame {frame:#06x} does not answer but was predicted to"
        );
    }
}

#[test]
fn expects_backward_covers_the_two_opcodes_the_codec_does_not_model() {
    let mut fleet = GearFleet::demo_bus();
    let read_memory = (u16::from(short(0).encode_address_byte() | 0x01) << 8)
        | u16::from(READ_MEMORY_LOCATION_OPCODE);
    assert!(fleet.expects_backward(read_memory), "READ MEMORY LOCATION");

    let colour_status = dt8(4, Dt8Command::QueryColourStatus);
    assert!(
        !fleet.expects_backward(colour_status),
        "a DT8 query means nothing until the device-type latch is armed"
    );
    enable_dt8(&mut fleet);
    assert!(fleet.expects_backward(colour_status), "latch armed");
    assert_ne!(
        exchange(&mut fleet, colour_status, true),
        TransferOutcome::NoAnswer
    );
}

#[test]
fn bench_fleet_lays_gear_out_from_the_base_address_upward() {
    let specs = crate::fleet::bench_fleet(10, 2, 2, 1, 42);
    assert_eq!(specs.len(), 5);
    let addrs: Vec<Option<u8>> = specs.iter().map(|s| s.short_address).collect();
    assert_eq!(addrs, vec![Some(10), Some(11), Some(12), Some(13), Some(14)]);
    assert!(!specs[0].tc_capable, "first block is DT6");
    assert!(specs[2].tc_capable && !specs[2].rgb_capable(), "then CCT");
    assert!(specs[4].rgb_capable() && specs[4].tc_capable, "then RGB+CCT");
    let mut randoms: Vec<u32> = specs.iter().map(|s| s.random_address).collect();
    randoms.sort_unstable();
    randoms.dedup();
    assert_eq!(randoms.len(), specs.len());
}

#[test]
fn a_sixty_gear_fleet_avoids_the_reserved_block_entirely() {
    let specs = crate::fleet::bench_fleet(10, 27, 18, 9, 7);
    let fleet = GearFleet::new(specs, DEFAULT_RESERVED_SHORT_ADDRESSES, 7);
    assert_eq!(fleet.gears().len(), 54);
    assert_eq!(
        fleet.stats().reserved_conflicts,
        0,
        "the default layout never asks for 0..=9"
    );
    let highest = fleet
        .gears()
        .iter()
        .filter_map(|g| g.spec.short_address)
        .max();
    assert_eq!(highest, Some(63), "54 gear from 10 fill the address space");
}

#[test]
fn a_fleet_larger_than_the_address_space_leaves_the_excess_unaddressed() {
    let specs = crate::fleet::bench_fleet(10, 40, 20, 10, 7);
    assert_eq!(specs.len(), 70);
    let addressed: Vec<u8> = specs.iter().filter_map(|s| s.short_address).collect();
    assert_eq!(addressed.len(), 54, "only 10..=63 are addressable");
    assert!(
        !specs
            .iter()
            .any(|s| matches!(s.short_address, Some(a) if !(10..=63).contains(&a))),
        "every addressed gear sits in the free block: {addressed:?}"
    );
}

#[test]
fn an_interleaved_frame_spends_the_enable_device_type_prelude() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x50)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x01)), false);
    enable_dt8(&mut fleet);
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x07)), false);
    exchange(
        &mut fleet,
        dt8(4, Dt8Command::SetTemporaryColourTemperature),
        false,
    );

    assert_eq!(
        fleet.stats().enable_device_type_consumed_by_interloper,
        1,
        "the extended command must be discarded, and the gear is the only one who knows"
    );

    exchange(&mut fleet, standard(4, StandardCommand::Off), false);
    enable_dt8(&mut fleet);
    let status = exchange(&mut fleet, dt8(4, Dt8Command::QueryColourStatus), true);
    assert_eq!(status, TransferOutcome::Answer(0), "nothing was staged");
}

#[test]
fn a_prelude_for_another_device_type_is_not_a_spent_one() {
    let mut fleet = GearFleet::demo_bus();
    exchange(
        &mut fleet,
        special(SpecialCommand::EnableDeviceType(DeviceType::Led.code())),
        false,
    );
    exchange(&mut fleet, dt8(4, Dt8Command::QueryColourStatus), true);

    assert_eq!(
        fleet.stats().enable_device_type_consumed_by_interloper,
        0,
        "a live prelude naming another device type is not a lost prelude"
    );
}

#[test]
fn an_adjacent_prelude_still_arms_the_extended_command() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x50)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x01)), false);
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(4, Dt8Command::SetTemporaryColourTemperature),
        false,
    );
    assert_eq!(fleet.stats().enable_device_type_consumed_by_interloper, 0);
}

#[test]
fn a_foreign_dtr_write_between_staging_and_use_produces_the_foreign_value() {
    let mut fleet = GearFleet::demo_bus();
    let ours = 0x64;
    let theirs = 0x07;

    exchange(&mut fleet, special(SpecialCommand::Dtr0(ours)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr0(theirs)), false);
    config(&mut fleet, standard(4, StandardCommand::SetFadeTime));

    let readback = exchange(
        &mut fleet,
        standard(4, StandardCommand::QueryFadeTimeFadeRate),
        true,
    );
    let TransferOutcome::Answer(byte) = readback else {
        panic!("the gear must answer its fade registers, got {readback:?}");
    };
    assert_eq!(
        byte >> 4,
        theirs & 0x0F,
        "the gear stored the interloper's operand, and nothing anywhere says so"
    );
}

#[test]
fn a_configuration_command_sent_once_does_not_execute() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    exchange(&mut fleet, standard(0, StandardCommand::SetMaxLevel), false);
    assert_eq!(
        query_max_level(&mut fleet, 0),
        254,
        "a single instance must not be executed"
    );
    assert_eq!(fleet.stats().send_twice_pairs_executed, 0);

    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    config(&mut fleet, standard(0, StandardCommand::SetMaxLevel));
    assert_eq!(query_max_level(&mut fleet, 0), 200, "the pair executes");
    assert_eq!(fleet.stats().send_twice_pairs_executed, 1);
}

#[test]
fn a_frame_addressed_to_another_gear_does_not_break_the_pair() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    exchange(&mut fleet, standard(0, StandardCommand::SetMaxLevel), false);
    exchange(&mut fleet, standard(1, StandardCommand::QueryStatus), true);
    exchange(&mut fleet, standard(0, StandardCommand::SetMaxLevel), false);

    assert_eq!(query_max_level(&mut fleet, 0), 200, "the pair still executed");
    assert_eq!(
        fleet.stats().send_twice_split_by_interloper,
        0,
        "somebody else's traffic is not our violation"
    );
}

#[test]
fn a_frame_addressed_to_the_same_gear_breaks_the_pair() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    exchange(&mut fleet, standard(0, StandardCommand::SetMaxLevel), false);
    exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true);
    exchange(&mut fleet, standard(0, StandardCommand::SetMaxLevel), false);

    assert_eq!(fleet.stats().send_twice_split_by_interloper, 1);
    assert_eq!(query_max_level(&mut fleet, 0), 254, "the first half is ignored");
}

#[test]
fn an_unaddressed_dtr_write_between_the_halves_voids_the_pair() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(90)), false);
    let set_scene = standard(2, StandardCommand::SetScene { scene: 5 });
    exchange(&mut fleet, set_scene, false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x11)), false);
    exchange(&mut fleet, set_scene, false);

    assert_eq!(fleet.stats().send_twice_split_by_interloper, 1);
    let stored = exchange(
        &mut fleet,
        standard(2, StandardCommand::QuerySceneLevel { scene: 5 }),
        true,
    );
    assert_eq!(
        stored,
        TransferOutcome::Answer(0xFF),
        "the scene slot must be untouched"
    );
}

#[test]
fn a_broadcast_pair_executes_on_every_gear_but_the_interrupted_one() {
    let mut fleet = GearFleet::demo_bus();
    let broadcast = frame(DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::SetMaxLevel,
    });
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    exchange(&mut fleet, broadcast, false);
    exchange(&mut fleet, standard(3, StandardCommand::QueryStatus), true);
    exchange(&mut fleet, broadcast, false);

    assert_eq!(
        fleet.stats().send_twice_split_by_interloper,
        1,
        "exactly one gear lost its pair"
    );
    assert_eq!(query_max_level(&mut fleet, 0), 200);
    assert_eq!(query_max_level(&mut fleet, 3), 254, "gear 3 was interrupted");
}

#[test]
fn an_extended_pair_keeps_its_prelude_across_the_held_half() {
    let mut fleet = GearFleet::demo_bus();
    exchange(
        &mut fleet,
        special(SpecialCommand::EnableDeviceType(DeviceType::Led.code())),
        false,
    );
    let reference_system_power =
        (u16::from(short(0).encode_address_byte() | 0x01) << 8) | u16::from(0xE0u8);
    exchange(&mut fleet, reference_system_power, false);
    exchange(&mut fleet, reference_system_power, false);

    assert_eq!(
        fleet.stats().enable_device_type_consumed_by_interloper,
        0,
        "one prelude legitimately covers both halves"
    );
    assert_eq!(fleet.stats().send_twice_split_by_interloper, 0);
}

#[test]
fn a_dt8_control_command_still_executes_on_first_receipt() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x34)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x12)), false);
    enable_dt8(&mut fleet);
    exchange(&mut fleet, dt8(4, Dt8Command::SetTemporaryXCoordinate), false);

    assert_eq!(
        fleet.gears()[4].pending_x,
        0x1234,
        "a DT8 control command executes on the first frame"
    );
    assert_eq!(fleet.stats().send_twice_split_by_interloper, 0);
    assert_eq!(fleet.stats().send_twice_pairs_executed, 0);
}

#[test]
fn initialise_executes_only_on_its_second_instance() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Initialise(0x00)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x01)), false);
    exchange(&mut fleet, special(SpecialCommand::Initialise(0x00)), false);
    assert!(
        fleet.gears().iter().all(|g| !g.initialise),
        "a split INITIALISE selects nobody"
    );

    config(&mut fleet, special(SpecialCommand::Initialise(0x00)));
    assert!(fleet.gears().iter().all(|g| g.initialise));
}

#[test]
fn a_repeated_arc_power_frame_is_not_a_pair() {
    let mut fleet = GearFleet::demo_bus();
    let dapc = standard(0, StandardCommand::DirectArcPower { level: 120 });
    exchange(&mut fleet, dapc, false);
    exchange(&mut fleet, dapc, false);

    assert_eq!(
        exchange(&mut fleet, standard(0, StandardCommand::QueryActualLevel), true),
        TransferOutcome::Answer(120)
    );
    assert_eq!(fleet.stats().send_twice_pairs_executed, 0);
    assert_eq!(fleet.stats().send_twice_split_by_interloper, 0);
}

#[test]
fn a_held_first_half_is_not_a_violation_until_something_else_arrives() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    exchange(&mut fleet, standard(0, StandardCommand::SetMaxLevel), false);
    assert_eq!(fleet.stats().send_twice_split_by_interloper, 0, "still held");

    exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true);
    assert_eq!(fleet.stats().send_twice_split_by_interloper, 1);
}

#[test]
fn a_disabled_gear_cannot_split_a_pair_it_never_heard() {
    let mut fleet = GearFleet::demo_bus();
    fleet.gears_mut()[0].enabled = false;
    exchange(&mut fleet, special(SpecialCommand::Dtr0(200)), false);
    exchange(&mut fleet, standard(0, StandardCommand::SetMaxLevel), false);
    exchange(&mut fleet, standard(0, StandardCommand::SetMaxLevel), false);

    assert_eq!(fleet.stats().send_twice_split_by_interloper, 0);
    assert_eq!(fleet.stats().send_twice_pairs_executed, 0);
}

fn query_max_level(fleet: &mut GearFleet, addr: u8) -> u8 {
    let outcome = exchange(fleet, standard(addr, StandardCommand::QueryMaxLevel), true);
    let TransferOutcome::Answer(level) = outcome else {
        panic!("QUERY MAX LEVEL must answer, got {outcome:?}");
    };
    level
}

const RGBWAF_GEAR: u8 = 5;
const TC_ONLY_GEAR: u8 = 4;

fn read_rgbwaf_control(fleet: &mut GearFleet, addr: u8) -> TransferOutcome {
    enable_dt8(fleet);
    exchange(fleet, dt8(addr, Dt8Command::QueryRgbwafControl), true)
}

fn stage_rgbwaf_control(fleet: &mut GearFleet, operand: u8) {
    exchange(fleet, special(SpecialCommand::Dtr0(operand)), false);
    enable_dt8(fleet);
    exchange(
        fleet,
        dt8(RGBWAF_GEAR, Dt8Command::SetTemporaryRgbwafControl),
        false,
    );
}

fn stage_rgb(fleet: &mut GearFleet, r: u8, g: u8, b: u8) {
    exchange(fleet, special(SpecialCommand::Dtr0(r)), false);
    exchange(fleet, special(SpecialCommand::Dtr1(g)), false);
    exchange(fleet, special(SpecialCommand::Dtr2(b)), false);
    enable_dt8(fleet);
    exchange(
        fleet,
        dt8(RGBWAF_GEAR, Dt8Command::SetTemporaryRgbDimLevel),
        false,
    );
}

fn dapc(fleet: &mut GearFleet, addr: u8, level: u8) {
    let frame = (u16::from(short(addr).encode_address_byte()) << 8) | u16::from(level);
    exchange(fleet, frame, false);
}

fn colour_value(fleet: &mut GearFleet, addr: u8, selector: u8) -> TransferOutcome {
    exchange(fleet, special(SpecialCommand::Dtr0(selector)), false);
    enable_dt8(fleet);
    exchange(fleet, dt8(addr, Dt8Command::QueryColourValue), true)
}

#[test]
fn a_tc_activation_unlinks_the_channels_underneath_it() {
    let mut fleet = GearFleet::demo_bus();
    assert_eq!(
        read_rgbwaf_control(&mut fleet, RGBWAF_GEAR),
        TransferOutcome::Answer(0x3F),
        "power-up default: channel control, all six linked"
    );

    stage_cct(&mut fleet);
    dapc(&mut fleet, RGBWAF_GEAR, 120);

    assert_eq!(
        read_rgbwaf_control(&mut fleet, RGBWAF_GEAR),
        TransferOutcome::Answer(0x00),
        "channels unlinked by an activation nobody aimed at this byte"
    );
}

#[test]
fn the_asserted_control_byte_survives_a_later_tc_write() {
    let mut fleet = GearFleet::demo_bus();

    stage_rgbwaf_control(&mut fleet, 0x80);
    stage_rgb(&mut fleet, 254, 0, 0);
    dapc(&mut fleet, RGBWAF_GEAR, 200);
    assert_eq!(
        read_rgbwaf_control(&mut fleet, RGBWAF_GEAR),
        TransferOutcome::Answer(0x80),
        "promoted by the arc-power command, not by the staging"
    );

    stage_cct(&mut fleet);
    dapc(&mut fleet, RGBWAF_GEAR, 200);
    assert_eq!(
        read_rgbwaf_control(&mut fleet, RGBWAF_GEAR),
        TransferOutcome::Answer(0x80),
        "the unlink rule has nothing left to clear, and the control type stands"
    );
}

#[test]
fn a_staged_control_byte_is_not_the_live_one() {
    let mut fleet = GearFleet::demo_bus();
    stage_rgbwaf_control(&mut fleet, 0x80);

    assert_eq!(
        colour_value(&mut fleet, RGBWAF_GEAR, 207),
        TransferOutcome::Answer(0x80),
        "identifier 207 reads the temporary"
    );
    assert_eq!(
        read_rgbwaf_control(&mut fleet, RGBWAF_GEAR),
        TransferOutcome::Answer(0x3F),
        "251 still answers the live byte — a verify loop here could never converge"
    );
}

#[test]
fn query_actual_level_masks_while_the_default_linkage_stands() {
    let mut fleet = GearFleet::demo_bus();
    stage_rgb(&mut fleet, 254, 0, 0);
    dapc(&mut fleet, RGBWAF_GEAR, 200);

    assert_eq!(
        exchange(
            &mut fleet,
            standard(RGBWAF_GEAR, StandardCommand::QueryActualLevel),
            true
        ),
        TransferOutcome::Answer(0xFF),
        "six linked channels have no single level to report"
    );

    stage_rgbwaf_control(&mut fleet, 0x80);
    stage_rgb(&mut fleet, 254, 0, 0);
    dapc(&mut fleet, RGBWAF_GEAR, 200);
    assert_eq!(
        exchange(
            &mut fleet,
            standard(RGBWAF_GEAR, StandardCommand::QueryActualLevel),
            true
        ),
        TransferOutcome::Answer(200),
        "normalised colour control answers the arc level again"
    );
}

#[test]
fn a_gear_without_rgbwaf_channels_does_not_answer_251() {
    let mut fleet = GearFleet::demo_bus();
    assert_eq!(
        read_rgbwaf_control(&mut fleet, TC_ONLY_GEAR),
        TransferOutcome::NoAnswer
    );
}

#[test]
fn an_unimplemented_channel_answers_mask() {
    let mut fleet = GearFleet::demo_bus();
    stage_rgb(&mut fleet, 10, 20, 30);
    exchange(&mut fleet, special(SpecialCommand::Dtr0(40)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(50)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr2(60)), false);
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(RGBWAF_GEAR, Dt8Command::SetTemporaryWafDimLevel),
        false,
    );
    dapc(&mut fleet, RGBWAF_GEAR, 200);

    assert_eq!(
        colour_value(&mut fleet, RGBWAF_GEAR, 9),
        TransferOutcome::Answer(10),
        "red is implemented"
    );
    assert_eq!(
        colour_value(&mut fleet, RGBWAF_GEAR, 12),
        TransferOutcome::Answer(0xFF),
        "white is not, whatever 236 carried"
    );
}

fn write_gear_features_at(fleet: &mut GearFleet, addr: u8, operand: u8) {
    exchange(fleet, special(SpecialCommand::Dtr0(operand)), false);
    enable_dt8(fleet);
    config(fleet, dt8(addr, Dt8Command::StoreGearFeaturesStatus));
}

fn stage_cct_at(fleet: &mut GearFleet, addr: u8, mirek: u16) {
    exchange(fleet, special(SpecialCommand::Dtr0(mirek as u8)), false);
    exchange(fleet, special(SpecialCommand::Dtr1((mirek >> 8) as u8)), false);
    enable_dt8(fleet);
    exchange(
        fleet,
        dt8(addr, Dt8Command::SetTemporaryColourTemperature),
        false,
    );
}

fn activate(fleet: &mut GearFleet, addr: u8) {
    enable_dt8(fleet);
    exchange(fleet, dt8(addr, Dt8Command::Activate), false);
}

fn colour_status_of(fleet: &mut GearFleet, addr: u8) -> TransferOutcome {
    enable_dt8(fleet);
    exchange(fleet, dt8(addr, Dt8Command::QueryColourStatus), true)
}

fn colour_value16(fleet: &mut GearFleet, addr: u8, selector: u8) -> Option<u16> {
    assert!(
        colour_value_is_wide(selector),
        "selector {selector} answers whole — read it with `colour_value`"
    );
    let TransferOutcome::Answer(msb) = colour_value(fleet, addr, selector) else {
        return None;
    };
    let TransferOutcome::Answer(lsb) = exchange(
        fleet,
        standard(addr, StandardCommand::QueryContentDtr0),
        true,
    ) else {
        panic!("DTR0 readback after a wide colour value read");
    };
    Some((u16::from(msb) << 8) | u16::from(lsb))
}

fn write_tc_limit(fleet: &mut GearFleet, addr: u8, selector: u8, mirek: u16) {
    exchange(fleet, special(SpecialCommand::Dtr0(mirek as u8)), false);
    exchange(fleet, special(SpecialCommand::Dtr1((mirek >> 8) as u8)), false);
    exchange(fleet, special(SpecialCommand::Dtr2(selector)), false);
    enable_dt8(fleet);
    config(fleet, dt8(addr, Dt8Command::StoreColourTemperatureTcLimit));
}

fn program_tc_scene(fleet: &mut GearFleet, addr: u8, mirek: u16, level: u8) {
    stage_cct_at(fleet, addr, mirek);
    exchange(fleet, special(SpecialCommand::Dtr0(level)), false);
    config(fleet, standard(addr, StandardCommand::SetScene { scene: 5 }));
}

#[test]
fn store_dtr_as_scene_captures_the_staged_colour_and_a_scene_query_reports_it() {
    let mut fleet = GearFleet::demo_bus();
    program_tc_scene(&mut fleet, TC_ONLY_GEAR, 200, 90);

    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE),
        TransferOutcome::Answer(0xFF),
        "Table 6: the temporaries are consumed by the store"
    );

    let level = exchange(
        &mut fleet,
        standard(TC_ONLY_GEAR, StandardCommand::QuerySceneLevel { scene: 5 }),
        true,
    );
    assert_eq!(level, TransferOutcome::Answer(90));
    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_REPORT_COLOUR_TYPE),
        TransferOutcome::Answer(COLOUR_TYPE_BYTE_TC)
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_REPORT_TC),
        Some(200),
        "the report carries the scene's stored colour, not the active one"
    );
}

#[test]
fn a_query_actual_level_between_load_and_read_replaces_the_scene_report() {
    let mut fleet = GearFleet::demo_bus();
    program_tc_scene(&mut fleet, TC_ONLY_GEAR, 200, 90);
    stage_cct_at(&mut fleet, TC_ONLY_GEAR, 300);
    activate(&mut fleet, TC_ONLY_GEAR);

    exchange(
        &mut fleet,
        standard(TC_ONLY_GEAR, StandardCommand::QuerySceneLevel { scene: 5 }),
        true,
    );
    exchange(
        &mut fleet,
        standard(TC_ONLY_GEAR, StandardCommand::QueryActualLevel),
        true,
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_REPORT_TC),
        Some(300),
        "the report now carries the ACTIVE colour while the caller believes it reads the scene"
    );
}

#[test]
fn go_to_scene_recalls_the_stored_colour_with_the_bit_set() {
    let mut fleet = GearFleet::demo_bus();
    program_tc_scene(&mut fleet, TC_ONLY_GEAR, 200, 90);
    stage_cct_at(&mut fleet, TC_ONLY_GEAR, 300);
    activate(&mut fleet, TC_ONLY_GEAR);

    exchange(
        &mut fleet,
        standard(TC_ONLY_GEAR, StandardCommand::GoToScene { scene: 5 }),
        false,
    );
    assert_eq!(fleet.gears()[usize::from(TC_ONLY_GEAR)].level, 90);
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC),
        Some(200),
        "the scene's colour is active again"
    );
    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE),
        TransferOutcome::Answer(0xFF)
    );
}

#[test]
fn go_to_scene_with_the_bit_clear_stages_the_scene_colour_without_moving_it() {
    let mut fleet = GearFleet::demo_bus();
    program_tc_scene(&mut fleet, TC_ONLY_GEAR, 200, 90);
    stage_cct_at(&mut fleet, TC_ONLY_GEAR, 300);
    activate(&mut fleet, TC_ONLY_GEAR);
    write_gear_features_at(&mut fleet, TC_ONLY_GEAR, 0x00);

    exchange(
        &mut fleet,
        standard(TC_ONLY_GEAR, StandardCommand::GoToScene { scene: 5 }),
        false,
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC),
        Some(300),
        "no colour change while the bit is clear"
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TEMPORARY_TC),
        Some(200),
        "the scene's colour sits staged instead"
    );
    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE),
        TransferOutcome::Answer(COLOUR_TYPE_BYTE_TC)
    );
}

#[test]
fn copy_report_to_temporary_restages_a_scene_colour_for_activation() {
    let mut fleet = GearFleet::demo_bus();
    program_tc_scene(&mut fleet, TC_ONLY_GEAR, 200, 90);
    exchange(
        &mut fleet,
        standard(TC_ONLY_GEAR, StandardCommand::QuerySceneLevel { scene: 5 }),
        true,
    );

    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(TC_ONLY_GEAR, Dt8Command::CopyReportToTemporary),
        false,
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TEMPORARY_TC),
        Some(200)
    );

    activate(&mut fleet, TC_ONLY_GEAR);
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC),
        Some(200)
    );
}

#[test]
fn tc_steps_move_one_mirek_and_stop_at_the_rendered_limit() {
    let mut fleet = GearFleet::demo_bus();
    stage_cct_at(&mut fleet, TC_ONLY_GEAR, 369);
    activate(&mut fleet, TC_ONLY_GEAR);

    for _ in 0..2 {
        enable_dt8(&mut fleet);
        exchange(
            &mut fleet,
            dt8(TC_ONLY_GEAR, Dt8Command::ColourTemperatureStepWarmer),
            false,
        );
    }
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC),
        Some(370),
        "one step reaches the warmest limit, the second is 'no change'"
    );

    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(TC_ONLY_GEAR, Dt8Command::ColourTemperatureStepCooler),
        false,
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC),
        Some(369)
    );

    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(RGBWAF_GEAR, Dt8Command::ColourTemperatureStepCooler),
        false,
    );
    assert_eq!(fleet.gears()[usize::from(RGBWAF_GEAR)].color_ct, 0);
}

#[test]
fn xy_steps_move_the_actual_coordinate_and_do_not_pair() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x00)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x10)), false);
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(RGBWAF_GEAR, Dt8Command::SetTemporaryXCoordinate),
        false,
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x00)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x20)), false);
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(RGBWAF_GEAR, Dt8Command::SetTemporaryYCoordinate),
        false,
    );
    activate(&mut fleet, RGBWAF_GEAR);

    enable_dt8(&mut fleet);
    assert!(
        !fleet.frame_pairs_now(dt8(RGBWAF_GEAR, Dt8Command::XCoordinateStepUp)),
        "a step under the DT8 prelude is a control command, never a send-twice pair"
    );
    exchange(
        &mut fleet,
        dt8(RGBWAF_GEAR, Dt8Command::XCoordinateStepUp),
        false,
    );
    assert_eq!(
        colour_value16(&mut fleet, RGBWAF_GEAR, COLOUR_VALUE_X),
        Some(0x1100)
    );

    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(RGBWAF_GEAR, Dt8Command::YCoordinateStepDown),
        false,
    );
    assert_eq!(
        colour_value16(&mut fleet, RGBWAF_GEAR, COLOUR_VALUE_Y),
        Some(0x1F00)
    );
}

#[test]
fn store_tc_limit_writes_cascade_and_snap_the_actual_colour() {
    let mut fleet = GearFleet::demo_bus();
    stage_cct_at(&mut fleet, TC_ONLY_GEAR, 300);
    activate(&mut fleet, TC_ONLY_GEAR);

    write_tc_limit(&mut fleet, TC_ONLY_GEAR, TC_LIMIT_SELECTOR_WARMEST, 250);
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC_WARMEST),
        Some(250)
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC_PHYSICAL_WARMEST),
        Some(370),
        "the physical limit is the fixture's own and does not move"
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC),
        Some(250),
        "§9.13: an actual Tc outside the new range snaps to the boundary"
    );
    assert_eq!(
        colour_status_of(&mut fleet, TC_ONLY_GEAR),
        TransferOutcome::Answer(DT8_STATUS_TC_ACTIVE | COLOUR_STATUS_TC_OUT_OF_RANGE)
    );

    write_tc_limit(&mut fleet, TC_ONLY_GEAR, TC_LIMIT_SELECTOR_COOLEST, 280);
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC_COOLEST),
        Some(280)
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC_WARMEST),
        Some(280)
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC),
        Some(280)
    );

    exchange(&mut fleet, special(SpecialCommand::Dtr0(350u16 as u8)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(0x01)), false);
    exchange(
        &mut fleet,
        special(SpecialCommand::Dtr2(TC_LIMIT_SELECTOR_WARMEST)),
        false,
    );
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(TC_ONLY_GEAR, Dt8Command::StoreColourTemperatureTcLimit),
        false,
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC_WARMEST),
        Some(280),
        "an unpaired 242 is discarded"
    );
}

#[test]
fn colour_value_answers_mask_for_empty_and_silence_for_unsupported() {
    let mut fleet = GearFleet::demo_bus();

    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_RED),
        TransferOutcome::NoAnswer,
        "no RGBWAF channels on the Tc-only gear"
    );
    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_X),
        TransferOutcome::NoAnswer,
        "xy not implemented"
    );
    assert_eq!(
        colour_value(&mut fleet, RGBWAF_GEAR, COLOUR_VALUE_TC),
        TransferOutcome::NoAnswer,
        "Tc not implemented on the strip"
    );
    assert_eq!(
        colour_value(&mut fleet, RGBWAF_GEAR, COLOUR_VALUE_TC_COOLEST),
        TransferOutcome::NoAnswer,
        "Tc limits belong to the Tc type"
    );

    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, 60),
        TransferOutcome::NoAnswer
    );

    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_REPORT_COLOUR_TYPE),
        TransferOutcome::Answer(0xFF),
        "an unloaded report answers its MASK type"
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_REPORT_TC),
        Some(0xFFFF)
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TEMPORARY_TC),
        Some(0xFFFF),
        "nothing staged"
    );

    assert_eq!(
        colour_value(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_NUMBER_OF_PRIMARIES),
        TransferOutcome::Answer(0),
        "no primaries, and the count says so instead of staying silent"
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, 66),
        Some(0xFFFF),
        "TY PRIMARY 0: the primary is not there, so the answer is MASK"
    );
}

#[test]
fn identify_device_moves_no_variable_at_all() {
    let mut fleet = GearFleet::demo_bus();
    exchange(
        &mut fleet,
        standard(0, StandardCommand::DirectArcPower { level: 120 }),
        false,
    );
    let before = {
        let g = &fleet.gears()[0];
        (g.level, g.last_active_level, g.power_cycle_seen, g.reset_state)
    };

    config(&mut fleet, standard(0, StandardCommand::IdentifyDevice));

    let after = &fleet.gears()[0];
    assert!(after.identifying, "§9.14.3.2: the procedure started");
    assert_eq!(after.level, before.0, "actualLevel must not move");
    assert_eq!(
        after.last_active_level, before.1,
        "§9.4: identification must not write lastActiveLevel"
    );
    assert_eq!(
        after.power_cycle_seen, before.2,
        "§9.16.9: identification is not a level command and must not clear powerCycleSeen"
    );
    assert_eq!(after.reset_state, before.3);
}

#[test]
fn a_query_leaves_identification_running_and_an_instruction_stops_it() {
    let mut fleet = GearFleet::demo_bus();
    config(&mut fleet, standard(0, StandardCommand::IdentifyDevice));

    exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true);
    assert!(
        fleet.gears()[0].identifying,
        "Table 93 i=5: a query must not stop the procedure"
    );

    exchange(
        &mut fleet,
        standard(0, StandardCommand::DirectArcPower { level: 60 }),
        false,
    );
    assert!(
        !fleet.gears()[0].identifying,
        "§9.14.3.1: any other instruction stops it"
    );
}

#[test]
fn a_single_identify_frame_does_not_start_the_procedure() {
    let mut fleet = GearFleet::demo_bus();
    exchange(
        &mut fleet,
        standard(0, StandardCommand::IdentifyDevice),
        false,
    );
    assert!(!fleet.gears()[0].identifying);
}

#[test]
fn the_bus_unit_extension_moves_the_declared_bound_not_just_the_bytes() {
    let mut spec = GearSpec::dt6(Some(0), demo_random_address(0));
    spec.bus_unit_extension = true;
    let mut fleet = GearFleet::new(vec![spec], 0, 7);
    let gear = &fleet.gears_mut()[0];
    assert_eq!(gear.memory_location(0, 0x00), Some(0x1C));
    assert!(gear.memory_location(0, 0x1B).is_some());
    assert!(gear.memory_location(0, 0x1C).is_some());
    assert_eq!(gear.memory_location(0, 0x1D), None, "past the declared bound");

    let plain = GearSpec::dt6(Some(1), demo_random_address(1));
    let mut fleet = GearFleet::new(vec![plain], 0, 7);
    let gear = &fleet.gears_mut()[0];
    assert_eq!(gear.memory_location(0, 0x00), Some(0x1A));
    assert_eq!(gear.memory_location(0, 0x1B), None, "Table 9 reserves it");
}

#[test]
fn bank_1_declares_the_length_its_content_format_requires() {
    for (format, last) in [
        (LuminaireFormat::V3, 0x77u8),
        (LuminaireFormat::V4, 0x8F),
        (LuminaireFormat::V5, 0xC9),
    ] {
        let mut spec = GearSpec::dt6(Some(0), demo_random_address(0));
        spec.luminaire_format = Some(format);
        let mut fleet = GearFleet::new(vec![spec], 0, 7);
        let gear = &fleet.gears_mut()[0];
        assert_eq!(gear.memory_location(1, 0x00), Some(last), "{format:?}");
        assert_eq!(gear.memory_location(1, 0x02), Some(BANK1_UNLOCKED));
        assert!(gear.memory_location(1, last).is_some());
        assert_eq!(gear.memory_location(1, last + 1), None);
    }

    let plain = GearSpec::dt6(Some(1), demo_random_address(1));
    let mut fleet = GearFleet::new(vec![plain], 0, 7);
    let gear = &fleet.gears_mut()[0];
    assert_eq!(gear.memory_location(1, 0x00), Some(0x10));
    assert_eq!(gear.memory_location(1, 0x11), None, "no content format ID");
}

fn dt6(addr: u8, command: Dt6Command) -> u16 {
    (u16::from(short(addr).encode_address_byte() | 0x01) << 8) | u16::from(command.opcode())
}

fn enable_dt6(fleet: &mut GearFleet) {
    exchange(
        fleet,
        special(SpecialCommand::EnableDeviceType(DeviceType::Led.code())),
        false,
    );
}

fn query_dimming_curve(fleet: &mut GearFleet, addr: u8) -> TransferOutcome {
    enable_dt6(fleet);
    exchange(fleet, dt6(addr, Dt6Command::QueryDimmingCurve), true)
}

fn select_dimming_curve(fleet: &mut GearFleet, addr: u8, curve: u8) {
    exchange(fleet, special(SpecialCommand::Dtr0(curve)), false);
    enable_dt6(fleet);
    config(fleet, dt6(addr, Dt6Command::SelectDimmingCurve));
}

#[test]
fn a_dimming_curve_write_is_a_send_twice_pair_read_back_by_its_query() {
    let mut fleet = GearFleet::demo_bus();
    assert_eq!(
        query_dimming_curve(&mut fleet, 0),
        TransferOutcome::Answer(0),
        "207 resets dimmingCurve to 0 — standard logarithmic"
    );
    select_dimming_curve(&mut fleet, 0, 1);
    assert_eq!(
        query_dimming_curve(&mut fleet, 0),
        TransferOutcome::Answer(1),
        "the linear curve is what the gear now applies"
    );
}

#[test]
fn one_half_of_the_dimming_curve_pair_does_nothing() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(1)), false);
    enable_dt6(&mut fleet);
    exchange(&mut fleet, dt6(0, Dt6Command::SelectDimmingCurve), false);
    assert_eq!(query_dimming_curve(&mut fleet, 0), TransferOutcome::Answer(0));
}

#[test]
fn the_same_opcode_under_a_dt8_prelude_never_touches_the_curve() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(1)), false);
    enable_dt8(&mut fleet);
    exchange(
        &mut fleet,
        dt8(RGBWAF_GEAR, Dt8Command::XCoordinateStepUp),
        false,
    );
    assert_eq!(
        query_dimming_curve(&mut fleet, 0),
        TransferOutcome::Answer(0),
        "a DT8 step must not reach a DT6 variable"
    );
}

#[test]
fn a_reserved_dimming_curve_value_leaves_the_gear_alone() {
    let mut fleet = GearFleet::demo_bus();
    select_dimming_curve(&mut fleet, 0, 1);
    select_dimming_curve(&mut fleet, 0, 2);
    assert_eq!(query_dimming_curve(&mut fleet, 0), TransferOutcome::Answer(1));
}

#[test]
fn a_gear_without_device_type_six_is_silent_on_the_query() {
    let mut fleet = GearFleet::demo_bus();
    assert_eq!(
        query_dimming_curve(&mut fleet, RGBWAF_GEAR),
        TransferOutcome::NoAnswer,
        "a DT8-only fixture has no dimmingCurve to report"
    );
}

fn status_of(fleet: &mut GearFleet, addr: u8) -> u8 {
    let outcome = exchange(fleet, standard(addr, StandardCommand::QueryStatus), true);
    let TransferOutcome::Answer(status) = outcome else {
        panic!("QUERY STATUS must answer, got {outcome:?}");
    };
    status
}

fn query_level(fleet: &mut GearFleet, addr: u8) -> u8 {
    let outcome = exchange(fleet, standard(addr, StandardCommand::QueryActualLevel), true);
    let TransferOutcome::Answer(level) = outcome else {
        panic!("QUERY ACTUAL LEVEL must answer, got {outcome:?}");
    };
    level
}

fn read_memory_frame(addr: u8) -> u16 {
    (u16::from(short(addr).encode_address_byte() | 0x01) << 8)
        | u16::from(READ_MEMORY_LOCATION_OPCODE)
}

#[test]
fn up_and_step_up_leave_a_dark_lamp_dark() {
    for command in [StandardCommand::Up, StandardCommand::StepUp] {
        let mut fleet = GearFleet::demo_bus();
        exchange(&mut fleet, standard(0, command), false);
        assert_eq!(
            query_level(&mut fleet, 0),
            0,
            "{command:?} must not light a dark lamp"
        );
    }
}

#[test]
fn on_and_step_up_lights_a_dark_lamp_at_min_level_and_steps_a_lit_one() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(50)), false);
    config(&mut fleet, standard(0, StandardCommand::SetMinLevel));

    exchange(&mut fleet, standard(0, StandardCommand::OnAndStepUp), false);
    assert_eq!(query_level(&mut fleet, 0), 50, "dark: on at minLevel, not min+1");
    exchange(&mut fleet, standard(0, StandardCommand::OnAndStepUp), false);
    assert_eq!(query_level(&mut fleet, 0), 51, "lit: one step up");

    dapc(&mut fleet, 0, 254);
    exchange(&mut fleet, standard(0, StandardCommand::OnAndStepUp), false);
    assert_eq!(query_level(&mut fleet, 0), 254, "at maxLevel: maxLevel");
}

#[test]
fn on_and_step_up_commits_staged_colour_like_any_arc_power_command() {
    let mut fleet = GearFleet::demo_bus();
    stage_cct_at(&mut fleet, TC_ONLY_GEAR, 200);
    exchange(
        &mut fleet,
        standard(TC_ONLY_GEAR, StandardCommand::OnAndStepUp),
        false,
    );
    assert_eq!(
        colour_status_of(&mut fleet, TC_ONLY_GEAR),
        TransferOutcome::Answer(DT8_STATUS_TC_ACTIVE),
        "the staged Tc committed"
    );
    assert_eq!(query_level(&mut fleet, TC_ONLY_GEAR), 1, "and the lamp lit at minLevel");
}

#[test]
fn query_lamp_power_on_agrees_with_the_status_byte_lamp_on_bit() {
    let mut fleet = GearFleet::demo_bus();
    let query = standard(0, StandardCommand::QueryLampPowerOn);
    dapc(&mut fleet, 0, 100);
    fleet.gears_mut()[0].lamp_starting = true;
    assert_eq!(
        exchange(&mut fleet, query, true),
        TransferOutcome::NoAnswer,
        "during startup"
    );
    fleet.gears_mut()[0].lamp_starting = false;
    fleet.gears_mut()[0].lamp_failure = true;
    assert_eq!(
        exchange(&mut fleet, query, true),
        TransferOutcome::NoAnswer,
        "on total lamp failure"
    );
    fleet.gears_mut()[0].lamp_failure = false;
    assert_eq!(
        exchange(&mut fleet, query, true),
        TransferOutcome::Answer(DALI_YES),
        "driven and producing light"
    );
}

#[test]
fn query_limit_error_answers_the_limit_error_bit() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(100)), false);
    config(&mut fleet, standard(0, StandardCommand::SetMinLevel));
    dapc(&mut fleet, 0, 50);
    assert_eq!(
        exchange(&mut fleet, standard(0, StandardCommand::QueryLimitError), true),
        TransferOutcome::Answer(DALI_YES),
        "a clamped DAPC raises limitError"
    );
    dapc(&mut fleet, 0, 150);
    assert_eq!(
        exchange(&mut fleet, standard(0, StandardCommand::QueryLimitError), true),
        TransferOutcome::NoAnswer,
        "an in-range DAPC resets it"
    );
}

#[test]
fn query_operating_mode_answers_standard_mode() {
    let mut fleet = GearFleet::demo_bus();
    assert_eq!(
        exchange(&mut fleet, standard(0, StandardCommand::QueryOperatingMode), true),
        TransferOutcome::Answer(0x00),
        "§9.9.2: standard mode"
    );
}

#[test]
fn lamp_failure_raises_bit_one_alone() {
    let mut fleet = GearFleet::demo_bus();
    fleet.gears_mut()[0].lamp_failure = true;
    let status = status_of(&mut fleet, 0);
    assert_ne!(status & STATUS_LAMP_FAILURE, 0, "§9.16.3: lampFailure");
    assert_eq!(
        status & STATUS_GEAR_FAILURE,
        0,
        "a lamp fault is not a gear fault (§9.16.2)"
    );
}

#[test]
fn reset_clears_power_cycle_seen() {
    let mut fleet = GearFleet::demo_bus();
    assert_ne!(
        status_of(&mut fleet, 0) & STATUS_POWER_CYCLE_SEEN,
        0,
        "set at power-up"
    );
    config(&mut fleet, standard(0, StandardCommand::Reset));
    assert_eq!(
        status_of(&mut fleet, 0) & STATUS_POWER_CYCLE_SEEN,
        0,
        "§9.16.9: RESET clears the bit"
    );
}

#[test]
fn reset_leaves_the_committed_colour_temperature_in_place() {
    let mut fleet = GearFleet::demo_bus();
    stage_cct_at(&mut fleet, TC_ONLY_GEAR, 200);
    activate(&mut fleet, TC_ONLY_GEAR);
    config(&mut fleet, standard(TC_ONLY_GEAR, StandardCommand::Reset));
    assert_eq!(
        colour_status_of(&mut fleet, TC_ONLY_GEAR),
        TransferOutcome::Answer(DT8_STATUS_TC_ACTIVE),
        "COLOUR STATUS: reset value is 'no change'"
    );
    assert_eq!(
        colour_value16(&mut fleet, TC_ONLY_GEAR, COLOUR_VALUE_TC),
        Some(200),
        "COLOUR TEMPERATURE Tc: reset value is 'no change'"
    );
}

#[test]
fn reset_leaves_rgbwaf_dim_levels_and_control_in_place() {
    let mut fleet = GearFleet::demo_bus();
    stage_rgbwaf_control(&mut fleet, 0x80);
    stage_rgb(&mut fleet, 200, 100, 50);
    dapc(&mut fleet, RGBWAF_GEAR, 200);
    config(&mut fleet, standard(RGBWAF_GEAR, StandardCommand::Reset));
    assert_eq!(
        read_rgbwaf_control(&mut fleet, RGBWAF_GEAR),
        TransferOutcome::Answer(0x80),
        "RGBWAF CONTROL: reset value is 'no change'"
    );
    assert_eq!(
        colour_value(&mut fleet, RGBWAF_GEAR, COLOUR_VALUE_RED),
        TransferOutcome::Answer(200),
        "RED DIMLEVEL: reset value is 'no change'"
    );
}

fn extended_version_query(addr: u8) -> u16 {
    (u16::from(short(addr).encode_address_byte() | 0x01) << 8)
        | u16::from(Dt6Command::QueryExtendedVersionNumber.opcode())
}

#[test]
fn extended_version_number_answers_per_the_live_prelude() {
    let mut fleet = GearFleet::demo_bus();

    enable_dt6(&mut fleet);
    assert!(fleet.expects_backward(extended_version_query(0)));
    assert_eq!(
        exchange(&mut fleet, extended_version_query(0), true),
        TransferOutcome::Answer(1),
        "IEC 62386-207:2009"
    );

    enable_dt8(&mut fleet);
    assert!(
        fleet.expects_backward(extended_version_query(TC_ONLY_GEAR)),
        "a wire driver must listen for the DT8 answer too"
    );
    assert_eq!(
        exchange(&mut fleet, extended_version_query(TC_ONLY_GEAR), true),
        TransferOutcome::Answer(2),
        "IEC 62386-209:2011"
    );

    enable_dt6(&mut fleet);
    assert_eq!(
        exchange(&mut fleet, extended_version_query(TC_ONLY_GEAR), true),
        TransferOutcome::NoAnswer,
        "a DT8-only gear does not hold a DT6 extended version"
    );

    assert_eq!(
        exchange(&mut fleet, extended_version_query(0), true),
        TransferOutcome::NoAnswer,
        "no prelude, no answer"
    );
}

#[test]
fn the_9_16_9_list_clears_power_cycle_seen_even_when_no_level_moves() {
    let receptions: [(&str, u16); 7] = [
        ("UP", standard(0, StandardCommand::Up)),
        ("DOWN", standard(0, StandardCommand::Down)),
        ("STEP UP", standard(0, StandardCommand::StepUp)),
        ("STEP DOWN", standard(0, StandardCommand::StepDown)),
        ("STEP DOWN AND OFF", standard(0, StandardCommand::StepDownAndOff)),
        (
            "GO TO SCENE (unprogrammed)",
            standard(0, StandardCommand::GoToScene { scene: 11 }),
        ),
        (
            "DAPC (MASK)",
            (u16::from(short(0).encode_address_byte()) << 8) | 0x00FF,
        ),
    ];
    for (name, frame) in receptions {
        let mut fleet = GearFleet::demo_bus();
        assert_ne!(
            status_of(&mut fleet, 0) & STATUS_POWER_CYCLE_SEEN,
            0,
            "{name}: bit set at power-up"
        );
        exchange(&mut fleet, frame, false);
        assert_eq!(
            status_of(&mut fleet, 0) & STATUS_POWER_CYCLE_SEEN,
            0,
            "{name}: §9.16.9 clears on reception"
        );
        assert_eq!(query_level(&mut fleet, 0), 0, "{name}: the dark lamp stayed dark");
    }
}

#[test]
fn off_clears_a_stale_limit_error_like_any_level_command() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(100)), false);
    config(&mut fleet, standard(0, StandardCommand::SetMinLevel));
    dapc(&mut fleet, 0, 50);
    assert_ne!(
        status_of(&mut fleet, 0) & STATUS_LIMIT_ERROR,
        0,
        "a clamped DAPC raises limitError first"
    );
    exchange(&mut fleet, standard(0, StandardCommand::Off), false);
    let status = status_of(&mut fleet, 0);
    assert_eq!(status & STATUS_LIMIT_ERROR, 0, "OFF is a level command");
    assert_eq!(query_level(&mut fleet, 0), 0);
}

#[test]
fn bank_zero_offset_one_answers_no_and_dtr0_still_increments() {
    let mut fleet = GearFleet::demo_bus();
    exchange(
        &mut fleet,
        special(SpecialCommand::Dtr1(MEMORY_BANK_IDENTITY)),
        false,
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x01)), false);
    let read = read_memory_frame(0);
    assert_eq!(
        exchange(&mut fleet, read, true),
        TransferOutcome::NoAnswer,
        "Table 9: reserved, answer NO"
    );
    assert_eq!(
        exchange(&mut fleet, standard(0, StandardCommand::QueryContentDtr0), true),
        TransferOutcome::Answer(0x02),
        "§9.10.4: incremented even though the location is not implemented"
    );
    assert_eq!(
        exchange(&mut fleet, read, true),
        TransferOutcome::Answer(BANK0_LAST_BANK),
        "the walk resumes at 0x02 with no re-arm"
    );
}

#[test]
fn dtr0_does_not_move_for_an_unimplemented_bank_or_at_the_top_location() {
    let mut fleet = GearFleet::demo_bus();
    let read = read_memory_frame(0);
    exchange(&mut fleet, special(SpecialCommand::Dtr1(42)), false);
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x05)), false);
    assert_eq!(exchange(&mut fleet, read, true), TransferOutcome::NoAnswer);
    assert_eq!(
        exchange(&mut fleet, standard(0, StandardCommand::QueryContentDtr0), true),
        TransferOutcome::Answer(0x05),
        "an unimplemented bank ignores the command"
    );

    exchange(
        &mut fleet,
        special(SpecialCommand::Dtr1(MEMORY_BANK_IDENTITY)),
        false,
    );
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0xFF)), false);
    assert_eq!(exchange(&mut fleet, read, true), TransferOutcome::NoAnswer);
    assert_eq!(
        exchange(&mut fleet, standard(0, StandardCommand::QueryContentDtr0), true),
        TransferOutcome::Answer(0xFF),
        "at 0xFF DTR0 shall not change — no wrap"
    );
}

#[test]
fn the_two_restored_102_queries_are_answered_rather_than_ignored() {
    let mut fleet = GearFleet::demo_bus();

    assert!(
        fleet.expects_backward(standard(0, StandardCommand::QueryManufacturerSpecificMode)),
        "0xA6 opens a backward window"
    );
    assert_eq!(
        exchange(
            &mut fleet,
            standard(0, StandardCommand::QueryManufacturerSpecificMode),
            true
        ),
        TransferOutcome::NoAnswer,
        "§11.5.27 NO for standard operating mode"
    );

    assert!(
        fleet.expects_backward(standard(0, StandardCommand::QueryControlGearFailure)),
        "0xAA opens a backward window"
    );
    assert_eq!(
        exchange(
            &mut fleet,
            standard(0, StandardCommand::QueryControlGearFailure),
            true
        ),
        TransferOutcome::NoAnswer,
        "a healthy gear answers NO"
    );
    fleet.gears_mut()[0].faults.forced_status = STATUS_GEAR_FAILURE;
    assert_eq!(
        exchange(
            &mut fleet,
            standard(0, StandardCommand::QueryControlGearFailure),
            true
        ),
        TransferOutcome::Answer(DALI_YES),
        "and YES once controlGearFailure is set"
    );
}

fn inject_failure(fleet: &mut GearFleet, addr: u8, byte: u8) {
    let gear = fleet
        .gears_mut()
        .iter_mut()
        .find(|g| g.spec.short_address == Some(addr))
        .expect("gear at that short address");
    gear.faults.failure_status = byte;
}

fn query_dt6(fleet: &mut GearFleet, addr: u8, command: Dt6Command) -> TransferOutcome {
    enable_dt6(fleet);
    exchange(fleet, dt6(addr, command), true)
}

#[test]
fn a_short_circuit_raises_lamp_failure_and_answers_the_207_byte() {
    let mut fleet = GearFleet::demo_bus();
    inject_failure(&mut fleet, 0, 0x01);

    assert_eq!(
        query_dt6(&mut fleet, 0, Dt6Command::QueryFailureStatus),
        TransferOutcome::Answer(0x01),
    );
    assert_eq!(
        query_dt6(&mut fleet, 0, Dt6Command::QueryShortCircuit),
        TransferOutcome::Answer(0xFF),
        "YES is a backward frame containing MASK",
    );
    assert_eq!(
        query_dt6(&mut fleet, 0, Dt6Command::QueryOpenCircuit),
        TransferOutcome::NoAnswer,
        "NO is silence (102 §3.13), never a zero byte",
    );
    let status = exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true);
    let TransferOutcome::Answer(raw) = status else {
        panic!("status must answer: {status:?}");
    };
    assert_eq!(raw & 0x02, 0x02, "bit 1 lampFailure, raised by the 207 byte");
    assert_eq!(raw & 0x01, 0x00, "bit 0 controlGearFailure is NOT tied to it");
}

#[test]
fn a_thermal_shut_down_reports_no_lamp_failure_and_a_masked_level() {
    let mut fleet = GearFleet::demo_bus();
    dapc(&mut fleet, 0, 200);
    inject_failure(&mut fleet, 0, 0x20);

    let status = exchange(&mut fleet, standard(0, StandardCommand::QueryStatus), true);
    let TransferOutcome::Answer(raw) = status else {
        panic!("status must answer: {status:?}");
    };
    assert_eq!(raw & 0x02, 0x00, "a thermal shut down is not a lamp failure");
    assert_eq!(raw & 0x04, 0x00, "and the lamp is off while it holds");
    assert_eq!(
        exchange(&mut fleet, standard(0, StandardCommand::QueryActualLevel), true),
        TransferOutcome::Answer(0xFF),
        "MASK: light was expected and there is none",
    );
    assert_eq!(
        query_dt6(&mut fleet, 0, Dt6Command::QueryThermalShutdown),
        TransferOutcome::Answer(0xFF),
    );
}

#[test]
fn a_healthy_dt6_gear_answers_features_and_a_zero_byte() {
    let mut fleet = GearFleet::demo_bus();
    assert_eq!(
        query_dt6(&mut fleet, 0, Dt6Command::QueryFeatures),
        TransferOutcome::Answer(0x7F),
        "bits 0-6: every detection this model can be asked about",
    );
    assert_eq!(
        query_dt6(&mut fleet, 0, Dt6Command::QueryFailureStatus),
        TransferOutcome::Answer(0x00),
    );
    assert_eq!(
        query_dt6(&mut fleet, 0, Dt6Command::QueryThermalOverload),
        TransferOutcome::NoAnswer,
    );
}

#[test]
fn a_dt8_gear_answers_no_part_207_query() {
    let mut fleet = GearFleet::demo_bus();
    assert_eq!(
        query_dt6(&mut fleet, 4, Dt6Command::QueryFailureStatus),
        TransferOutcome::NoAnswer,
    );
    assert_eq!(
        query_dt6(&mut fleet, 4, Dt6Command::QueryFeatures),
        TransferOutcome::NoAnswer,
    );
}

fn set_short_address(fleet: &mut GearFleet, addr: u8, operand: u8) {
    exchange(fleet, special(SpecialCommand::Dtr0(operand)), false);
    config(fleet, standard(addr, StandardCommand::SetShortAddress));
}

fn answers_at(fleet: &mut GearFleet, addr: u8) -> bool {
    matches!(
        exchange(fleet, standard(addr, StandardCommand::QueryStatus), true),
        TransferOutcome::Answer(_)
    )
}

#[test]
fn a_set_short_address_pair_moves_the_gear_to_the_dtr0_operand() {
    let mut fleet = GearFleet::demo_bus();
    set_short_address(&mut fleet, 5, 0x51);

    assert!(answers_at(&mut fleet, 40), "the gear did not move to 40");
    assert!(!answers_at(&mut fleet, 5), "and it must not still answer at 5");
}

#[test]
fn a_set_short_address_onto_a_reserved_address_is_refused() {
    let mut fleet = GearFleet::new(
        vec![GearSpec::dt6(Some(15), 0x0A_0B_0C)],
        crate::DEFAULT_RESERVED_SHORT_ADDRESSES,
        7,
    );
    let before = fleet.stats().refused_programs;
    set_short_address(&mut fleet, 15, 0x05);

    assert!(answers_at(&mut fleet, 15), "the gear must stay where it was");
    assert_eq!(
        fleet.stats().refused_programs,
        before + 1,
        "a refusal is counted, not silent"
    );
}

#[test]
fn a_malformed_short_address_operand_is_ignored() {
    let mut fleet = GearFleet::demo_bus();
    set_short_address(&mut fleet, 5, 0x50);

    assert!(answers_at(&mut fleet, 5), "the gear must not have moved");
    assert!(!answers_at(&mut fleet, 40), "and certainly not to 0x50 >> 1");
}

#[test]
fn a_mask_operand_deletes_the_short_address() {
    let mut fleet = GearFleet::demo_bus();
    set_short_address(&mut fleet, 5, 0xFF);

    assert!(!answers_at(&mut fleet, 5), "the address is gone");
    let broadcast_status = exchange(
        &mut fleet,
        frame(DaliCommand::Standard {
            address: DaliAddress::Broadcast,
            command: StandardCommand::QueryStatus,
        }),
        true,
    );
    assert!(
        !matches!(broadcast_status, TransferOutcome::NoAnswer),
        "an unaddressed gear still answers broadcast: {broadcast_status:?}"
    );
}

#[test]
fn one_half_of_the_short_address_pair_does_nothing() {
    let mut fleet = GearFleet::demo_bus();
    exchange(&mut fleet, special(SpecialCommand::Dtr0(0x51)), false);
    exchange(&mut fleet, standard(5, StandardCommand::SetShortAddress), false);

    assert!(answers_at(&mut fleet, 5), "a single frame must not move it");
    assert!(!answers_at(&mut fleet, 40));
}


#[cfg(test)]
mod bench_conformance {
    use super::*;

    const BENCH_GEAR: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/hil/corpus/20260915-213611/primary/gear/short-0.json"
    );

    const KNOWN_DIVERGENCE: &[(u8, &str, &str)] = &[
        (
            0x95,
            "model-only: state, not behaviour; each side answers honestly for the state it is in",
            "QUERY RESET STATE: a freshly built model IS at reset values \
            and answers YES (102 §11.5.6); every bench fixture is \
            configured and correctly stays silent",
        ),
        (
            0xA7,
            "bench-only",
            "QUERY NEXT DEVICE TYPE: all four bench fixtures answer 255 to \
            QUERY DEVICE TYPE — MASK, 'more than one' — and enumerate \
            through 0xA7 (§11.5.13). This model has no multi-type gear at \
            all: `GearSpec::dt8` declares a single 8. So the product's \
            device-type walk, which every real fixture on this bench \
            requires, is exercised against scripted mock frames and \
            against nothing that behaves like a driver. The wave that \
            closes it is a model that can hold a DeviceTypeSet",
        ),
    ];

    fn bench_answered() -> std::collections::BTreeSet<u8> {
        let text = std::fs::read_to_string(BENCH_GEAR)
            .unwrap_or_else(|e| panic!("bench corpus missing at {BENCH_GEAR}: {e}"));
        let standard = text
            .split_once("\"standard\": [")
            .expect("corpus carries a standard sweep")
            .1;
        let standard = standard.split_once("\n ]").map_or(standard, |(head, _)| head);
        let mut answered = std::collections::BTreeSet::new();
        let mut last_answered = false;
        for line in standard.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("\"answered\": ") {
                last_answered = value.trim_end_matches(',') == "true";
            } else if let Some(value) = line.strip_prefix("\"opcode\": ") {
                if last_answered {
                    if let Ok(op) = value.trim_end_matches(',').parse::<u8>() {
                        answered.insert(op);
                    }
                }
            }
        }
        answered
    }

    fn model_answered() -> std::collections::BTreeSet<u8> {
        let mut fleet = GearFleet::new(
            vec![GearSpec::dt8(Some(0), 0x5C_2A_42, (true, false, false), (153, 370))],
            0,
            1,
        );
        (0x90..=0xA8u8)
            .chain([0xAA])
            .chain(0xB0..=0xC4)
            .filter(|op| {
                let frame = (u16::from(0u8 << 1 | 1) << 8) | u16::from(*op);
                !matches!(fleet.exchange(frame, true), TransferOutcome::NoAnswer)
            })
            .collect()
    }

    #[test]
    fn the_model_answers_what_the_bench_fixtures_answered() {
        let bench = bench_answered();
        let model = model_answered();
        assert!(bench.len() > 30, "corpus parsed {} answers", bench.len());
        let known: std::collections::BTreeSet<u8> =
            KNOWN_DIVERGENCE.iter().map(|(op, _, _)| *op).collect();

        let diverging: Vec<String> = bench
            .symmetric_difference(&model)
            .filter(|op| !known.contains(op))
            .map(|op| format!("0x{op:02X}"))
            .collect();

        assert!(
            diverging.is_empty(),
            "model and bench fixture disagree on {diverging:?} — either the \
             model gained a behaviour a real driver does not have, or it lost \
             one it does. Neither belongs in KNOWN_DIVERGENCE without a reason."
        );
    }

    #[test]
    fn every_known_divergence_still_diverges() {
        let bench = bench_answered();
        let model = model_answered();
        for (op, side, reason) in KNOWN_DIVERGENCE {
            assert_ne!(
                bench.contains(op),
                model.contains(op),
                "0x{op:02X} no longer diverges — drop it from KNOWN_DIVERGENCE ({side}: {reason})"
            );
        }
    }
}
