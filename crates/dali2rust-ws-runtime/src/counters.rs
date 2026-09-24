use core::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Default)]
pub struct WsCounters {
    pub clients: AtomicU32,
    pub events_sent_total: AtomicU32,
    pub events_dropped_total: AtomicU32,
    pub events_coalesced_total: AtomicU32,
    pub inbox_overflow_total: AtomicU32,
    pub upgrades_rejected_total: AtomicU32,
    pub sniffer_records_total: AtomicU32,
    pub sniffer_dropped_total: AtomicU32,
    pub logs_lines_total: AtomicU32,
    pub logs_dropped_total: AtomicU32,
}

impl WsCounters {
    pub fn bump(counter: &AtomicU32, by: u32) {
        dali2rust_bus::worker_counters::bump_by(counter, by);
    }

    pub fn load(counter: &AtomicU32) -> u32 {
        dali2rust_bus::worker_counters::load(counter)
    }

    pub fn client_connected(&self) {
        self.clients.fetch_add(1, Ordering::Relaxed);
    }

    pub fn client_disconnected(&self) {
        let _ = self
            .clients
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(1))
            });
    }
}
