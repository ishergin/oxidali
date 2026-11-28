use std::sync::{Arc, Mutex};

use dali2rust_adapters::{
    build_http_test_stack, fixed_boot_epoch, BusStackRuntime, ClusterComposition, DaliRuntimeConfig, HttpTestPorts,
};
use dali2rust_api::http::router::Router;
use dali2rust_bus::BusConfig;
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_platform::dali::DaliTransport;
use dali2rust_test_support::{ClusterHub, ClusterHubBinder, MockNetworkLink};

pub struct Board {
    pub mac: [u8; 6],
    pub address: [u8; 4],
    pub epoch: u32,
}

pub fn stack_on<T: DaliTransport + Send + 'static>(
    hub: &Arc<ClusterHub>,
    board: &Board,
    period_ms: u32,
    wire: Arc<Mutex<T>>,
) -> (Router, Box<BusStackRuntime>) {
    let cluster = ClusterComposition {
        binder: Arc::new(ClusterHubBinder::new(Arc::clone(hub), board.address)),
        timing: PresenceTiming::every_ms(period_ms),
        boot_epoch: fixed_boot_epoch(board.epoch),
    };
    let link = MockNetworkLink::new(board.mac, Some(board.address), "cluster-node");
    let ports = HttpTestPorts { network_link: Some(Arc::new(link)), cluster: Some(cluster), ..HttpTestPorts::default() };
    let (router, _hub, runtime) =
        build_http_test_stack("0.1.0-test", wire, BusConfig::default(), DaliRuntimeConfig::default(), None, &[], ports);
    (router, runtime)
}
