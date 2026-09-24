use dali2rust_domain::dali::banks::part251::{self, LuminaireFormat, TextRegion};

pub(crate) const LUMINAIRE_FIRST_OFFSET: u8 = part251::CONTENT_FORMAT_ID_OFFSET;
const LUMINAIRE_LAST_OFFSET: u8 = 0xC9;
const LUMINAIRE_EXT_LEN: usize =
    (LUMINAIRE_LAST_OFFSET as usize) - (LUMINAIRE_FIRST_OFFSET as usize) + 1;

#[derive(Debug)]
pub struct LuminaireBank {
    last_accessible: u8,
    bytes: [u8; LUMINAIRE_EXT_LEN],
}

impl LuminaireBank {
    pub fn new(seed: u32, format: LuminaireFormat, tc_capable: bool) -> Self {
        let mut bank = Self {
            last_accessible: format.min_last_offset(),
            bytes: [0u8; LUMINAIRE_EXT_LEN],
        };
        bank.put_u16(part251::CONTENT_FORMAT_ID_OFFSET, format.content_format_id());
        bank.fill_numbers(seed, tc_capable);
        bank.fill_text(seed, format);
        bank
    }

    pub const fn last_accessible(&self) -> u8 {
        self.last_accessible
    }

    pub fn location(&self, offset: u8) -> Option<u8> {
        if offset > self.last_accessible || offset < LUMINAIRE_FIRST_OFFSET {
            return None;
        }
        self.bytes
            .get(usize::from(offset - LUMINAIRE_FIRST_OFFSET))
            .copied()
    }

    fn put(&mut self, offset: u8, value: u8) {
        if offset < LUMINAIRE_FIRST_OFFSET {
            return;
        }
        if let Some(slot) = self.bytes.get_mut(usize::from(offset - LUMINAIRE_FIRST_OFFSET)) {
            *slot = value;
        }
    }

    fn put_u16(&mut self, offset: u8, value: u16) {
        self.put(offset, (value >> 8) as u8);
        self.put(offset + 1, (value & 0xFF) as u8);
    }

    fn put_u24(&mut self, offset: u8, value: u32) {
        self.put(offset, ((value >> 16) & 0xFF) as u8);
        self.put(offset + 1, ((value >> 8) & 0xFF) as u8);
        self.put(offset + 2, (value & 0xFF) as u8);
    }

    fn fill_numbers(&mut self, seed: u32, tc_capable: bool) {
        self.put(part251::YEAR_OFFSET, 22 + (seed % 4) as u8);
        let week = if seed % 4 == 3 { 0 } else { 1 + (seed % 52) as u8 };
        self.put(part251::WEEK_OFFSET, week);
        self.put_u16(part251::NOMINAL_INPUT_POWER_OFFSET, 30 + (seed % 20) as u16);
        self.put_u16(part251::POWER_AT_MINIMUM_OFFSET, 2 + (seed % 3) as u16);
        self.put_u16(part251::NOMINAL_MIN_AC_VOLTAGE_OFFSET, 198);
        self.put_u16(part251::NOMINAL_MAX_AC_VOLTAGE_OFFSET, 264);
        self.put_u24(part251::NOMINAL_LIGHT_OUTPUT_OFFSET, 3_000 + (seed % 900));
        self.put(part251::CRI_OFFSET, 80 + (seed % 20) as u8);
        let cct = if tc_capable { u16::MAX - 1 } else { 3_000 + (seed % 8) as u16 * 250 };
        self.put_u16(part251::CCT_OFFSET, cct);
        self.put(part251::LIGHT_DISTRIBUTION_TYPE_OFFSET, 1 + (seed % 5) as u8);
    }

    fn fill_text(&mut self, seed: u32, format: LuminaireFormat) {
        self.put_text(part251::LUMINAIRE_COLOUR, b"warm white");
        let mut ident = *b"DALI2RUST BENCH LUMINAIRE 0000 / ASSET TAG A-000000000000000";
        write_digits(&mut ident[26..30], seed);
        self.put_text(part251::LUMINAIRE_IDENTIFICATION, &ident);
        if format.has_light_distribution() {
            self.put_text(part251::LIGHT_DISTRIBUTION, b"symmetric wide");
        }
        if format.has_extended_block() {
            self.put_text(part251::OEM_NAME, b"dali2rust reference OEM");
            self.put_text(part251::CUSTOMER_STOCKING_NUMBER, b"STK-0001");
            self.put_u16(part251::LAMP_CURRENT_OFFSET, 350 + (seed % 100) as u16);
            self.put_text(part251::FREE_USE, b"site-01");
        }
    }

