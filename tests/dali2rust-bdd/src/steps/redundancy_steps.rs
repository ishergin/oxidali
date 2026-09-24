use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_test_support::wait_until;

use crate::steps::physical_devices_steps::fetch_json;
use crate::steps::wire::{frame_claim, FrameClaim};
use crate::DaliWorld;

const TRANSITION_TIMEOUT: Duration = Duration::from_secs(4);

fn scalar_text(value: &serde_json::Value) -> String {
    match value.as_str() {
        Some(s) => s.to_string(),
        None => value.to_string(),
    }
}

// RED-026 RED-027 CFG-009 CFG-010
#[then(regex = r#"^the JSON pointer "([^"]*)" at "([^"]+)" should eventually be "([^"]*)"$"#)]
async fn json_pointer_eventually(world: &mut DaliWorld, pointer: String, path: String, expected: String) {
    let port = world.server_port();
    let read = || fetch_json(port, &path).and_then(|json| json.pointer(&pointer).cloned());
    wait_until(
        || read().map(|v| scalar_text(&v)).as_deref() == Some(expected.as_str()),
        TRANSITION_TIMEOUT,
    );
    let found = read();
    assert_eq!(
        found.as_ref().map(scalar_text).as_deref(),
        Some(expected.as_str()),
        "{path}{pointer}: {found:?}"
    );
}

// CFG-009 CFG-010
#[when(regex = r#"^I export "([^"]+)" and keep the body$"#)]
async fn export_and_keep(world: &mut DaliWorld, path: String) {
    world.send_http_request("GET", &path, None, "");
    let response = world.last_response().expect("last response");
    assert_eq!(response.status, 200, "export of {path} failed: {}", response.status);
    world.kept_body = response.body.clone();
    assert!(!world.kept_body.is_empty(), "export of {path} was empty");
}

// CFG-009 CFG-010
#[when(regex = r#"^I PUT the kept body to "([^"]+)"$"#)]
async fn put_kept_body(world: &mut DaliWorld, path: String) {
    let body = world.kept_body.clone();
    assert!(!body.is_empty(), "nothing was kept to import");
    world.send_http_request("PUT", &path, Some(&body), "application/octet-stream");
}


// RED-021 RED-023 RED-024
#[then(regex = r#"^the response header "([^"]+)" should be "([^"]*)"$"#)]
async fn response_header_is(world: &mut DaliWorld, name: String, expected: String) {
    let response = world.last_response().expect("last response");
    let wanted = name.to_lowercase();
    let found = response
        .headers
        .iter()
        .find(|(header, _)| *header == wanted)
        .map(|(_, value)| value.as_str());
    assert_eq!(
        found,
        Some(expected.as_str()),
        "header {name}: headers were {:?}",
        response.headers
    );
}

// RED-023 RED-024 RED-025
#[given("this controller has stood down")]
async fn controller_stands_down(world: &mut DaliWorld) {
    world.send_http_request(
        "PATCH",
        "/api/v1/settings/dali",
        Some(br#"{"application_active":false}"#),
        "application/json",
    );
    let status = world.last_response().expect("last response").status;
    assert_eq!(status, 200, "standing the controller down should succeed");
}

const ARBITRATION_PROBE: [u8; 3] = [0xFF, 0xFE, 0x3D];

// RED-028
#[then("the standby's arbitration probe should eventually be sent at priority 5")]
async fn probe_eventually_at_priority_five(world: &mut DaliWorld) {
    let mock = world.dali_mock().clone();
    let probe_index = || {
        mock.lock()
            .expect("mock lock")
            .sent_frames24()
            .iter()
            .position(|frame| *frame == ARBITRATION_PROBE)
    };
    wait_until(|| probe_index().is_some(), TRANSITION_TIMEOUT);
    let index = probe_index().expect("the standby never put its probe on the wire");
    let settle_us = mock.lock().expect("mock lock").sent_frame24_settle_us();
    let claim = frame_claim(index, settle_us[index]);
    assert!(
        matches!(claim, FrameClaim::Priority(5) | FrameClaim::BusRelease),
        "the probe announced {claim:?}, not priority 5 (DiiA 351 §7)"
    );
}
