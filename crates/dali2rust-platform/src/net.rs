#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkStatus {
    pub up: bool,
    pub speed_mbps: u16,
    pub full_duplex: bool,
    pub ipv4: Option<[u8; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkStats {
    pub rx_packets: u32,
    pub tx_packets: u32,
    pub rx_bytes: u32,
    pub tx_bytes: u32,
    pub rx_dropped: u32,
    pub tx_dropped: u32,
    pub rx_ring_overruns: u32,
    pub rx_fifo_overflows: u32,
    pub link_up_events: u32,
}

pub trait NetworkLink: Send + Sync {
    fn kind(&self) -> &'static str;

    fn status(&self) -> LinkStatus;

    fn stats(&self) -> LinkStats;

    fn hardware_address(&self) -> Option<[u8; 6]> {
        None
    }

    fn hostname(&self) -> Option<String> {
        None
    }
}

pub const NODE_NAME_PREFIX: &str = "dali-";

pub fn node_name(mac: [u8; 6]) -> String {
    let [_, _, _, a, b, c] = mac;
    format!("{NODE_NAME_PREFIX}{a:02x}{b:02x}{c:02x}")
}

pub fn mac_text(mac: [u8; 6]) -> String {
    let [a, b, c, d, e, f] = mac;
    format!("{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x}")
}

pub fn ipv4_text(ip: [u8; 4]) -> String {
    let [a, b, c, d] = ip;
    format!("{a}.{b}.{c}.{d}")
}

impl LinkStats {
    pub fn since(&self, earlier: Self) -> Self {
        Self {
            rx_packets: self.rx_packets.wrapping_sub(earlier.rx_packets),
            tx_packets: self.tx_packets.wrapping_sub(earlier.tx_packets),
            rx_bytes: self.rx_bytes.wrapping_sub(earlier.rx_bytes),
            tx_bytes: self.tx_bytes.wrapping_sub(earlier.tx_bytes),
            rx_dropped: self.rx_dropped.wrapping_sub(earlier.rx_dropped),
            tx_dropped: self.tx_dropped.wrapping_sub(earlier.tx_dropped),
            rx_ring_overruns: self.rx_ring_overruns.wrapping_sub(earlier.rx_ring_overruns),
            rx_fifo_overflows: self.rx_fifo_overflows.wrapping_sub(earlier.rx_fifo_overflows),
            link_up_events: self.link_up_events.wrapping_sub(earlier.link_up_events),
        }
    }

    pub fn has_loss(&self) -> bool {
        self.rx_dropped != 0
            || self.tx_dropped != 0
            || self.rx_ring_overruns != 0
            || self.rx_fifo_overflows != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_is_wrapping_so_a_byte_counter_rollover_is_not_a_negative_spike() {
        let earlier = LinkStats {
            rx_bytes: u32::MAX - 100,
            ..Default::default()
        };
        let now = LinkStats {
            rx_bytes: 50,
            ..Default::default()
        };
        assert_eq!(now.since(earlier).rx_bytes, 151);
    }

    #[test]
    fn has_loss_ignores_traffic_and_link_events() {
        let busy = LinkStats {
            rx_packets: 1_000_000,
            tx_bytes: 999_999,
            link_up_events: 3,
            ..Default::default()
        };
        assert!(!busy.has_loss());
    }

    #[test]
    fn a_node_is_named_after_the_last_three_bytes_of_its_mac() {
        assert_eq!(node_name([0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4]), "dali-7b90e4");
    }

    #[test]
    fn addresses_render_as_their_conventional_text() {
        assert_eq!(mac_text([0x02, 0x00, 0x00, 0xa1, 0xb2, 0xc3]), "02:00:00:a1:b2:c3");
        assert_eq!(ipv4_text([192, 168, 13, 240]), "192.168.13.240");
    }

    #[test]
    fn has_loss_reports_each_kind_of_loss() {
        for stats in [
            LinkStats { rx_dropped: 1, ..Default::default() },
            LinkStats { tx_dropped: 1, ..Default::default() },
            LinkStats { rx_ring_overruns: 1, ..Default::default() },
            LinkStats { rx_fifo_overflows: 1, ..Default::default() },
        ] {
            assert!(stats.has_loss(), "{stats:?} should count as loss");
        }
    }
}
