#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplaySample {
    pub adapter_id: u8,
    pub gear_known: u8,
    pub gear_unreachable: u8,
    pub gear_faulted: u8,
    pub input_known: u8,
    pub input_present: u8,
    pub adapter_enabled: bool,
    pub redundancy_enabled: bool,
    pub controller_active: bool,
    pub time_synced: bool,
    pub frames_sent_total: u32,
    pub foreign_frames_total: u32,
    pub wire_load_permille: u16,
    pub poller_enabled: bool,
    pub poller_interval_ms: u32,
    pub poller_reads_failed: u32,
    pub poller_reads_absent: u32,
    pub hcl_overrides_active: u32,
    pub mqtt_connected: bool,
    pub ws_clients: u32,
    pub persistence_failed: u32,
}

pub trait DisplaySource: Send + Sync {
    fn sample(&self) -> DisplaySample;
}

#[derive(Debug, Default)]
pub struct StaticDisplaySource(pub DisplaySample);

impl DisplaySource for StaticDisplaySource {
    fn sample(&self) -> DisplaySample {
        self.0
    }
}
