use dali2rust_domain::dali::banks::{
    encode_value, ValueWidth, BANK_ACTIVE_ENERGY, BANK_APPARENT_ENERGY,
    BANK_CONTROL_GEAR_DIAGNOSTICS, BANK_LIGHT_SOURCE_DIAGNOSTICS, BANK_LOADSIDE_ENERGY,
    BANK_LUMINAIRE_MAINTENANCE,
};

const BANK_UNLOCKED: u8 = 0x55;

const ENERGY_BANK_LEN: usize = 0x10;
const GEAR_BANK_LEN: usize = 0x1D;
const SOURCE_BANK_LEN: usize = 0x21;
const LUMINAIRE_BANK_LEN: usize = 0x08;

const BANK_VERSION: u8 = 0x01;

#[derive(Debug, Clone)]
pub struct MeteringBanks {
    active_energy: [u8; ENERGY_BANK_LEN],
    apparent_energy: [u8; ENERGY_BANK_LEN],
    loadside_energy: Option<[u8; ENERGY_BANK_LEN]>,
    control_gear: [u8; GEAR_BANK_LEN],
    light_source: [u8; SOURCE_BANK_LEN],
    luminaire: [u8; LUMINAIRE_BANK_LEN],
    energy: bool,
    diagnostics: bool,
}

fn write_meta(bank: &mut [u8]) {
    bank[0x00] = (bank.len() - 1) as u8;
    bank[0x01] = 0x00;
    bank[0x02] = BANK_UNLOCKED;
    bank[0x03] = BANK_VERSION;
}

fn put(bank: &mut [u8], offset: usize, raw: u64, width: ValueWidth) {
    encode_value(raw, width, &mut bank[offset..offset + width.bytes()]);
}

fn build_energy_bank(energy_wh: u64, power_mw: u64) -> [u8; ENERGY_BANK_LEN] {
    let mut b = [0u8; ENERGY_BANK_LEN];
    write_meta(&mut b);
    put(&mut b, 0x04, 0u64, ValueWidth::I8);
    put(&mut b, 0x05, energy_wh, ValueWidth::U48);
    put(&mut b, 0x0B, (-3i64) as u64 & 0xFF, ValueWidth::I8);
    put(&mut b, 0x0C, power_mw, ValueWidth::U32);
    b
}

fn build_gear_bank(seed: u32) -> [u8; GEAR_BANK_LEN] {
    let mut b = [0u8; GEAR_BANK_LEN];
    write_meta(&mut b);
    put(&mut b, 0x04, u64::from(seed % 1_000_000) + 3_600, ValueWidth::U32);
    put(&mut b, 0x08, u64::from(seed % 500), ValueWidth::U24);
    put(&mut b, 0x0B, 2_300, ValueWidth::U16);
    b[0x0D] = 50;
    b[0x0E] = 95;
    b[0x1B] = 60 + 45;
    b[0x1C] = 100;
    b
}

fn build_source_bank(seed: u32) -> [u8; SOURCE_BANK_LEN] {
    let mut b = [0u8; SOURCE_BANK_LEN];
    write_meta(&mut b);
    let starts = u64::from(seed % 400);
    put(&mut b, 0x04, starts, ValueWidth::U24);
    put(&mut b, 0x07, starts + 17, ValueWidth::U24);
    let on_time = u64::from(seed % 900_000) + 1_200;
    put(&mut b, 0x0A, on_time, ValueWidth::U32);
    put(&mut b, 0x0E, on_time + 4_242, ValueWidth::U32);
    put(&mut b, 0x12, 240, ValueWidth::U16);
    put(&mut b, 0x14, 350, ValueWidth::U16);
    b[0x20] = 60 + 50;
    b
}

fn build_luminaire_bank() -> [u8; LUMINAIRE_BANK_LEN] {
    let mut b = [0u8; LUMINAIRE_BANK_LEN];
    write_meta(&mut b);
    b[0x04] = 50;
    b[0x05] = 0xFF;
    put(&mut b, 0x06, 5_000, ValueWidth::U16);
    b
}

impl MeteringBanks {
    pub fn new(seed: u32, energy: bool, diagnostics: bool) -> Self {
        let energy_wh = u64::from(seed % 100_000) + 12_345;
        Self {
            active_energy: build_energy_bank(energy_wh, 18_500),
            apparent_energy: build_energy_bank(energy_wh + energy_wh / 20, 19_500),
            loadside_energy: None,
            control_gear: build_gear_bank(seed),
            light_source: build_source_bank(seed),
            luminaire: build_luminaire_bank(),
            energy,
            diagnostics,
        }
    }

    pub fn last_accessible_bank(&self, without_metering: u8) -> u8 {
        if self.diagnostics {
            BANK_LUMINAIRE_MAINTENANCE
        } else if self.energy {
            BANK_APPARENT_ENERGY
        } else {
            without_metering
        }
    }

    pub fn location(&self, bank: u8, offset: u8) -> Option<u8> {
        self.image(bank)?.get(usize::from(offset)).copied()
    }

    pub fn implements_bank(&self, bank: u8) -> bool {
        self.image(bank).is_some()
    }

