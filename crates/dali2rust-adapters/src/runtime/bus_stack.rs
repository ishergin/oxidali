use dali2rust_bus::{BusChannel, BusFrame, BusHost, BusPublisher, PublishResult};

use super::workers::SpawnedWorkers;

pub struct BusStackRuntime {
    pub(crate) _host: BusHost,
    pub(crate) event_publisher: BusPublisher,
    pub(crate) _worker: std::thread::JoinHandle<()>,
    pub(crate) _registry_worker: std::thread::JoinHandle<()>,
    pub(crate) _bridge: std::thread::JoinHandle<()>,
    pub(crate) _display: std::thread::JoinHandle<()>,
    pub(crate) _operation_tracker_worker: std::thread::JoinHandle<()>,
    pub(crate) _apply_orchestrator: std::thread::JoinHandle<()>,
    pub(crate) _projector: std::thread::JoinHandle<()>,
    pub(crate) _sniffer_translator: std::thread::JoinHandle<()>,
    pub(crate) _hcl_scheduler: std::thread::JoinHandle<()>,
    pub(crate) _poller: std::thread::JoinHandle<()>,
    _rules: std::thread::JoinHandle<()>,
    _arbitration_supervisor: std::thread::JoinHandle<()>,
    _replication: std::thread::JoinHandle<()>,
    _arbitration: std::thread::JoinHandle<()>,
    pub(crate) _ws_worker: Option<std::thread::JoinHandle<()>>,
    pub(crate) _mqtt_worker: Option<std::thread::JoinHandle<()>>,
}

impl BusStackRuntime {
    pub(crate) fn new(host: BusHost, event_publisher: BusPublisher, workers: SpawnedWorkers) -> Self {
        Self {
            _host: host,
            event_publisher,
            _worker: workers.dali,
            _registry_worker: workers.registry,
            _bridge: workers.confirmation_bridge,
            _display: workers.display,
            _operation_tracker_worker: workers.operation_tracker_worker,
            _apply_orchestrator: workers.apply_orchestrator,
            _projector: workers.projector,
            _sniffer_translator: workers.sniffer_translator,
            _hcl_scheduler: workers.hcl_scheduler,
            _poller: workers.poller,
            _rules: workers.rules,
            _arbitration_supervisor: workers.arbitration_supervisor,
            _replication: workers.replication,
            _arbitration: workers.arbitration,
            _ws_worker: workers.ws_worker,
            _mqtt_worker: workers.mqtt,
        }
    }

    pub fn try_publish_ip_address_assigned(&self, ip: &str) -> PublishResult {
        let ev = dali2rust_contracts::bus::event_envelope(dali2rust_contracts::SOURCE_ID_UNSPECIFIED, 0, 0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::IpAddressAssignedEvent::from_ip_text(ip));
        self.event_publisher
            .try_publish(BusChannel::Events, BusFrame::event(ev))
    }
}

impl std::fmt::Debug for BusStackRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BusStackRuntime").finish_non_exhaustive()
    }
}
