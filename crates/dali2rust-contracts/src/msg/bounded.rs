use core::fmt;

use heapless::Vec as HeaplessVec;
use heapless::String as HeaplessString;
use serde::{Deserialize, Serialize};

pub type FixedText<const N: usize> = HeaplessString<N>;

pub type FixedText32 = HeaplessString<32>;
pub type FixedText48 = HeaplessString<48>;
pub type FixedText64 = HeaplessString<64>;
pub type FixedText96 = HeaplessString<96>;
pub type FixedItems<T, const N: usize> = HeaplessVec<T, N>;

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedBytes24 {
    pub len: u8,
    pub data: [u8; 24],
}

impl FixedBytes24 {
    pub fn from_slice(bytes: &[u8]) -> Self {
        let mut data = [0u8; 24];
        let take = bytes.len().min(24);
        data[..take].copy_from_slice(&bytes[..take]);
        Self {
            len: take as u8,
            data,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

impl fmt::Debug for FixedBytes24 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FixedBytes24")
            .field("len", &self.len)
            .finish()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedBytes32 {
    pub len: u8,
    pub data: [u8; 32],
}

impl FixedBytes32 {
    pub fn from_slice(bytes: &[u8]) -> Self {
        let mut data = [0u8; 32];
        let take = bytes.len().min(32);
        data[..take].copy_from_slice(&bytes[..take]);
        Self {
            len: take as u8,
            data,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

impl fmt::Debug for FixedBytes32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FixedBytes32")
            .field("len", &self.len)
            .finish()
    }
}

fn fixed_text<const N: usize>(input: &str) -> HeaplessString<N> {
    let mut out = HeaplessString::new();
    for ch in input.chars() {
        if out.len() + ch.len_utf8() > N {
            break;
        }
        let _ = out.push(ch);
    }
    out
}

pub fn fixed_text_32(input: &str) -> FixedText32 {
    fixed_text::<32>(input)
}

pub fn fixed_text_48(input: &str) -> FixedText48 {
    fixed_text::<48>(input)
}

pub fn fixed_text_64(input: &str) -> FixedText64 {
    fixed_text::<64>(input)
}

pub fn fixed_text_96(input: &str) -> FixedText96 {
    fixed_text::<96>(input)
}
