use std::collections::HashMap;

use dali2rust_api::http::handler::ApiHandler;
use dali2rust_api::http::handlers::redundancy::RedundancyConfigChangedHandler;
use dali2rust_bus::{BusConfig, BusHost, BusId};
use dali2rust_test_support::hold_the_events_ingress_full;

const PATH: &str = "/api/v1/redundancy/config-changed";
const WATCHED: &[&str] = &["PeerConfigChangedEvent"];

#[test]
fn a_wake_the_bus_refuses_is_the_overload_answer() {
    let (host, publisher, _rx) =
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_events(4, WATCHED));
    let held = hold_the_events_ingress_full(&publisher);
    let response = RedundancyConfigChangedHandler::new(publisher.clone(), BusId(1))
        .handle_request("POST", PATH, b"", &HashMap::new());
    held.release();
    assert_eq!(response.status, 503);
    drop(host);
}
