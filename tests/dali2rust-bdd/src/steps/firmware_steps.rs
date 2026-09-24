use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::last_json;
use crate::steps::physical_devices_steps::fetch_json;
use crate::DaliWorld;

const UPDATE_TIMEOUT: Duration = Duration::from_secs(10);

const IMAGE_FILL: u8 = 0xA5;

fn serve_one_image(size: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind image server");
    let port = listener.local_addr().expect("image server addr").port();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request);
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n"
        );
        if stream.write_all(header.as_bytes()).is_ok() {
            let _ = stream.write_all(&vec![IMAGE_FILL; size]);
        }
    });
    format!("http://127.0.0.1:{port}/dali2rust.bin")
}

fn firmware_state(world: &DaliWorld) -> Value {
    fetch_json(world.server_port(), "/api/v1/firmware").expect("firmware read model")
}

fn operation_status(world: &DaliWorld, id: &str) -> Option<String> {
    fetch_json(world.server_port(), &format!("/api/v1/operations/{id}"))?
        .get("status")?
        .as_str()
        .map(str::to_string)
}

// SYS-243
#[given(regex = r"^a firmware image server offering (\d+) bytes$")]
async fn given_image_server(world: &mut DaliWorld, size: usize) {
    world.firmware_image_url = Some(serve_one_image(size));
}

// SYS-245
#[given(regex = r"^a firmware image URL nothing is listening on$")]
async fn given_dead_image_server(world: &mut DaliWorld) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("probe bind");
    let port = listener.local_addr().expect("probe addr").port();
    drop(listener);
    world.firmware_image_url = Some(format!("http://127.0.0.1:{port}/gone.bin"));
}

// SYS-243 SYS-245
#[when(regex = r"^I request a firmware update from that URL$")]
async fn when_update_requested(world: &mut DaliWorld) {
    let url = world
        .firmware_image_url
        .clone()
        .expect("no image URL — the scenario is missing its Given");
    let body = serde_json::json!({ "url": url }).to_string();
    world.send_http_request(
        "POST",
        "/api/v1/firmware/updates",
        Some(body.as_bytes()),
        "application/json",
    );
}

// SYS-243 SYS-245
#[then(regex = r#"^the firmware operation should end as "([^"]+)"$"#)]
async fn then_operation_ends_as(world: &mut DaliWorld, expected: String) {
    let id = last_json(world)
        .get("operation_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .expect("the 202 must name the operation");
    wait_until(
        || operation_status(world, &id).as_deref() == Some(expected.as_str()),
        UPDATE_TIMEOUT,
    );
}

// SYS-242 SYS-243 SYS-245
#[then(regex = r#"^the firmware update state should be "([^"]+)"$"#)]
async fn then_update_state_is(world: &mut DaliWorld, expected: String) {
    wait_until(
        || {
            firmware_state(world)
                .pointer("/update/state")
                .and_then(Value::as_str)
                == Some(expected.as_str())
        },
        UPDATE_TIMEOUT,
    );
}

// SYS-243
#[then(regex = r"^the firmware update should have written (\d+) bytes$")]
async fn then_written_bytes(world: &mut DaliWorld, expected: u64) {
    let written = firmware_state(world)
        .pointer("/update/downloaded_bytes")
        .and_then(Value::as_u64);
    assert_eq!(
        written,
        Some(expected),
        "the slot must receive exactly what the server served"
    );
}

// SYS-245
#[then(regex = r#"^the firmware update error should be "([^"]+)"$"#)]
async fn then_update_error_is(world: &mut DaliWorld, expected: String) {
    let error = firmware_state(world)
        .pointer("/update/error")
        .and_then(Value::as_str)
        .map(str::to_string);
    assert_eq!(error.as_deref(), Some(expected.as_str()));
}

// SYS-242
#[then(regex = r"^the controller should report it can update itself$")]
async fn then_ota_capable(world: &mut DaliWorld) {
    let capable = firmware_state(world)
        .get("ota_capable")
        .and_then(Value::as_bool);
    assert_eq!(capable, Some(true), "the host stack wires the simulated slot");
}
