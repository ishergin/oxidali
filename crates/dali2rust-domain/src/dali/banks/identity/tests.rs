use super::*;

const IDENTITY: DeviceIdentity = DeviceIdentity {
    gtin: 0x009D_ADA2_1B43,
    identification_number: 0x2601_06FA_9E78_7D9B,
    logical_gear_index: 0,
};

const FIRMWARE_VERSION: [u8; 2] = [0x01, 0x00];
const BETWEEN_NUMBER_AND_INDEX: [u8; 7] = [0x01, 0x00, 0x08, 0x08, 0xFF, 0x00, 0x01];
const DECLARED_LAST: u8 = 0x1C;
const META: [u8; 3] = [DECLARED_LAST, 0xFF, 0x01];
const TAIL: [u8; 2] = [0x02, 0x05];

fn window() -> Vec<u8> {
    let mut bytes = IDENTITY.gtin.to_be_bytes()[2..].to_vec();
    bytes.extend_from_slice(&FIRMWARE_VERSION);
    bytes.extend_from_slice(&IDENTITY.identification_number.to_be_bytes());
    bytes.extend_from_slice(&BETWEEN_NUMBER_AND_INDEX);
    bytes.push(IDENTITY.logical_gear_index);
    bytes
}

fn bank0() -> Vec<u8> {
    [META.as_slice(), &window(), &TAIL].concat()
}

fn at(offset: u8) -> usize {
    usize::from(offset - GTIN_OFFSET)
}

fn flipped(offset: u8) -> Vec<u8> {
    let mut bytes = window();
    bytes[at(offset)] ^= 0x01;
    bytes
}

fn masked(offset: u8, len: usize) -> Vec<u8> {
    let mut bytes = window();
    bytes[at(offset)..at(offset) + len].fill(MASK);
    bytes
}

fn holds_another(window: &[u8]) -> bool {
    matches!(IdentityRead::of_window(window), IdentityRead::Known(seen) if seen != IDENTITY)
}

#[test]
fn the_window_from_the_gtin_to_the_logical_unit_index_holds_the_identity() {
    assert_eq!(window().len(), usize::from(IDENTITY_WINDOW_LEN));
    assert_eq!(IdentityRead::of_window(&window()), IdentityRead::Known(IDENTITY));
}

#[test]
fn a_gtin_off_in_its_last_byte_is_another_device() {
    assert!(holds_another(&flipped(GTIN_OFFSET + GTIN_LEN as u8 - 1)));
}

#[test]
fn an_identification_number_off_in_its_last_byte_is_another_device() {
    assert!(holds_another(&flipped(IDENTIFICATION_NUMBER_OFFSET + IDENTIFICATION_NUMBER_LEN as u8 - 1)));
}

#[test]
fn another_logical_unit_index_is_another_device() {
    assert!(holds_another(&flipped(LOGICAL_GEAR_INDEX_OFFSET)));
}

#[test]
fn nothing_between_the_numbers_and_the_index_is_identity() {
    let firmware = GTIN_OFFSET + GTIN_LEN as u8;
    let after_number = IDENTIFICATION_NUMBER_OFFSET + IDENTIFICATION_NUMBER_LEN as u8;
    for offset in (firmware..IDENTIFICATION_NUMBER_OFFSET).chain(after_number..LOGICAL_GEAR_INDEX_OFFSET) {
        assert_eq!(IdentityRead::of_window(&flipped(offset)), IdentityRead::Known(IDENTITY), "offset {offset:#04x}");
    }
}

#[test]
fn a_window_read_short_or_long_holds_no_identity_to_compare() {
    let window = window();
    assert_eq!(IdentityRead::of_window(&window[..window.len() - 1]), IdentityRead::Unread);
    let longer = [window.as_slice(), &[0x00]].concat();
    assert_eq!(IdentityRead::of_window(&longer), IdentityRead::Unread, "a window is read whole or not at all");
}

// IEC 62386-102 §9.10.6
#[test]
fn an_unprogrammed_gtin_or_identification_number_cannot_tell_two_gear_apart() {
    let gtin = masked(GTIN_OFFSET, GTIN_LEN);
    assert_eq!(IdentityRead::of_window(&gtin), IdentityRead::Unprovable, "a real number behind a MASK GTIN");
    let number = masked(IDENTIFICATION_NUMBER_OFFSET, IDENTIFICATION_NUMBER_LEN);
    assert_eq!(IdentityRead::of_window(&number), IdentityRead::Unprovable, "a real GTIN before a MASK number");
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn a_logical_unit_index_that_reads_mask_cannot_tell_two_units_apart() {
    let index = masked(LOGICAL_GEAR_INDEX_OFFSET, 1);
    assert_eq!(IdentityRead::of_window(&index), IdentityRead::Unprovable, "MASK is never an index");
}

#[test]
fn a_mask_byte_inside_a_programmed_number_is_part_of_it() {
    let one_byte = masked(IDENTIFICATION_NUMBER_OFFSET + IDENTIFICATION_NUMBER_LEN as u8 - 1, 1);
    assert!(holds_another(&one_byte));
}

#[test]
fn a_bank_0_read_from_its_start_holds_the_identity_at_its_offsets() {
    assert_eq!(IdentityRead::of_bank0(&bank0()), IdentityRead::Known(IDENTITY));
}

#[test]
fn a_bank_0_read_that_stopped_before_the_index_is_unread_and_a_re_read_may_help() {
    let bank = bank0();
    assert_eq!(IdentityRead::of_bank0(&bank[..usize::from(LOGICAL_GEAR_INDEX_OFFSET)]), IdentityRead::Unread);
    assert_eq!(IdentityRead::of_bank0(&[]), IdentityRead::Unread);
}

// IEC 62386-102 §9.10.6, Table 9
#[test]
fn a_bank_0_declared_shorter_than_the_index_never_carries_an_identity() {
    let mut bank = bank0();
    bank[0] = LOGICAL_GEAR_INDEX_OFFSET - 1;
    assert_eq!(IdentityRead::of_bank0(&bank), IdentityRead::Unprovable, "re-reading cannot help");
    bank[0] = LOGICAL_GEAR_INDEX_OFFSET;
    assert_eq!(IdentityRead::of_bank0(&bank), IdentityRead::Known(IDENTITY), "a bank ending at the index carries it");
}

#[test]
fn a_refusal_names_whether_a_re_read_can_help() {
    assert_eq!(IdentityRead::Known(IDENTITY).known(), Ok(IDENTITY));
    assert_eq!(IdentityRead::Unread.known(), Err(IDENTITY_UNKNOWN));
    assert_eq!(IdentityRead::Unprovable.known(), Err(IDENTITY_UNPROVABLE));
}
