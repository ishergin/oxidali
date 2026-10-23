use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_api::http::router::Router;
use dali2rust_bsp::slice_store_files::InMemorySliceStore;
use dali2rust_bus::BusConfig;
use dali2rust_dali_runtime::DaliRuntimeConfig;
use dali2rust_platform::slice_store::SliceStore;
use dali2rust_registry_runtime::{
    encode_persistence_blob, PersistableGroupRecord, PersistableGroupsSlice,
    PersistableVirtualLampsSlice, PersistableVlRecord, PersistenceEnvelope, GROUPS_SLICE_VERSION,
    VIRTUAL_LAMPS_SLICE_VERSION,
};
use dali2rust_test_support::{remains_false_for, wait_until};

use super::{build_http_test_stack, BusStackRuntime, HttpTestPorts};
use crate::dali::transport::mock::MockDaliTransport;

const ADAPTERS: u8 = 2;
const LAMPS_PER_ADAPTER: u8 = 64;
const GROUPS_PER_ADAPTER: u8 = 16;
const ANNOUNCED_TO_THE_HUB: u32 = 2 * (64 + 16 + 1);
const IMPORT_ACCEPTED_WITHIN: Duration = Duration::from_secs(10);
const DRAINED_WITHIN: Duration = Duration::from_secs(12);
const WS_FANOUT: &str = "ws_fanout";
const ACCEPTED: u16 = 202;

fn lamps_blob(adapter_id: u8) -> Vec<u8> {
    let lamp = |virtual_lamp_id| PersistableVlRecord {
        virtual_lamp_id,
        name: format!("lamp {virtual_lamp_id}"),
        ha_entity_enabled: true,
        binding_short: None,
    };
    let lamps = (0..LAMPS_PER_ADAPTER).map(lamp).collect();
    let slice = PersistableVirtualLampsSlice { adapter_id, lamps };
    encode_persistence_blob(&PersistenceEnvelope::new(VIRTUAL_LAMPS_SLICE_VERSION, slice))
        .expect("encode lamps")
}

fn groups_blob(adapter_id: u8) -> Vec<u8> {
    let group = |group_id| PersistableGroupRecord {
        group_id,
        name: format!("group {group_id}"),
        ha_entity_enabled: true,
    };
    let groups = (0..GROUPS_PER_ADAPTER).map(group).collect();
    let slice = PersistableGroupsSlice { adapter_id, groups, rows: Vec::new() };
    encode_persistence_blob(&PersistenceEnvelope::new(GROUPS_SLICE_VERSION, slice))
        .expect("encode groups")
}

fn import(router: &Router, name: &str, body: &[u8]) {
    let uri = format!("/api/v1/config/slices/{}", name.replace('/', "%2F"));
    let deadline = Instant::now() + IMPORT_ACCEPTED_WITHIN;
    let mut status = 0;
    while status != ACCEPTED && Instant::now() < deadline {
        status = router.dispatch("PUT", &uri, body).status;
    }
    assert_eq!(status, ACCEPTED, "the import of {name} was not accepted");
}

fn hub_inbox(runtime: &BusStackRuntime) -> (u32, u32) {
    let counters = runtime.event_publisher.counters_snapshot();
    let hub = counters.event_subscribers.iter().find(|s| s.name == WS_FANOUT);
    hub.map_or((0, 0), |s| (s.delivered, s.receiver_overflow))
}

#[test]
fn a_large_import_reaches_a_parked_websocket_hub_without_an_inbox_overflow() {
    let slices: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
    let mock = Arc::new(Mutex::new(MockDaliTransport::new()));
    let (router, _hub, runtime) = build_http_test_stack(
        "0.1.0-test",
        mock,
        BusConfig::default(),
        DaliRuntimeConfig::default(),
        Some(slices),
        &[],
        HttpTestPorts::default(),
    );
    let (delivered_before, _) = hub_inbox(&runtime);
    for adapter_id in 0..ADAPTERS {
        import(&router, &format!("a{adapter_id}/virtual_lamps"), &lamps_blob(adapter_id));
        import(&router, &format!("a{adapter_id}/groups"), &groups_blob(adapter_id));
    }
    let overflow = || hub_inbox(&runtime).1;
    let lossless = remains_false_for(|| overflow() > 0, DRAINED_WITHIN);
    assert!(lossless, "the parked hub's inbox overflowed {} times", overflow());
    let delivered = || hub_inbox(&runtime).0.wrapping_sub(delivered_before);
    wait_until(|| delivered() >= ANNOUNCED_TO_THE_HUB, IMPORT_ACCEPTED_WITHIN);
    assert!(delivered() >= ANNOUNCED_TO_THE_HUB, "only {} reached the hub", delivered());
}
