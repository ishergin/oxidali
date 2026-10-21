use super::*;

#[test]
fn the_product_lamp_limit_and_the_wire_address_space_are_separate_facts() {
    assert_eq!(usize::from(VIRTUAL_LAMP_COUNT), MAX_SHORT_ADDRESSES);
    assert!(u32::from(GROUP_COUNT) <= u16::BITS);
}

#[test]
fn groups_and_scenes_are_each_four_bits_wide() {
    assert_eq!(GROUP_COUNT, 16);
    assert_eq!(SCENE_COUNT, 16);
}

#[test]
fn a_lamp_id_is_below_the_count_not_below_the_type() {
    const { assert!(VIRTUAL_LAMP_COUNT < u8::MAX, "MAX would be a valid id") };
    assert!(
        (0..VIRTUAL_LAMP_COUNT).all(|id| usize::from(id) < MAX_SHORT_ADDRESSES),
        "every lamp row must be able to name a short address"
    );
}

#[test]
fn both_conversions_saturate_rather_than_truncate() {
    assert_eq!(kelvin_to_mirek(15), Some(u16::MAX));
    assert_eq!(kelvin_to_mirek(1), Some(u16::MAX));
    assert_eq!(mirek_to_kelvin(15), Some(u16::MAX));
    assert_eq!(mirek_to_kelvin(1), Some(u16::MAX));
    assert_eq!(kelvin_to_mirek(0), None);
    assert_eq!(mirek_to_kelvin(0), None);
}

#[test]
fn the_admitted_kelvin_band_round_trips() {
    for kelvin in [1_000u16, 2_700, 4_000, 6_500, 20_000] {
        let mirek = kelvin_to_mirek(kelvin).expect("a real temperature");
        let back = mirek_to_kelvin(mirek).expect("a real mirek");
        let drift = i32::from(back) - i32::from(kelvin);
        assert!(
            drift.abs() <= i32::from(kelvin) / 100,
            "{kelvin} K -> {mirek} mirek -> {back} K drifted {drift}"
        );
    }
}
