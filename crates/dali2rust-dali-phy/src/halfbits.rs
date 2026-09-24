#[derive(Clone, Debug, Default)]
pub struct HalfBitBuffer {
    pub data: [u8; Self::DATA_LEN],
    pub length: u8,
}

impl HalfBitBuffer {
    pub const MAX_DATA_HALF_BITS: u8 = 72;
    pub const DATA_LEN: usize = 9;
}
