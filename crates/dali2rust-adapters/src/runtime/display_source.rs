use std::sync::atomic::Ordering;
use std::sync::Arc;

use dali2rust_display_runtime::source::{DisplaySample, DisplaySource};
use dali2rust_platform::dali::{DaliWireCounters, PhySnifferCounters, WIRE_LOAD_FULL_PERMILLE};
use dali2rust_platform::wall_clock::{TimeSource, WallClock};
use dali2rust_registry_runtime::RegistryStore;

pub(crate) struct ControllerDisplaySource {
    pub registry: Arc<RegistryStore>,
    pub wire: Arc<DaliWireCounters>,
    pub phy_sniffer: Arc<PhySnifferCounters>,
    pub poller: Arc<dali2rust_poller_runtime::PollerCounters>,
    pub hcl_overrides: dali2rust_hcl_runtime::SharedOverrideLedger,
    pub mqtt: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    pub ws: Arc<dali2rust_ws_runtime::WsCounters>,
    pub clock: Arc<dyn WallClock>,
    pub adapter_id: u8,
}

impl DisplaySource for ControllerDisplaySource {
    fn sample(&self) -> DisplaySample {
        let parts = self.registry.display_sample_parts(self.adapter_id);
        let counts = parts.counts;
        DisplaySample {
            adapter_id: self.adapter_id,
            gear_known: counts.gear_known,
            gear_unreachable: counts.gear_unreachable,
            gear_faulted: counts.gear_faulted,
            input_known: counts.input_known,
            input_present: counts.input_present,
            adapter_enabled: parts.adapter_enabled,
            redundancy_enabled: parts.redundancy_enabled,
            controller_active: parts.controller_active,
            time_synced: self.clock.source() != TimeSource::Unset,
            frames_sent_total: self.frames_sent(),
            foreign_frames_total: self.phy_sniffer.frames.load(Ordering::Relaxed),
            wire_load_permille: self.wire_load_permille(),
            poller_enabled: parts.poller_enabled,
            poller_interval_ms: parts.poller_interval_ms,
            poller_reads_failed: self.poller.reads_failed.load(Ordering::Relaxed),
            poller_reads_absent: self.poller.reads_absent.load(Ordering::Relaxed),
            hcl_overrides_active: self.overrides_active(),
            mqtt_connected: self.mqtt.connected.load(Ordering::Relaxed) != 0,
            ws_clients: self.ws.clients.load(Ordering::Relaxed),
            persistence_failed: self.persistence_failed(),
        }
    }
}

impl ControllerDisplaySource {
    fn wire_load_permille(&self) -> u16 {
        let permille = self.wire.load_permille.load(Ordering::Relaxed);
        u16::try_from(permille.min(WIRE_LOAD_FULL_PERMILLE)).unwrap_or(u16::MAX)
    }

    fn frames_sent(&self) -> u32 {
        self.wire
            .frames_sent_by_priority
            .iter()
            .fold(0u32, |acc, c| acc.saturating_add(c.load(Ordering::Relaxed)))
    }

    fn overrides_active(&self) -> u32 {
        u32::try_from(
            self.hcl_overrides
                .lock()
                .map(|l| l.len())
                .unwrap_or_default(),
        )
        .unwrap_or(u32::MAX)
    }

    fn persistence_failed(&self) -> u32 {
        let c = self.registry.persistence_counters();
        c.flush_error_total
            .load(Ordering::Relaxed)
            .saturating_add(c.no_space_total.load(Ordering::Relaxed))
            .saturating_add(c.hydrate_error_total.load(Ordering::Relaxed))
    }
}
