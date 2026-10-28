use super::*;

const V3_LAYOUT: Bank1Layout = Bank1Layout {
    implemented: Some(true),
    content_format_id: Some(3),
    last_offset: Some(0x77),
};

fn field(name: &str) -> &'static LuminaireField {
    LuminaireField::named(name).expect(name)
}

#[test]
fn the_table_runs_in_offset_order_without_overlap() {
    for pair in LUMINAIRE_FIELDS.windows(2) {
        assert!(
            pair[0].last_offset() < pair[1].offset,
            "{} overlaps {}",
            pair[0].name,
            pair[1].name
        );
    }
}

#[test]
fn every_numeric_patch_bit_belongs_to_one_field() {
    let mut seen = 0u16;
    for entry in LUMINAIRE_FIELDS {
        if let FieldKind::Number { patch, .. } = entry.kind {
            assert_eq!(seen & patch, 0, "{} reuses a bit", entry.name);
            seen |= patch;
        }
    }
    assert_eq!(seen.count_ones(), 11, "the eleven numeric fields of Part 251");
}

#[test]
fn every_text_field_has_its_region() {
    for which in [
        T::LuminaireColour,
        T::LuminaireIdentification,
        T::LightDistribution,
        T::OemName,
        T::CustomerStockingNumber,
        T::FreeUse,
    ] {
        assert!(LuminaireField::of_text(which).is_some(), "{which:?}");
    }
}

#[test]
fn the_content_format_id_is_refused_before_anything_else() {
    let unknown = Bank1Layout::default();
    assert_eq!(
        check_writable(&unknown, field("content_format_id")),
        Err(Bank1WriteRefusal::FieldReadOnly)
    );
}

#[test]
fn a_bank_that_is_absent_is_not_implemented() {
    let absent = Bank1Layout::from_header(&[]);
    assert_eq!(check_writable(&absent, field("year")), Err(Bank1WriteRefusal::NotImplemented));
}

#[test]
fn an_unknown_or_foreign_format_is_not_part_251() {
    assert_eq!(
        check_writable(&Bank1Layout::default(), field("year")),
        Err(Bank1WriteRefusal::NotPart251)
    );
    let vendor = Bank1Layout { content_format_id: Some(0x0042), ..V3_LAYOUT };
    assert_eq!(check_writable(&vendor, field("year")), Err(Bank1WriteRefusal::NotPart251));
    let unmeasured = Bank1Layout { last_offset: None, ..V3_LAYOUT };
    assert_eq!(check_writable(&unmeasured, field("year")), Err(Bank1WriteRefusal::NotPart251));
}

#[test]
fn a_field_newer_than_the_format_is_refused() {
    assert_eq!(
        check_writable(&V3_LAYOUT, field("light_distribution")),
        Err(Bank1WriteRefusal::FieldNotInFormat)
    );
    assert_eq!(
        check_writable(&V3_LAYOUT, field("oem_name")),
        Err(Bank1WriteRefusal::FieldNotInFormat)
    );
    let v4 = Bank1Layout { content_format_id: Some(4), last_offset: Some(0x8F), ..V3_LAYOUT };
    assert_eq!(check_writable(&v4, field("light_distribution")), Ok(LuminaireFormat::V4));
    assert_eq!(check_writable(&v4, field("lamp_current_ma")), Err(Bank1WriteRefusal::FieldNotInFormat));
}

#[test]
fn a_bank_ending_inside_the_field_is_too_short() {
    let short = Bank1Layout { last_offset: Some(0x76), ..V3_LAYOUT };
    assert_eq!(
        check_writable(&short, field("luminaire_identification")),
        Err(Bank1WriteRefusal::TooShort),
        "0x3C + 60 ends at 0x77"
    );
    assert_eq!(check_writable(&short, field("luminaire_colour")), Ok(LuminaireFormat::V3));
}

