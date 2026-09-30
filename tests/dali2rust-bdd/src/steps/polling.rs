use std::cell::RefCell;
use std::time::{Duration, Instant};

use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::{last_json, response_json};
use crate::DaliWorld;

const OPERATION_TIMEOUT: Duration = Duration::from_secs(8);

fn operation_path_from_last_response(world: &DaliWorld) -> String {
    let response = last_json(world);
    let op_id = response
        .get("operation_id")
        .and_then(Value::as_str)
        .expect("operation_id in last response");
    format!("/api/v1/operations/{op_id}")
}

pub(crate) fn fetch_json(port: u16, path: &str) -> Option<Value> {
    let response = DaliWorld::send_http_request_raw(port, "GET", path, None, "");
    (response.status == 200).then(|| response_json(&response))
}

pub(crate) fn wait_for_operation_status(world: &mut DaliWorld, expected: &str) {
    wait_for_operation_status_within(world, expected, OPERATION_TIMEOUT);
}

pub(crate) fn wait_for_operation_status_within(
    world: &mut DaliWorld,
    expected: &str,
    budget: Duration,
) {
    let port = world.server_port();
    let path = operation_path_from_last_response(world);
    let start = Instant::now();
    let last_json = RefCell::new(None);
    wait_until(
        || {
            let fetched = fetch_json(port, &path);
            *last_json.borrow_mut() = fetched.clone();
            let status = fetched
                .as_ref()
                .and_then(|json| json.get("status"))
                .and_then(Value::as_str);
            matches!(status, Some("failed" | "timed_out" | "superseded"))
                || status == Some(expected)
                || start.elapsed() >= budget
        },
        budget + Duration::from_millis(50),
    );
    let last_json = last_json.into_inner();
    let status = last_json
        .as_ref()
        .and_then(|json| json.get("status"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if status != expected {
        let mock = world.dali_mock().lock().expect("mock lock");
        panic!(
            "operation did not reach expected status {expected}: last_status={status}; last_json={last_json:?}; script_error={:?}; frames={:?}",
            mock.script_error(),
            mock.sent_frames()
        );
    }
    world.send_http_request("GET", &path, None, "");
}
