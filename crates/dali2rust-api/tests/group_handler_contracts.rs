use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_api::http::adapter_state::{AdapterCountersDto, AdapterDto, AdapterHttpState, AdapterLimitsDto};
use dali2rust_api::http::dispatcher::CorrelationIdAllocator;
use dali2rust_api::http::group_state::{
    GroupDto, GroupHttpState, GroupMatrixGroupDto, GroupMembershipMatrixDto, GroupMembershipMatrixRowDto,
};
use dali2rust_api::http::handler::ApiHandler;
use dali2rust_api::http::handlers::groups::GroupApplyHandler;
use dali2rust_api::http::types::HttpResponse;
use dali2rust_bus::{BusConfig, BusFrame, BusHost, BusId};
use dali2rust_contracts::msg::{BusCommandPayload, OperationType};
use dali2rust_domain::registry::{
    GroupApplyRowView, GroupApplySnapshot, OperationReadPort, OperationView,
};
use serde_json::{json, Value};

fn params(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn response_json(resp: HttpResponse) -> Value {
    serde_json::from_slice(&resp.into_body_bytes()).expect("json response")
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

fn matrix_with_rows(rows: Vec<GroupMembershipMatrixRowDto>) -> GroupMembershipMatrixDto {
    GroupMembershipMatrixDto {
        adapter_id: 0,
        groups: vec![
            GroupMatrixGroupDto {
                group_id: 0,
                name: "g0".to_string(),
                dirty: true,
            },
            GroupMatrixGroupDto {
                group_id: 1,
                name: "g1".to_string(),
                dirty: true,
            },
            GroupMatrixGroupDto {
                group_id: 2,
                name: "g2".to_string(),
                dirty: true,
            },
        ],
        rows,
        dirty: true,
    }
}

#[derive(Clone)]
struct TestGroupState {
    adapter: AdapterDto,
    matrix: GroupMembershipMatrixDto,
    snapshot: GroupApplySnapshot,
}

impl AdapterHttpState for TestGroupState {
    fn adapter_count(&self) -> u8 {
        1
    }

    fn adapter_dto(&self, id: u8) -> Option<AdapterDto> {
        (id == self.adapter.adapter_id).then(|| self.adapter.clone())
    }

    fn list_adapter_dtos(&self) -> Vec<AdapterDto> {
        vec![self.adapter.clone()]
    }
}

impl GroupHttpState for TestGroupState {
    fn group_dto(&self, _adapter_id: u8, _group_id: u8) -> Option<GroupDto> {
        None
    }

    fn list_group_dtos(&self, _adapter_id: u8) -> Vec<GroupDto> {
        vec![]
    }

    fn group_membership_matrix_dto(&self, adapter_id: u8) -> Option<GroupMembershipMatrixDto> {
        (adapter_id == self.matrix.adapter_id).then(|| self.matrix.clone())
    }

    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
        (adapter_id == self.snapshot.adapter_id).then(|| self.snapshot.clone())
    }
}

struct TestOperations {
    active_group_apply: bool,
}

impl OperationReadPort for TestOperations {
    fn operation_status(&self, _operation_id: &str) -> Option<std::borrow::Cow<'static, str>> {
        None
    }

    fn operation_view(&self, _operation_id: &str) -> Option<OperationView> {
        None
    }

    fn list_operation_keys(&self) -> Vec<String> {
        vec![]
    }

    fn has_active_operation(&self, operation_type: OperationType, _adapter_id: u8) -> bool {
        self.active_group_apply && operation_type == OperationType::GroupApply
    }
}

struct PublishProbe {
    _host: BusHost,
    publisher: dali2rust_bus::BusPublisher,
    commands: Receiver<BusFrame>,
    events: Receiver<BusFrame>,
}

impl PublishProbe {
    fn new() -> Self {
        let (host, publisher, (commands, events)) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                (
                    reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                    reg.subscribe_events(32, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
                )
            },
        );
        Self {
            _host: host,
            publisher,
            commands,
            events,
        }
    }
}

