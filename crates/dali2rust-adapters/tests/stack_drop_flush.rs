use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_adapters::dali::MockDaliTransport;
use dali2rust_adapters::{
    build_http_test_stack, BusStackRuntime, DaliRuntimeConfig, HttpTestPorts,
};
use dali2rust_api::http::router::Router;
use dali2rust_bsp::slice_store_files::InMemorySliceStore;
use dali2rust_bus::BusConfig;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_test_support::HeldSliceWrite;

const HOLD: Duration = Duration::from_secs(2);
const DISPLAY_SETTINGS: &str = "/api/v1/settings/display";
const UPSIDE_DOWN: &[u8] = br#"{"rotation_deg":180}"#;
const READS_UPSIDE_DOWN: &str = r#""rotation_deg":180"#;
const OK: u16 = 200;

fn stack_on(slices: &Arc<HeldSliceWrite>) -> (Router, Box<BusStackRuntime>) {
    let store: Arc<dyn SliceStore> = slices.clone();
    let (router, _hub, runtime) = build_http_test_stack(
        "0.1.0-test",
        Arc::new(Mutex::new(MockDaliTransport::new())),
        BusConfig::default(),
        DaliRuntimeConfig::default(),
        Some(store),
        &[],
        HttpTestPorts::default(),
    );
    (router, runtime)
}

#[test]
fn a_stack_built_after_a_drop_hydrates_what_the_dropped_one_confirmed() {
    let stored: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::default());
    let slices = Arc::new(HeldSliceWrite::over(stored, HOLD));
    let (router, runtime) = stack_on(&slices);
    slices.hold_next_write_of(SliceKey::DisplaySettings);
    assert_eq!(router.dispatch("PATCH", DISPLAY_SETTINGS, UPSIDE_DOWN).status, OK);
    drop(router);
    drop(runtime);
    let (router, _runtime) = stack_on(&slices);
    let body = router.dispatch("GET", DISPLAY_SETTINGS, &[]).into_body_bytes();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains(READS_UPSIDE_DOWN), "the next stack read {body}");
}
