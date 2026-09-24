use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dali2rust_api::http::adapter_state::{
    AdapterCountersDto, AdapterDto, AdapterHttpState, AdapterLimitsDto,
};
use dali2rust_api::http::dispatcher::CorrelationIdAllocator;
use dali2rust_api::http::handler::ApiHandler;
use dali2rust_api::http::handlers::scenes::{SceneApplyHandler, SceneMatrixWriteHandler};
use dali2rust_api::http::scene_state::{SceneDto, SceneHttpState, SceneMatrixDto};
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId};
use dali2rust_contracts::msg::{DaliSceneTargetState, OperationType, PowerState};
use dali2rust_domain::registry::{
    OperationReadPort, OperationView, SceneApplyRowView, SceneApplySnapshot,
};
use serde_json::Value;

fn params(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn base_adapter() -> AdapterDto {
    AdapterDto {
        adapter_id: 0,
        name: "adapter-0".to_string(),
        enabled: true,
        limits: AdapterLimitsDto {
            virtual_lamps: 64,
            groups: 16,
            scenes: 16,
        },
        bus_status: "ok".to_string(),
        counters: AdapterCountersDto {
            commands: 0,
            timeouts: 0,
            errors: 0,
        },
    }
}

fn dirty_snapshot() -> SceneApplySnapshot {
    SceneApplySnapshot {
        adapter_id: 0,
        scene_id: 3,
        rows: vec![SceneApplyRowView {
            virtual_lamp_id: 1,
            desired_included: true,
            desired_target: Some(DaliSceneTargetState {
                power: Some(PowerState::On),
                level: Some(100),
                color: None,
            }),
            applied_included: false,
            applied_target: None,
            binding_short: Some(0),
        }],
    }
}

struct TestSceneState;

impl AdapterHttpState for TestSceneState {
    fn adapter_count(&self) -> u8 {
        1
    }

    fn adapter_dto(&self, id: u8) -> Option<AdapterDto> {
        (id == 0).then(base_adapter)
    }

    fn list_adapter_dtos(&self) -> Vec<AdapterDto> {
        vec![base_adapter()]
    }
}

impl SceneHttpState for TestSceneState {
    fn scene_dto(&self, _adapter_id: u8, _scene_id: u8) -> Option<SceneDto> {
        None
    }

    fn list_scene_dtos(&self, _adapter_id: u8) -> Vec<SceneDto> {
        Vec::new()
    }

    fn scene_matrix_dto(&self, adapter_id: u8, scene_id: u8) -> Option<SceneMatrixDto> {
        Some(SceneMatrixDto {
            adapter_id,
            scene_id,
            rows: Vec::new(),
        })
    }

    fn scene_apply_snapshot(&self, _adapter_id: u8, _scene_id: u8) -> Option<SceneApplySnapshot> {
        Some(dirty_snapshot())
    }
}

struct NoOperations;

impl OperationReadPort for NoOperations {
    fn operation_status(&self, _operation_id: &str) -> Option<std::borrow::Cow<'static, str>> {
        None
    }

    fn operation_view(&self, _operation_id: &str) -> Option<OperationView> {
        None
    }

    fn list_operation_keys(&self) -> Vec<String> {
        Vec::new()
    }

    fn has_active_operation(&self, _operation_type: OperationType, _adapter_id: u8) -> bool {
        false
    }
}