#[test]
fn group_apply_noop_returns_matrix_without_operation() {
    let harness = PublishProbe::new();
    let handler = GroupApplyHandler::new(
        harness.publisher.clone(),
        Arc::new(CorrelationIdAllocator::new()),
        Arc::new(TestGroupState {
            adapter: base_adapter(),
            matrix: matrix_with_rows(vec![GroupMembershipMatrixRowDto {
                virtual_lamp_id: 1,
                name: "vl-1".to_string(),
                desired: [false; 16],
                applied: [false; 16],
            }]),
            snapshot: GroupApplySnapshot {
                adapter_id: 0,
                rows: vec![GroupApplyRowView {
                    virtual_lamp_id: 1,
                    desired_groups_mask: 0,
                    applied_groups_mask: 0,
                    binding_short: Some(7),
                }],
            },
        }),
        Arc::new(TestOperations {
            active_group_apply: false,
        }),
        BusId::default(),
    );

    let response = handler.handle_request("POST", "/api/v1/adapters/0/groups/apply", &[], &params(&[("adapter_id", "0")]));
    assert_eq!(response.status, 200);
    assert_eq!(
        response_json(response),
        json!({
            "adapter_id": 0,
            "groups": [
                {"group_id": 0, "name": "g0", "dirty": true},
                {"group_id": 1, "name": "g1", "dirty": true},
                {"group_id": 2, "name": "g2", "dirty": true}
            ],
            "rows": [
                {
                    "virtual_lamp_id": 1,
                    "name": "vl-1",
                    "desired": [false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false],
                    "applied": [false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false]
                }
            ],
            "dirty": true
        })
    );
    assert!(harness.commands.recv_timeout(Duration::from_millis(50)).is_err());
    assert!(harness.events.recv_timeout(Duration::from_millis(50)).is_err());
}

#[test]
fn group_apply_publishes_one_execute_command_for_the_orchestrator() {
    let harness = PublishProbe::new();
    let handler = GroupApplyHandler::new(
        harness.publisher.clone(),
        Arc::new(CorrelationIdAllocator::new()),
        Arc::new(TestGroupState {
            adapter: base_adapter(),
            matrix: matrix_with_rows(vec![]),
            snapshot: GroupApplySnapshot {
                adapter_id: 0,
                rows: vec![
                    GroupApplyRowView {
                        virtual_lamp_id: 2,
                        desired_groups_mask: 1 << 1,
                        applied_groups_mask: 0,
                        binding_short: Some(12),
                    },
                    GroupApplyRowView {
                        virtual_lamp_id: 3,
                        desired_groups_mask: 1 << 2,
                        applied_groups_mask: 0,
                        binding_short: None,
                    },
                ],
            },
        }),
        Arc::new(TestOperations {
            active_group_apply: false,
        }),
        BusId::default(),
    );

    let response = handler.handle_request("POST", "/api/v1/adapters/0/groups/apply", &[], &params(&[("adapter_id", "0")]));
    assert_eq!(response.status, 202);
    let body = response_json(response);
    assert_eq!(body.get("type").and_then(Value::as_str), Some("group_apply"));
    assert_eq!(body.get("status").and_then(Value::as_str), Some("accepted"));
    let operation_id = body
        .get("operation_id")
        .and_then(Value::as_str)
        .expect("operation_id");
    let correlation_id = operation_id
        .rsplit('-')
        .next()
        .expect("workflow suffix")
        .parse::<u64>()
        .expect("workflow id");

    let BusFrame::Command(command) = harness
        .commands
        .recv_timeout(Duration::from_millis(200))
        .expect("execute command frame")
    else {
        panic!("expected command frame");
    };
    assert_eq!(command.meta.correlation_id, correlation_id);
    let BusCommandPayload::GroupApplyExecuteCommand(execute) = &command.payload else {
        panic!("expected GroupApplyExecuteCommand, got {:?}", command.payload);
    };
    assert_eq!(execute.registry_adapter_id, 0);
    assert_eq!(execute.operation_key.as_str(), operation_id);

    assert!(harness
        .commands
        .recv_timeout(Duration::from_millis(50))
        .is_err());
    assert!(harness
        .events
        .recv_timeout(Duration::from_millis(50))
        .is_err());
}

#[test]
fn group_apply_rejects_when_same_adapter_operation_is_active() {
    let harness = PublishProbe::new();
    let handler = GroupApplyHandler::new(
        harness.publisher.clone(),
        Arc::new(CorrelationIdAllocator::new()),
        Arc::new(TestGroupState {
            adapter: base_adapter(),
            matrix: matrix_with_rows(vec![]),
            snapshot: GroupApplySnapshot {
                adapter_id: 0,
                rows: vec![],
            },
        }),
        Arc::new(TestOperations {
            active_group_apply: true,
        }),
        BusId::default(),
    );

    let response = handler.handle_request("POST", "/api/v1/adapters/0/groups/apply", &[], &params(&[("adapter_id", "0")]));
    assert_eq!(response.status, 409);
    assert_eq!(response_json(response), json!({ "error": "conflict" }));
    assert!(harness.commands.recv_timeout(Duration::from_millis(50)).is_err());
}