    fn put_text(&mut self, region: TextRegion, text: &[u8]) {
        for i in 0..region.len {
            let byte = text.get(usize::from(i)).copied().unwrap_or(0);
            self.put(region.offset + i, byte);
        }
    }
}

fn write_digits(slot: &mut [u8], value: u32) {
    let mut n = value;
    for cell in slot.iter_mut().rev() {
        *cell = b'0' + (n % 10) as u8;
        n /= 10;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_domain::dali::banks::part251::CctReading;

    fn image(bank: &LuminaireBank) -> Vec<u8> {
        (0..=bank.last_accessible())
            .map(|offset| bank.location(offset).unwrap_or(0))
            .collect()
    }

    #[test]
    fn the_content_format_id_is_what_the_product_gates_on() {
        for format in [LuminaireFormat::V3, LuminaireFormat::V4, LuminaireFormat::V5] {
            let bank = LuminaireBank::new(7, format, false);
            let img = image(&bank);
            let id = u16::from(img[usize::from(part251::CONTENT_FORMAT_ID_OFFSET)]) << 8
                | u16::from(img[usize::from(part251::CONTENT_FORMAT_ID_OFFSET) + 1]);
            assert_eq!(LuminaireFormat::from_content_format_id(id), Some(format));
            assert_eq!(bank.last_accessible(), format.min_last_offset());
        }
    }

    #[test]
    fn a_tunable_gear_answers_the_part_209_encoding_for_cct() {
        let bank = LuminaireBank::new(1, LuminaireFormat::V3, true);
        let img = image(&bank);
        let raw = u16::from(img[usize::from(part251::CCT_OFFSET)]) << 8
            | u16::from(img[usize::from(part251::CCT_OFFSET) + 1]);
        assert_eq!(part251::cct(raw), CctReading::Part209Implemented);

        let fixed = LuminaireBank::new(1, LuminaireFormat::V3, false);
        let img = image(&fixed);
        let raw = u16::from(img[usize::from(part251::CCT_OFFSET)]) << 8
            | u16::from(img[usize::from(part251::CCT_OFFSET) + 1]);
        assert!(matches!(part251::cct(raw), CctReading::Kelvin(_)));
    }

    #[test]
    fn every_generated_number_is_inside_its_own_range_of_validity() {
        let bank = LuminaireBank::new(5, LuminaireFormat::V5, false);
        let img = image(&bank);
        let at = |o: u8| img[usize::from(o)];
        assert!(part251::year(at(part251::YEAR_OFFSET)).is_some());
        assert!(part251::cri(at(part251::CRI_OFFSET)).is_some());
        let volts = u16::from(at(part251::NOMINAL_MIN_AC_VOLTAGE_OFFSET)) << 8
            | u16::from(at(part251::NOMINAL_MIN_AC_VOLTAGE_OFFSET + 1));
        assert_eq!(part251::mains_voltage_v(volts), Some(198));
    }

    #[test]
    fn one_seed_in_four_reports_week_as_unknown() {
        let bank = LuminaireBank::new(3, LuminaireFormat::V3, false);
        let img = image(&bank);
        assert_eq!(part251::week(img[usize::from(part251::WEEK_OFFSET)]), None);
    }

    #[test]
    fn text_regions_round_trip_through_the_products_own_reader() {
        let bank = LuminaireBank::new(42, LuminaireFormat::V5, false);
        let img = image(&bank);
        assert_eq!(
            part251::text_bytes(&img, part251::LUMINAIRE_COLOUR),
            Some(&b"warm white"[..])
        );
        let ident = part251::text_bytes(&img, part251::LUMINAIRE_IDENTIFICATION).expect("read");
        assert_eq!(ident.len(), 60, "a full region carries no terminator");
        assert!(ident.starts_with(b"DALI2RUST BENCH LUMINAIRE 0042"));
        assert_eq!(
            part251::text_bytes(&img, part251::FREE_USE),
            Some(&b"site-01"[..])
        );
    }

    #[test]
    fn a_format_3_gear_stops_where_format_3_stops() {
        let bank = LuminaireBank::new(1, LuminaireFormat::V3, false);
        assert_eq!(bank.last_accessible(), 0x77);
        assert!(bank.location(0x77).is_some());
        assert!(bank.location(0x78).is_none(), "light distribution is format 4+");
        assert!(bank.location(part251::OEM_NAME.offset).is_none());
    }

    #[test]
    fn the_base_of_bank_1_is_not_this_extensions_business() {
        let bank = LuminaireBank::new(1, LuminaireFormat::V3, false);
        for offset in 0..LUMINAIRE_FIRST_OFFSET {
            assert!(bank.location(offset).is_none(), "offset {offset:#04x}");
        }
    }
}