#[test]
fn scene_matrix_replace_publishes_the_whole_series_without_waiting() {
    let (_host, publisher, cmd_rx) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_commands(64, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES)
    });

    let handler = SceneMatrixWriteHandler::put(
        publisher,
        Arc::new(CorrelationIdAllocator::new()),
        Arc::new(TestSceneState),
        BusId::default(),
    );

    let rows: Vec<Value> = (0u8..64)
        .map(|vl| serde_json::json!({ "virtual_lamp_id": vl, "desired": { "included": false } }))
        .collect();
    let body = serde_json::to_vec(&serde_json::json!({ "rows": rows })).expect("matrix put body");

    let started = std::time::Instant::now();
    let response = handler.handle_request(
        "PUT",
        "/matrix",
        &body,
        &params(&[("adapter_id", "0"), ("scene_id", "3")]),
    );
    let elapsed = started.elapsed();

    assert_eq!(response.status, 202, "a chunked write is accepted, not completed");
    let json: Value = serde_json::from_slice(&response.into_body_bytes()).expect("json body");
    assert_eq!(json["type"].as_str(), Some("config_write"));
    assert_eq!(json["status"].as_str(), Some("accepted"));
    let operation_id = json["operation_id"].as_str().expect("operation id");
    assert!(
        operation_id.starts_with("cfg-scn-0-3-"),
        "operation key must name the resource, got {operation_id}"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(200),
        "handler took {elapsed:?}; it must not wait for the bus at all"
    );

    let mut published = Vec::new();
    while let Ok(frame) = dali2rust_bus::Receiver::recv_timeout(&cmd_rx, 100) {
        if let BusFrame::Command(ce) = frame {
            published.push(ce);
        }
    }
    assert_eq!(
        published.len(),
        18,
        "expected begin + 16 chunks + commit bracket, got {}",
        published.len()
    );
    match &published[0].payload {
        dali2rust_contracts::msg::BusCommandPayload::OperationBeginCommand(begin) => {
            assert_eq!(begin.operation_type, OperationType::ConfigWrite);
            assert_eq!(begin.ttl_ms, 10_000);
        }
        other => panic!("series must open with OperationBegin, got {other:?}"),
    }
    let last = published.last().expect("bracket frame");
    match &last.payload {
        dali2rust_contracts::msg::BusCommandPayload::ConfigWriteCommitCommand(commit) => {
            assert_eq!(commit.chunks, 16);
            assert_eq!(commit.scene_id, Some(3));
        }
        other => panic!("expected the series to end with a commit bracket, got {other:?}"),
    }
    assert_eq!(
        last.meta.correlation_id, published[0].meta.correlation_id,
        "the bracket's correlation id is the operation's workflow id"
    );
}

#[test]
fn scene_apply_maps_full_commands_ingress_to_503() {
    let config = BusConfig {
        commands_ingress: 1,
        ..BusConfig::default()
    };
    let (_host, publisher, _cmd_rx) = BusHost::spawn(config, |reg| {
        reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES)
    });

    let handler = SceneApplyHandler::new(
        publisher.clone(),
        Arc::new(CorrelationIdAllocator::new()),
        Arc::new(TestSceneState),
        Arc::new(NoOperations),
        BusId::default(),
    );

    let stop = Arc::new(AtomicBool::new(false));
    let flood_stop = Arc::clone(&stop);
    let flood_publisher = publisher.clone();
    let flooder = std::thread::spawn(move || {
        let frame = BusFrame::command(dali2rust_contracts::bus::command_envelope(
            0,
            1,
            BusId::default().0,
            None,
            dali2rust_contracts::msg::DaliCommandPayload {
                wire_address: 0,
                command: 0xFE,
                repeat_count: 1,
                raw_mode: false,
                raw_expects_backward: false,
            },
        ));
        while !flood_stop.load(Ordering::Relaxed) {
            let _ = flood_publisher.try_publish(BusChannel::Commands, frame.clone());
        }
    });

    let mut saw_overload = false;
    for _ in 0..500 {
        let response =
            handler.handle_request("POST", "/apply", &[], &params(&[("adapter_id", "0"), ("scene_id", "3")]));
        match response.status {
            503 => {
                let body: Value =
                    serde_json::from_slice(&response.into_body_bytes()).expect("json body");
                assert_eq!(body["error"].as_str(), Some("commands_ingress_overload"));
                saw_overload = true;
                break;
            }
            202 => {}
            other => panic!("unexpected apply status: {other}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    flooder.join().expect("flooder join");
    assert!(
        saw_overload,
        "expected at least one 503 commands_ingress_overload under a saturated capacity-1 ingress"
    );
}
