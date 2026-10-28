// IEC 62386-102 §9.10.6, Table 9
pub const IDENTITY_BANK: u8 = 0;
pub const GTIN_OFFSET: u8 = 0x03;
pub const GTIN_LEN: usize = 6;
pub const IDENTIFICATION_NUMBER_OFFSET: u8 = 0x0B;
pub const IDENTIFICATION_NUMBER_LEN: usize = 8;
pub const IDENTIFICATION_NUMBER: core::ops::Range<usize> =
    IDENTIFICATION_NUMBER_OFFSET as usize..IDENTIFICATION_NUMBER_OFFSET as usize + IDENTIFICATION_NUMBER_LEN;
pub const LOGICAL_GEAR_INDEX_OFFSET: u8 = 0x1A;
pub const IDENTITY_WINDOW_LEN: u16 = (LOGICAL_GEAR_INDEX_OFFSET - GTIN_OFFSET) as u16 + 1;

pub const IDENTITY_UNKNOWN: &str = "device_identity_unknown";
pub const IDENTITY_UNPROVABLE: &str = "device_identity_unprovable";

pub const MASK: u8 = 0xFF;

// IEC 62386-102 §9.10.6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub gtin: u64,
    pub identification_number: u64,
    pub logical_gear_index: u8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IdentityRead {
    #[default]
    Unread,
    Unprovable,
    Known(DeviceIdentity),
}

impl IdentityRead {
    // IEC 62386-102 §9.10.6, Table 9
    pub fn of_bank0(bank: &[u8]) -> Self {
        let window = usize::from(GTIN_OFFSET)..=usize::from(LOGICAL_GEAR_INDEX_OFFSET);
        match bank.first() {
            None => Self::Unread,
            Some(last) if *last < LOGICAL_GEAR_INDEX_OFFSET => Self::Unprovable,
            Some(_) => bank.get(window).map_or(Self::Unread, Self::of_window),
        }
    }

    // IEC 62386-102 §9.10.6, Table 9
    pub fn of_window(window: &[u8]) -> Self {
        if window.len() != usize::from(IDENTITY_WINDOW_LEN) {
            return Self::Unread;
        }
        let gtin = within(window, GTIN_OFFSET, GTIN_LEN);
        let number = within(window, IDENTIFICATION_NUMBER_OFFSET, IDENTIFICATION_NUMBER_LEN);
        let index = within(window, LOGICAL_GEAR_INDEX_OFFSET, 1);
        if is_mask(gtin) || is_mask(number) || is_mask(index) {
            return Self::Unprovable;
        }
        Self::Known(DeviceIdentity {
            gtin: number_of(gtin),
            identification_number: number_of(number),
            logical_gear_index: index.first().copied().unwrap_or(MASK),
        })
    }

    pub const fn known(self) -> Result<DeviceIdentity, &'static str> {
        match self {
            Self::Known(identity) => Ok(identity),
            Self::Unread => Err(IDENTITY_UNKNOWN),
            Self::Unprovable => Err(IDENTITY_UNPROVABLE),
        }
    }
}

fn within(window: &[u8], offset: u8, len: usize) -> &[u8] {
    let start = usize::from(offset - GTIN_OFFSET);
    window.get(start..start + len).unwrap_or_default()
}

fn is_mask(bytes: &[u8]) -> bool {
    bytes.iter().all(|byte| *byte == MASK)
}

fn number_of(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |number, byte| number << 8 | u64::from(*byte))
}

#[cfg(test)]
mod tests;
