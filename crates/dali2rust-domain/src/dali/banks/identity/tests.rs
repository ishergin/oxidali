use super::*;

const IDENTITY: DeviceIdentity =
    DeviceIdentity { gtin: 0x009D_ADA2_1B43, identification_number: 0x2601_06FA_9E78_7D9B };

const FIRMWARE_VERSION: [u8; 2] = [0x01, 0x00];

fn window() -> Vec<u8> {
    let mut bytes = IDENTITY.gtin.to_be_bytes()[2..].to_vec();
    bytes.extend_from_slice(&FIRMWARE_VERSION);
    bytes.extend_from_slice(&IDENTITY.identification_number.to_be_bytes());
    bytes
}

fn flipped(index: u8) -> Vec<u8> {
    let mut bytes = window();
    bytes[usize::from(index - GTIN_OFFSET)] ^= 0x01;
    bytes
}

fn holds_another(window: &[u8]) -> bool {
    DeviceIdentity::in_window(window).is_some_and(|seen| seen != IDENTITY)
}

#[test]
fn the_window_of_bank_0_from_the_gtin_to_the_identification_number_holds_the_identity() {
    assert_eq!(window().len(), usize::from(IDENTITY_WINDOW_LEN));
    assert_eq!(DeviceIdentity::in_window(&window()), Some(IDENTITY));
}

#[test]
fn a_gtin_off_in_its_last_byte_is_another_device() {
    let last = GTIN_OFFSET + GTIN_LEN as u8 - 1;
    assert!(holds_another(&flipped(last)));
}

#[test]
fn an_identification_number_off_in_its_last_byte_is_another_device() {
    let last = IDENTIFICATION_NUMBER_OFFSET + IDENTIFICATION_NUMBER_LEN as u8 - 1;
    assert!(holds_another(&flipped(last)));
}

#[test]
fn the_firmware_version_between_them_is_not_identity() {
    assert_eq!(DeviceIdentity::in_window(&flipped(GTIN_OFFSET + GTIN_LEN as u8)), Some(IDENTITY));
}

#[test]
fn a_window_that_stops_short_holds_no_identity_to_compare() {
    let window = window();
    assert_eq!(DeviceIdentity::in_window(&window[..window.len() - 1]), None, "neither this device nor another");
    let longer = [window.as_slice(), &[0x00]].concat();
    assert_eq!(DeviceIdentity::in_window(&longer), None, "a window is read whole or not at all");
}
