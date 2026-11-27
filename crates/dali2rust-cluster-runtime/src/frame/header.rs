use serde::{Deserialize, Serialize};

use super::limits::is_node;

const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub cluster_tag: u32,
    pub sender_mac: [u8; 6],
    pub boot_epoch: u32,
    pub node_no: u8,
    pub seq: u32,
}

impl Header {
    pub(crate) const fn in_range(&self) -> bool {
        is_node(self.node_no)
    }
}

pub fn cluster_tag(cluster_name: &str) -> u32 {
    cluster_name
        .bytes()
        .fold(FNV_OFFSET_BASIS, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(FNV_PRIME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cluster_tag_is_fnv_1a_32_over_the_name_bytes() {
        assert_eq!(cluster_tag(""), 0x811c_9dc5);
        assert_eq!(cluster_tag("a"), 0xe40c_292c);
        assert_eq!(cluster_tag("foobar"), 0xbf9c_f968);
    }

    #[test]
    fn the_cluster_tag_takes_the_name_as_it_is_written() {
        assert_ne!(cluster_tag("Home"), cluster_tag("home"));
        assert_ne!(cluster_tag("home "), cluster_tag("home"));
    }
}