#[test]
fn the_header_names_its_format_and_length() {
    let mut header = [0u8; 0x13];
    header[0] = 0x77;
    header[0x11] = 0x00;
    header[0x12] = 0x05;
    let layout = Bank1Layout::from_header(&header);
    assert_eq!(layout.content_format_id, Some(5));
    assert_eq!(layout.last_offset, Some(0x77));
    let cut = Bank1Layout::from_header(&header[..0x12]);
    assert_eq!(cut.content_format_id, None, "a header that stops at 0x11 holds no format ID");
}

#[test]
fn a_number_is_accepted_only_where_the_read_side_shows_it_back() {
    let v3 = LuminaireFormat::V3;
    assert!(field("year").accepts_number(99, v3));
    assert!(!field("year").accepts_number(100, v3));
    assert!(!field("week").accepts_number(0, v3), "week 0 reads back as unknown");
    assert!(field("week").accepts_number(53, v3));
    assert!(!field("nominal_input_power_w").accepts_number(0xFFFF, v3), "MASK is unknown");
    assert!(!field("nominal_min_ac_voltage_v").accepts_number(89, v3));
    assert!(field("nominal_light_output_lm").accepts_number(0x00FF_FFFE, v3));
    assert!(!field("nominal_light_output_lm").accepts_number(0x0100_0000, v3), "wider than 24 bits");
    assert!(!field("cct_kelvin").accepts_number(0xFFFE, v3), "MASK-1 is Part 209, not a CCT");
    assert!(!field("light_distribution_type").accepts_number(6, v3));
    assert!(field("light_distribution_type").accepts_number(6, LuminaireFormat::V4));
    assert!(!field("luminaire_colour").accepts_number(1, v3));
}

#[test]
fn text_is_printable_ascii_that_fits_its_region() {
    assert!(field("free_use").accepts_text("12345678"));
    assert!(!field("free_use").accepts_text("123456789"));
    assert!(!field("luminaire_colour").accepts_text("caf\u{e9}"));
    assert!(!field("luminaire_colour").accepts_text("tab\there"));
    assert!(field("luminaire_colour").accepts_text(""));
    assert!(!field("year").accepts_text("24"));
}

#[test]
fn numbers_are_written_big_endian_at_their_width_in_offset_order() {
    let numbers = number_patch(&[
        (N::PATCH_CCT_KELVIN, 2_700),
        (N::PATCH_YEAR, 24),
        (N::PATCH_NOMINAL_LIGHT_OUTPUT_LM, 0x01_02_03),
    ]);
    let writes = planned_writes(&numbers, None);
    let shape: Vec<(&str, Vec<u8>)> =
        writes.iter().map(|write| (write.field.name, write.bytes.clone())).collect();
    assert_eq!(
        shape,
        vec![
            ("year", vec![24]),
            ("nominal_light_output_lm", vec![0x01, 0x02, 0x03]),
            ("cct_kelvin", vec![0x0A, 0x8C]),
        ]
    );
}

#[test]
fn a_short_text_is_null_terminated_and_a_full_one_is_not() {
    let colour = text_patch(field("luminaire_colour"), "cool").expect("text field");
    let writes = planned_writes(&N::default(), Some(&colour));
    assert_eq!(writes[0].bytes, b"cool\0");
    let full = text_patch(field("free_use"), "12345678").expect("text field");
    assert_eq!(planned_writes(&N::default(), Some(&full))[0].bytes, b"12345678");
    assert!(text_patch(field("year"), "24").is_none());
}

#[test]
fn a_write_is_held_only_by_a_bank_carrying_its_bytes() {
    let numbers = number_patch(&[(N::PATCH_YEAR, 24)]);
    let write = &planned_writes(&numbers, None)[0];
    let mut bank = vec![0u8; 0x78];
    assert!(!write.held_by(&bank));
    bank[0x13] = 24;
    assert!(write.held_by(&bank));
    assert!(!write.held_by(&bank[..0x13]), "a bank read that stops short proves nothing");
}
