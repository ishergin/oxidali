use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    BusCommandPayload, ClusterBirthMark, ClusterPartLink, ClusterPartUpdateCommand, ClusterRecordKind,
    ClusterRecordRef, ClusterSettingsCommitCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{CLUSTER_PART_LOCALS, MAX_ADAPTERS};
use serde_json::Value;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::cluster_parts_state::{ClusterPartDto, ClusterPartsHttpState};
use crate::http::cluster_settings_state::ClusterSettingsApplyWatch;
use crate::http::dispatcher::{publish_batch_and_wait_for_success, CorrelationIdAllocator};
use crate::http::handler::ApiHandler;
use crate::http::types::HttpResponse;

use super::common::{
    json_err, json_stream_dto, parse_body_object, parse_resource_id_param, wait_apply_counter,
    write_json_array_items, APPLY_WATCH_BUDGET_MS,
};

type JsonObject = serde_json::Map<String, Value>;
type Answer = Result<HttpResponse, HttpResponse>;

const ONE_COMMIT: u32 = 1;
const ADAPTER_PARAM: &str = "adapter_id";

const PUT_KEYS: &[&str] = &["record", "send", "accept"];
const PUT_READ_ONLY: &[&str] = &["adapter", "group", "scene", "revision"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterPartRoute {
    List,
    Get,
    Put,
    Delete,
}

impl ClusterPartRoute {
    const fn method(self) -> &'static str {
        match self {
            Self::List | Self::Get => "GET",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

pub struct ClusterPartDeps {
    pub publisher: BusPublisher,
    pub slots: Arc<PendingConfirmationSlots>,
    pub correlation: Arc<CorrelationIdAllocator>,
    pub state: Arc<dyn ClusterPartsHttpState>,
    pub apply_watch: Arc<dyn ClusterSettingsApplyWatch>,
    pub bus_id: BusId,
    pub timeout_ms: u64,
}

pub struct ClusterPartHandler {
    deps: Arc<ClusterPartDeps>,
    kind: ClusterRecordKind,
    route: ClusterPartRoute,
}

impl ClusterPartHandler {
    pub const fn new(deps: Arc<ClusterPartDeps>, kind: ClusterRecordKind, route: ClusterPartRoute) -> Self {
        Self { deps, kind, route }
    }
}

impl ApiHandler for ClusterPartHandler {
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
        let answer = match self.route {
            ClusterPartRoute::List => Ok(self.list()),
            ClusterPartRoute::Get => self.get(params),
            ClusterPartRoute::Put => self.put(params, body),
            ClusterPartRoute::Delete => self.delete(params),
        };
        answer.unwrap_or_else(|refusal| refusal)
    }
}

impl ClusterPartHandler {
    fn list(&self) -> HttpResponse {
        let (kind, state) = (self.kind, Arc::clone(&self.deps.state));
        HttpResponse::json_stream(
            200,
            Box::new(move |w: &mut dyn Write| {
                write!(w, "{{\"{}\":[", list_key(kind))?;
                let keys = state.cluster_part_keys(kind);
                let parts = keys.iter().filter_map(|(adapter, local)| state.cluster_part_dto(kind, adapter, local));
                write_json_array_items(w, parts)?;
                w.write_all(b"]}")
            }),
        )
    }

    fn get(&self, params: &HashMap<String, String>) -> Answer {
        let (adapter, local) = self.path_key(params)?;
        Ok(json_stream_dto(self.held(adapter, local)?))
    }

    fn put(&self, params: &HashMap<String, String>, body: &[u8]) -> Answer {
        let (adapter, local) = self.path_key(params)?;
        let link = part_link(&parse_body_object(body, PUT_KEYS, PUT_READ_ONLY)?)?;
        self.commit(adapter, local, Some(link))?;
        Ok(json_stream_dto(self.held(adapter, local)?))
    }

    fn delete(&self, params: &HashMap<String, String>) -> Answer {
        let (adapter, local) = self.path_key(params)?;
        self.commit(adapter, local, None)?;
        Ok(HttpResponse::no_content())
    }

    fn path_key(&self, params: &HashMap<String, String>) -> Result<(u8, u8), HttpResponse> {
        let adapter = parse_resource_id_param(params, ADAPTER_PARAM, u16::from(MAX_ADAPTERS) - 1)?;
        let local = parse_resource_id_param(params, local_param(self.kind), u16::from(CLUSTER_PART_LOCALS) - 1)?;
        Ok((adapter, local))
    }

    fn held(&self, adapter: u8, local: u8) -> Result<ClusterPartDto, HttpResponse> {
        self.deps.state.cluster_part_dto(self.kind, adapter, local).ok_or_else(|| json_err(404, "not_found"))
    }

    fn commit(&self, adapter: u8, local: u8, link: Option<ClusterPartLink>) -> Result<(), HttpResponse> {
        let deps = &self.deps;
        let before = deps.apply_watch.cluster_settings_applied_load();
        let batch = self.batch(adapter, local, link);
        publish_batch_and_wait_for_success(&deps.publisher, &deps.slots, deps.timeout_ms, batch)?;
        let watch = Arc::clone(&deps.apply_watch);
        let budget = APPLY_WATCH_BUDGET_MS.min(deps.timeout_ms);
        wait_apply_counter(move || watch.cluster_settings_applied_load(), before, ONE_COMMIT, budget)
    }

    fn batch(&self, adapter: u8, local: u8, link: Option<ClusterPartLink>) -> Vec<(u64, BusFrame)> {
        let series = self.deps.correlation.next_id();
        let part = ClusterPartUpdateCommand { series, kind: self.kind, adapter, local, link };
        let commit = ClusterSettingsCommitCommand { series, frames: 1 };
        vec![self.frame(self.deps.correlation.next_id(), part), self.frame(series, commit)]
    }

    fn frame(&self, corr: u64, payload: impl Into<BusCommandPayload>) -> (u64, BusFrame) {
        let bus = self.deps.bus_id.0;
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, bus, None, payload);
        (corr, BusFrame::command(cmd))
    }
}

const fn list_key(kind: ClusterRecordKind) -> &'static str {
    match kind {
        ClusterRecordKind::Channel => "channel_parts",
        ClusterRecordKind::Scene => "scene_parts",
    }
}

