use core::sync::atomic::AtomicU32;

#[derive(Debug, Default)]
pub struct MqttCounters {
    pub connected: AtomicU32,
    pub connects_total: AtomicU32,
    pub publishes_total: AtomicU32,
    pub publish_failures_total: AtomicU32,
    pub commands_received_total: AtomicU32,
    pub commands_dropped_total: AtomicU32,
    pub rule_publishes_dropped_total: AtomicU32,
    pub commands_unroutable_total: AtomicU32,
    pub commands_ingress_rejected_total: AtomicU32,
    pub discovery_published_total: AtomicU32,
    pub discovery_failed_total: AtomicU32,
    pub bus_discarded_total: AtomicU32,
    pub bus_coalesced_total: AtomicU32,
    pub terminal_event_publish_failed_total: AtomicU32,
    pub terminal_event_publish_retried_total: AtomicU32,
}

impl MqttCounters {
    pub fn load(counter: &AtomicU32) -> u32 {
        dali2rust_bus::worker_counters::load(counter)
    }

    pub fn set_connected(&self, up: bool) {
        dali2rust_bus::worker_counters::set_flag(&self.connected, up);
    }

    pub fn is_connected(&self) -> bool {
        Self::load(&self.connected) != 0
    }
}
