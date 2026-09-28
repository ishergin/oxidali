use dali2rust_platform::net::{node_name, LinkStats, LinkStatus, NetworkLink};

pub const MOCK_LINK_SPEED_MBPS: u16 = 100;

pub struct MockNetworkLink {
    mac: [u8; 6],
    ipv4: Option<[u8; 4]>,
}

impl MockNetworkLink {
    pub fn new(mac: [u8; 6], ipv4: Option<[u8; 4]>) -> Self {
        Self { mac, ipv4 }
    }
}

impl NetworkLink for MockNetworkLink {
    fn kind(&self) -> &'static str {
        "mock"
    }

    fn status(&self) -> LinkStatus {
        LinkStatus {
            up: true,
            speed_mbps: MOCK_LINK_SPEED_MBPS,
            full_duplex: true,
            ipv4: self.ipv4,
        }
    }

    fn stats(&self) -> LinkStats {
        LinkStats::default()
    }

    fn hardware_address(&self) -> Option<[u8; 6]> {
        Some(self.mac)
    }

    fn hostname(&self) -> Option<String> {
        Some(node_name(self.mac))
    }
}
