use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    ClusterAction, ClusterBirthMark, ClusterCommandCommand, ClusterLightVerb, ClusterRecallScope, ClusterRecordKind,
    ClusterRecordRef, ClusterSourceClass, ClusterTarget, LevelPreset, LevelTransition, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::CLUSTER_RECORD_NUMBER_COUNT;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::cluster_info_state::{ClusterInfoHttpState, ClusterView};
use crate::http::dispatcher::{dispatch_and_wait_for_success, CorrelationIdAllocator};
use crate::http::handler::ApiHandler;
use crate::http::target_state_request::{parse_group_target_request, GroupTargetRequest, TargetRequest, TargetStateBody};
use crate::http::types::HttpResponse;

use super::common::{json_err, parse_body_object, parse_resource_id_param, parse_typed_body, serialize_or_log};

const NUMBER_PARAM: &str = "number";
const CHANNEL_KEY: &str = "channel";
const RECALL_KEYS: &[&str] = &[CHANNEL_KEY];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterCommandRoute {
    ChannelTargetState,
    SceneRecall,
}

impl ClusterCommandRoute {
    const fn method(self) -> &'static str {
        match self {
            Self::ChannelTargetState => "PUT",
            Self::SceneRecall => "POST",
        }
    }
}

pub struct ClusterCommandDeps {
    pub publisher: BusPublisher,
    pub slots: Arc<PendingConfirmationSlots>,
    pub correlation: Arc<CorrelationIdAllocator>,
    pub cluster: Arc<dyn ClusterInfoHttpState>,
    pub bus_id: BusId,
    pub timeout_ms: u64,
}

pub struct ClusterCommandHandler {
    deps: Arc<ClusterCommandDeps>,
    route: ClusterCommandRoute,
}

impl ClusterCommandHandler {
    pub const fn new(deps: Arc<ClusterCommandDeps>, route: ClusterCommandRoute) -> Self {
        Self { deps, route }
    }
}

impl ApiHandler for ClusterCommandHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if method != self.route.method() {
            return HttpResponse::method_not_allowed();
        }
        let action = match self.route {
            ClusterCommandRoute::ChannelTargetState => self.channel_action(params, body),
            ClusterCommandRoute::SceneRecall => self.recall_action(params, body),
        };
        action.and_then(|action| self.publish(action)).unwrap_or_else(|refusal| refusal)
    }
}

impl ClusterCommandHandler {
    fn channel_action(&self, params: &HashMap<String, String>, body: &[u8]) -> Result<ClusterAction, HttpResponse> {
        let channel = self.known(ClusterRecordKind::Channel, path_number(params)?)?;
        let request = parse_group_target_request(&parse_typed_body::<TargetStateBody>(body)?, |_| true)?;
        Ok(ClusterAction::Light { target: ClusterTarget::Channel(channel), verb: light_verb(request) })
    }

    fn recall_action(&self, params: &HashMap<String, String>, body: &[u8]) -> Result<ClusterAction, HttpResponse> {
        let scene = self.known(ClusterRecordKind::Scene, path_number(params)?)?;
        let scope = match recall_channel(body)? {
            None => ClusterRecallScope::Node,
            Some(number) => ClusterRecallScope::Channel(self.known(ClusterRecordKind::Channel, number)?),
        };
        Ok(ClusterAction::RecallScene { scene, scope })
    }

    fn known(&self, kind: ClusterRecordKind, number: u8) -> Result<ClusterRecordRef, HttpResponse> {
        let record = ClusterView::read(self.deps.cluster.as_ref()).record(kind, number);
        let record = record.ok_or_else(|| json_err(404, "not_found"))?;
        let mark = ClusterBirthMark { generation: record.birth_mark.generation, nonce: record.birth_mark.nonce };
        Ok(ClusterRecordRef { number, mark })
    }

    fn publish(&self, action: ClusterAction) -> Result<HttpResponse, HttpResponse> {
        let deps = &self.deps;
        let correlation_id = deps.correlation.next_id();
        let command = ClusterCommandCommand { action, source: ClusterSourceClass::Operator, hold_hcl: true };
        let envelope = command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, deps.bus_id.0, Some(Origin::Api), command);
        let frame = BusFrame::command(envelope);
        dispatch_and_wait_for_success(&deps.publisher, &deps.slots, correlation_id, deps.timeout_ms, frame)?;
        let body = serde_json::json!({ "correlation_id": correlation_id, "status": "sent" });
        Ok(HttpResponse::json(200, serialize_or_log(&body)))
    }
}

fn path_number(params: &HashMap<String, String>) -> Result<u8, HttpResponse> {
    parse_resource_id_param(params, NUMBER_PARAM, u16::from(CLUSTER_RECORD_NUMBER_COUNT) - 1)
}

fn recall_channel(body: &[u8]) -> Result<Option<u8>, HttpResponse> {
    let object = parse_body_object(body, RECALL_KEYS, &[])?;
    let Some(value) = object.get(CHANNEL_KEY) else {
        return Ok(None);
    };
    let number = value.as_u64().filter(|number| *number < u64::from(CLUSTER_RECORD_NUMBER_COUNT));
    number.and_then(|number| u8::try_from(number).ok()).map(Some).ok_or_else(|| json_err(422, "invalid_value"))
}

const fn light_verb(request: GroupTargetRequest) -> ClusterLightVerb {
    match request {
        GroupTargetRequest::Target(TargetRequest::Setpoint(setpoint)) => ClusterLightVerb::Setpoint(setpoint),
        GroupTargetRequest::Target(TargetRequest::Recall(LevelPreset::MaxLevel)) => {
            ClusterLightVerb::Transition(LevelTransition::RecallMaxLevel)
        }
        GroupTargetRequest::Target(TargetRequest::Recall(LevelPreset::MinLevel)) => {
            ClusterLightVerb::Transition(LevelTransition::RecallMinLevel)
        }
        GroupTargetRequest::Relative(verb) => verb.light_verb(),
    }
}
