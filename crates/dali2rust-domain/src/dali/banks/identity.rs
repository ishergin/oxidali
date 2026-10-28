// IEC 62386-102 §9.10.6, Table 9
pub const GTIN_OFFSET: u8 = 0x03;
pub const GTIN_LEN: usize = 6;
pub const IDENTIFICATION_NUMBER_OFFSET: u8 = 0x0B;
pub const IDENTIFICATION_NUMBER_LEN: usize = 8;
pub const IDENTITY_WINDOW_LEN: u16 =
    (IDENTIFICATION_NUMBER_OFFSET - GTIN_OFFSET) as u16 + IDENTIFICATION_NUMBER_LEN as u16;

// IEC 62386-102 §9.10.6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub gtin: u64,
    pub identification_number: u64,
}

impl DeviceIdentity {
    pub fn in_window(window: &[u8]) -> Option<Self> {
        if window.len() != usize::from(IDENTITY_WINDOW_LEN) {
            return None;
        }
        Some(Self {
            gtin: number_within(window, GTIN_OFFSET, GTIN_LEN),
            identification_number: number_within(window, IDENTIFICATION_NUMBER_OFFSET, IDENTIFICATION_NUMBER_LEN),
        })
    }
}

fn number_within(window: &[u8], offset: u8, len: usize) -> u64 {
    let start = usize::from(offset - GTIN_OFFSET);
    window
        .iter()
        .skip(start)
        .take(len)
        .fold(0, |number, byte| number << 8 | u64::from(*byte))
}

#[cfg(test)]
mod tests;