const fn local_param(kind: ClusterRecordKind) -> &'static str {
    match kind {
        ClusterRecordKind::Channel => "group_id",
        ClusterRecordKind::Scene => "scene_id",
    }
}

fn invalid() -> HttpResponse {
    json_err(422, "invalid_value")
}

fn field<'a>(obj: &'a JsonObject, key: &str) -> Result<&'a Value, HttpResponse> {
    obj.get(key).ok_or_else(invalid)
}

fn uint<T: TryFrom<u64>>(obj: &JsonObject, key: &str) -> Result<T, HttpResponse> {
    field(obj, key)?.as_u64().and_then(|n| T::try_from(n).ok()).ok_or_else(invalid)
}

fn flag(obj: &JsonObject, key: &str) -> Result<bool, HttpResponse> {
    field(obj, key)?.as_bool().ok_or_else(invalid)
}

fn object<'a>(obj: &'a JsonObject, key: &str) -> Result<&'a JsonObject, HttpResponse> {
    field(obj, key)?.as_object().ok_or_else(invalid)
}

fn record_ref(obj: &JsonObject) -> Result<ClusterRecordRef, HttpResponse> {
    let mark = object(obj, "birth_mark")?;
    let mark = ClusterBirthMark { generation: uint(mark, "generation")?, nonce: uint(mark, "nonce")? };
    Ok(ClusterRecordRef { number: uint(obj, "number")?, mark })
}

fn part_link(obj: &JsonObject) -> Result<ClusterPartLink, HttpResponse> {
    let record = record_ref(object(obj, "record")?)?;
    Ok(ClusterPartLink { record, send: flag(obj, "send")?, accept: flag(obj, "accept")? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(value: Value) -> Option<ClusterPartLink> {
        part_link(value.as_object().expect("object")).ok()
    }

    fn body(record: Value) -> Value {
        serde_json::json!({ "record": record, "send": true, "accept": false })
    }

    #[test]
    fn a_link_takes_every_field_from_its_own_key() {
        let record = serde_json::json!({ "number": 40, "birth_mark": { "generation": 3, "nonce": u32::MAX } });
        let mark = ClusterBirthMark { generation: 3, nonce: u32::MAX };
        let expected = ClusterPartLink { record: ClusterRecordRef { number: 40, mark }, send: true, accept: false };
        assert_eq!(link(body(record)), Some(expected));
    }

    #[test]
    fn a_link_refuses_a_missing_or_mistyped_field() {
        let mark = serde_json::json!({ "generation": 1, "nonce": 5 });
        let refused = [
            body(serde_json::json!({ "birth_mark": mark })),
            body(serde_json::json!({ "number": 256, "birth_mark": mark })),
            body(serde_json::json!({ "number": 1, "birth_mark": { "generation": 1 } })),
            body(serde_json::json!({ "number": 1, "birth_mark": { "generation": -1, "nonce": 5 } })),
            body(serde_json::json!({ "number": 1, "birth_mark": { "generation": 1, "nonce": 1_u64 << 32 } })),
            body(serde_json::json!({ "number": 1, "birth_mark": 7 })),
            serde_json::json!({ "record": { "number": 1, "birth_mark": mark }, "send": 1, "accept": false }),
            serde_json::json!({ "record": { "number": 1, "birth_mark": mark }, "send": true }),
        ];
        refused.into_iter().for_each(|value| assert_eq!(link(value.clone()), None, "{value}"));
    }

    #[test]
    fn each_route_answers_its_own_method() {
        let routes = [ClusterPartRoute::List, ClusterPartRoute::Get, ClusterPartRoute::Put, ClusterPartRoute::Delete];
        assert_eq!(routes.map(ClusterPartRoute::method), ["GET", "GET", "PUT", "DELETE"]);
    }

    #[test]
    fn each_kind_has_its_own_list_key_and_path_parameter() {
        let kinds = [ClusterRecordKind::Channel, ClusterRecordKind::Scene];
        assert_eq!(kinds.map(list_key), ["channel_parts", "scene_parts"]);
        assert_eq!(kinds.map(local_param), ["group_id", "scene_id"]);
    }
}
