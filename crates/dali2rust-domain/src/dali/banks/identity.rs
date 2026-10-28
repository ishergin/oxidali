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
    pub fn read_in(&self, window: &[u8]) -> bool {
        let gtin = self.gtin.to_be_bytes();
        let number = self.identification_number.to_be_bytes();
        let gtin = gtin.get(gtin.len() - GTIN_LEN..);
        within(window, GTIN_OFFSET, GTIN_LEN) == gtin
            && within(window, IDENTIFICATION_NUMBER_OFFSET, IDENTIFICATION_NUMBER_LEN) == Some(number.as_slice())
    }
}

fn within(window: &[u8], offset: u8, len: usize) -> Option<&[u8]> {
    let start = usize::from(offset - GTIN_OFFSET);
    window.get(start..start + len)
}

#[cfg(test)]
mod tests;
