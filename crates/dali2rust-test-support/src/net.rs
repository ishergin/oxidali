use dali2rust_platform::net::{LinkStats, LinkStatus, NetworkLink};

pub const MOCK_LINK_SPEED_MBPS: u16 = 100;

pub struct MockNetworkLink {
    mac: [u8; 6],
    ipv4: Option<[u8; 4]>,
    hostname: String,
}

impl MockNetworkLink {
    pub fn new(mac: [u8; 6], ipv4: Option<[u8; 4]>, hostname: &str) -> Self {
        Self { mac, ipv4, hostname: hostname.to_string() }
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
        Some(self.hostname.clone())
    }
}