    fn image(&self, bank: u8) -> Option<&[u8]> {
        Some(match bank {
            BANK_ACTIVE_ENERGY if self.energy => &self.active_energy,
            BANK_APPARENT_ENERGY if self.energy => &self.apparent_energy,
            BANK_LOADSIDE_ENERGY if self.energy && self.loadside_energy.is_some() => {
                self.loadside_energy.as_ref().expect("checked")
            }
            BANK_CONTROL_GEAR_DIAGNOSTICS if self.diagnostics => &self.control_gear,
            BANK_LIGHT_SOURCE_DIAGNOSTICS if self.diagnostics => &self.light_source,
            BANK_LUMINAIRE_MAINTENANCE if self.diagnostics => &self.luminaire,
            _ => return None,
        })
    }

    pub fn set_temporarily_unavailable(&mut self, on: bool) {
        let power = if on {
            ValueWidth::U32.tmask()
        } else {
            18_500
        };
        put(&mut self.active_energy, 0x0C, power, ValueWidth::U32);
        put(&mut self.apparent_energy, 0x0C, power, ValueWidth::U32);
        let voltage = if on { ValueWidth::U16.tmask() } else { 2_300 };
        put(&mut self.control_gear, 0x0B, voltage, ValueWidth::U16);
        self.control_gear[0x0E] = if on { ValueWidth::U8.tmask() as u8 } else { 95 };
    }

    pub fn implement_loadside(&mut self, energy_wh: u64, power_available: bool) {
        let mut bank = build_energy_bank(energy_wh, 0);
        if !power_available {
            put(&mut bank, 0x0C, ValueWidth::U32.tmask(), ValueWidth::U32);
        }
        self.loadside_energy = Some(bank);
    }

    pub fn set_condition(&mut self, bank: u8, flag_offset: u8, on: bool, count: u8) {
        let image: &mut [u8] = match bank {
            BANK_CONTROL_GEAR_DIAGNOSTICS => &mut self.control_gear,
            BANK_LIGHT_SOURCE_DIAGNOSTICS => &mut self.light_source,
            _ => return,
        };
        let idx = usize::from(flag_offset);
        if idx + 1 < image.len() {
            image[idx] = u8::from(on);
            image[idx + 1] = count;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_domain::dali::banks::{bank_last_offset, read_value, BankValue};

    #[test]
    fn each_bank_declares_the_last_offset_the_standard_gives_it() {
        let banks = MeteringBanks::new(7, true, true);
        for bank in [
            BANK_ACTIVE_ENERGY,
            BANK_APPARENT_ENERGY,
            BANK_CONTROL_GEAR_DIAGNOSTICS,
            BANK_LIGHT_SOURCE_DIAGNOSTICS,
            BANK_LUMINAIRE_MAINTENANCE,
        ] {
            assert_eq!(
                banks.location(bank, 0x00),
                bank_last_offset(bank),
                "bank {bank} offset 0x00"
            );
            assert_eq!(banks.location(bank, 0x03), Some(BANK_VERSION));
        }
    }

    #[test]
    fn an_unimplemented_bank_answers_nothing() {
        let banks = MeteringBanks::new(7, true, true);
        assert_eq!(banks.location(BANK_LOADSIDE_ENERGY, 0x00), None);
        let energy_only = MeteringBanks::new(7, true, false);
        assert_eq!(energy_only.location(BANK_CONTROL_GEAR_DIAGNOSTICS, 0x00), None);
        assert_eq!(energy_only.last_accessible_bank(1), BANK_APPARENT_ENERGY);
    }

    #[test]
    fn the_scale_factor_is_twos_complement() {
        let banks = MeteringBanks::new(7, true, true);
        assert_eq!(banks.location(BANK_ACTIVE_ENERGY, 0x0B), Some(0xFD));
        assert_eq!(banks.location(BANK_ACTIVE_ENERGY, 0x04), Some(0x00));
    }

    #[test]
    fn tmask_is_settable_and_reads_back_as_tmask_not_as_a_number() {
        let mut banks = MeteringBanks::new(7, true, true);
        banks.set_temporarily_unavailable(true);
        let image: Vec<u8> = (0x0Cu8..0x10).map(|o| banks.location(BANK_ACTIVE_ENERGY, o).unwrap()).collect();
        assert_eq!(
            read_value(&image, 0, ValueWidth::U32),
            Some(BankValue::TemporarilyUnavailable)
        );
        banks.set_temporarily_unavailable(false);
        let image: Vec<u8> = (0x0Cu8..0x10).map(|o| banks.location(BANK_ACTIVE_ENERGY, o).unwrap()).collect();
        assert_eq!(
            read_value(&image, 0, ValueWidth::U32),
            Some(BankValue::Reading(18_500))
        );
    }

    #[test]
    fn an_unwritten_luminaire_constant_is_mask() {
        let banks = MeteringBanks::new(7, true, true);
        let byte = banks
            .location(BANK_LUMINAIRE_MAINTENANCE, 0x05)
            .expect("implemented");
        assert_eq!(
            read_value(&[byte], 0, ValueWidth::U8),
            Some(BankValue::NotImplemented)
        );
    }
}
