use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    fixed_text_64, BusCommandPayload, ClusterRecordAction, ClusterRecordKind, ClusterRecordUpdateCommand,
    ClusterSettingsCommitCommand, ClusterTextField, ClusterTextUpdateCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{CLUSTER_RECORD_NUMBER_COUNT, MAX_CLUSTER_RECORD_NAME_BYTES};

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::cluster_info_state::{write_object_then, ClusterInfoHttpState, ClusterView};
use crate::http::cluster_records_state::{record_nonce, ClusterRecordDto, ClusterRecordsHttpState};
use crate::http::cluster_settings_state::ClusterSettingsApplyWatch;
use crate::http::dispatcher::{publish_batch_and_wait_for_success, CorrelationIdAllocator};
use crate::http::handler::ApiHandler;
use crate::http::types::HttpResponse;

use super::common::{
    json_err, json_err_with_message, json_stream_dto, parse_body_object, parse_resource_id_param, wait_apply_counter,
    write_json_array_items, APPLY_WATCH_BUDGET_MS,
};

type JsonObject = serde_json::Map<String, serde_json::Value>;
type Answer = Result<HttpResponse, HttpResponse>;

const ONE_COMMIT: u32 = 1;
const CREATED: u16 = 201;
const NUMBER_PARAM: &str = "number";

const CREATE_KEYS: &[&str] = &["name", "number"];
const CREATE_READ_ONLY: &[&str] = &["home_node", "birth_mark", "revision"];
const RENAME_KEYS: &[&str] = &["name", "revision"];
const DELETE_KEYS: &[&str] = &["revision"];
const EDIT_READ_ONLY: &[&str] = &["number", "home_node", "birth_mark"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterRecordRoute {
    List,
    Get,
    Create,
    Rename,
    Delete,
}

impl ClusterRecordRoute {
    const fn method(self) -> &'static str {
        match self {
            Self::List | Self::Get => "GET",
            Self::Create => "POST",
            Self::Rename => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

pub struct ClusterRecordDeps {
    pub publisher: BusPublisher,
    pub slots: Arc<PendingConfirmationSlots>,
    pub correlation: Arc<CorrelationIdAllocator>,
    pub state: Arc<dyn ClusterRecordsHttpState>,
    pub cluster: Arc<dyn ClusterInfoHttpState>,
    pub apply_watch: Arc<dyn ClusterSettingsApplyWatch>,
    pub bus_id: BusId,
    pub timeout_ms: u64,
}

pub struct ClusterRecordHandler {
    deps: Arc<ClusterRecordDeps>,
    kind: ClusterRecordKind,
    route: ClusterRecordRoute,
}

impl ClusterRecordHandler {
    pub const fn new(deps: Arc<ClusterRecordDeps>, kind: ClusterRecordKind, route: ClusterRecordRoute) -> Self {
        Self { deps, kind, route }
    }
}

impl ApiHandler for ClusterRecordHandler {
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
            ClusterRecordRoute::List => Ok(self.list()),
            ClusterRecordRoute::Get => self.get(params),
            ClusterRecordRoute::Create => self.create(body),
            ClusterRecordRoute::Rename => self.rename(params, body),
            ClusterRecordRoute::Delete => self.delete(params, body),
        };
        answer.unwrap_or_else(|refusal| refusal)
    }
}

impl ClusterRecordHandler {
    fn list(&self) -> HttpResponse {
        let (kind, cluster) = (self.kind, Arc::clone(&self.deps.cluster));
        HttpResponse::json_stream(
            200,
            Box::new(move |w: &mut dyn Write| {
                write!(w, "{{\"{}\":[", list_key(kind))?;
                write_json_array_items(w, ClusterView::read(cluster.as_ref()).records(kind))?;
                w.write_all(b"]}")
            }),
        )
    }

    fn get(&self, params: &HashMap<String, String>) -> Answer {
        let (kind, number, cluster) = (self.kind, path_number(params)?, Arc::clone(&self.deps.cluster));
        let record = ClusterView::read(cluster.as_ref()).record(kind, number).ok_or_else(|| json_err(404, "not_found"))?;
        Ok(HttpResponse::json_stream(
            200,
            Box::new(move |w: &mut dyn Write| {
                write_object_then(w, &record, "parts")?;
                w.write_all(b"[")?;
                write_json_array_items(w, ClusterView::read(cluster.as_ref()).parts_of(kind, &record))?;
                w.write_all(b"]}")
            }),
        ))
    }

    fn create(&self, body: &[u8]) -> Answer {
        let obj = parse_body_object(body, CREATE_KEYS, CREATE_READ_ONLY)?;
        let number = optional_number(&obj)?;
        let name = optional_name(&obj)?;
        let nonce = record_nonce();
        self.commit(ClusterRecordAction::Create { number, nonce }, name)?;
        let created = self.created(nonce).ok_or_else(|| json_err(404, "not_found"))?;
        let mut response = json_stream_dto(created);
        response.status = CREATED;
        Ok(response)
    }

    fn rename(&self, params: &HashMap<String, String>, body: &[u8]) -> Answer {
        let number = path_number(params)?;
        let obj = parse_body_object(body, RENAME_KEYS, EDIT_READ_ONLY)?;
        let name = required_name(&obj)?;
        let revision = required_revision(&obj)?;
        self.commit(ClusterRecordAction::Rename { number, revision }, Some(name))?;
        Ok(json_stream_dto(self.held(number)?))
    }

    fn delete(&self, params: &HashMap<String, String>, body: &[u8]) -> Answer {
        let number = path_number(params)?;
        let revision = required_revision(&parse_body_object(body, DELETE_KEYS, EDIT_READ_ONLY)?)?;
        self.commit(ClusterRecordAction::Delete { number, revision }, None)?;
        Ok(HttpResponse::no_content())
    }

    fn held(&self, number: u8) -> Result<ClusterRecordDto, HttpResponse> {
        self.deps.state.cluster_record_dto(self.kind, number).ok_or_else(|| json_err(404, "not_found"))
    }

    fn created(&self, nonce: u32) -> Option<ClusterRecordDto> {
        let state = &self.deps.state;
        let numbers = state.cluster_record_numbers(self.kind).iter();
        numbers.filter_map(|n| state.cluster_record_dto(self.kind, n)).find(|record| record.birth_mark.nonce == nonce)
    }

    fn commit(&self, action: ClusterRecordAction, name: Option<String>) -> Result<(), HttpResponse> {
        let deps = &self.deps;
        if let Some(refusal) = deps.cluster.record_write_refusal() {
            return Err(json_err_with_message(409, "conflict", refusal));
        }
        let before = deps.apply_watch.cluster_settings_applied_load();
        publish_batch_and_wait_for_success(&deps.publisher, &deps.slots, deps.timeout_ms, self.batch(action, name))?;
        let watch = Arc::clone(&deps.apply_watch);
        let budget = APPLY_WATCH_BUDGET_MS.min(deps.timeout_ms);
        wait_apply_counter(move || watch.cluster_settings_applied_load(), before, ONE_COMMIT, budget)
    }

    fn batch(&self, action: ClusterRecordAction, name: Option<String>) -> Vec<(u64, BusFrame)> {
        let series = self.deps.correlation.next_id();
        let record = ClusterRecordUpdateCommand { series, kind: self.kind, action };
        let text = name.map(|name| ClusterTextUpdateCommand {
            series,
            field: ClusterTextField::RecordName,
            text: fixed_text_64(&name),
        });
        let mut batch: Vec<(u64, BusFrame)> = core::iter::once(BusCommandPayload::from(record))
            .chain(text.map(BusCommandPayload::from))
            .map(|payload| self.frame(self.deps.correlation.next_id(), payload))
            .collect();
        let frames = u8::try_from(batch.len()).unwrap_or(u8::MAX);
        batch.push(self.frame(series, ClusterSettingsCommitCommand { series, frames }));
        batch
    }

    fn frame(&self, corr: u64, payload: impl Into<BusCommandPayload>) -> (u64, BusFrame) {
        let bus = self.deps.bus_id.0;
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, bus, None, payload);
        (corr, BusFrame::command(cmd))
    }
}

const fn list_key(kind: ClusterRecordKind) -> &'static str {
    match kind {
        ClusterRecordKind::Channel => "channels",
        ClusterRecordKind::Scene => "scenes",
    }
}

fn invalid() -> HttpResponse {
    json_err(422, "invalid_value")
}

fn path_number(params: &HashMap<String, String>) -> Result<u8, HttpResponse> {
    parse_resource_id_param(params, NUMBER_PARAM, u16::from(CLUSTER_RECORD_NUMBER_COUNT) - 1)
}

fn optional_number(obj: &JsonObject) -> Result<Option<u8>, HttpResponse> {
    obj.get("number")
        .map(|value| {
            let number = value.as_u64().filter(|n| *n < u64::from(CLUSTER_RECORD_NUMBER_COUNT));
            number.and_then(|n| u8::try_from(n).ok()).ok_or_else(invalid)
        })
        .transpose()
}

fn optional_name(obj: &JsonObject) -> Result<Option<String>, HttpResponse> {
    obj.get("name")
        .map(|value| {
            let name = value.as_str().filter(|name| name.len() <= MAX_CLUSTER_RECORD_NAME_BYTES);
            name.map(str::to_owned).ok_or_else(invalid)
        })
        .transpose()
}

fn required_name(obj: &JsonObject) -> Result<String, HttpResponse> {
    optional_name(obj)?.filter(|name| !name.is_empty()).ok_or_else(invalid)
}

fn required_revision(obj: &JsonObject) -> Result<u32, HttpResponse> {
    let revision = obj.get("revision").and_then(serde_json::Value::as_u64);
    revision.and_then(|r| u32::try_from(r).ok()).ok_or_else(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(value: serde_json::Value) -> JsonObject {
        value.as_object().expect("object").clone()
    }

    #[test]
    fn a_number_is_kept_up_to_the_last_record_number_and_refused_past_it() {
        let last = u64::from(CLUSTER_RECORD_NUMBER_COUNT) - 1;
        assert_eq!(optional_number(&object(serde_json::json!({ "number": last }))).ok(), Some(Some(127)));
        assert!(optional_number(&object(serde_json::json!({ "number": last + 1 }))).is_err());
        assert_eq!(optional_number(&object(serde_json::json!({}))).ok(), Some(None));
    }

    #[test]
    fn a_name_of_the_full_length_is_kept_and_one_byte_more_refused() {
        let full = "n".repeat(MAX_CLUSTER_RECORD_NAME_BYTES);
        let kept = optional_name(&object(serde_json::json!({ "name": full }))).ok().flatten();
        assert_eq!(kept.as_deref(), Some(full.as_str()));
        let longer = "n".repeat(MAX_CLUSTER_RECORD_NAME_BYTES + 1);
        assert!(optional_name(&object(serde_json::json!({ "name": longer }))).is_err());
    }

    #[test]
    fn a_rename_needs_a_name_that_is_not_empty() {
        assert!(required_name(&object(serde_json::json!({ "name": "" }))).is_err());
        assert!(required_name(&object(serde_json::json!({}))).is_err());
        assert_eq!(required_name(&object(serde_json::json!({ "name": "Hall" }))).ok().as_deref(), Some("Hall"));
    }

    #[test]
    fn a_revision_is_a_u32() {
        let read = |value: serde_json::Value| required_revision(&object(serde_json::json!({ "revision": value }))).ok();
        assert_eq!(read(serde_json::json!(u32::MAX)), Some(u32::MAX));
        assert_eq!(read(serde_json::json!(u64::from(u32::MAX) + 1)), None);
        assert_eq!(read(serde_json::json!(-1)), None);
        assert_eq!(read(serde_json::json!("1")), None);
    }

    #[test]
    fn each_route_answers_its_own_method() {
        let methods = [
            ClusterRecordRoute::List,
            ClusterRecordRoute::Get,
            ClusterRecordRoute::Create,
            ClusterRecordRoute::Rename,
            ClusterRecordRoute::Delete,
        ]
        .map(ClusterRecordRoute::method);
        assert_eq!(methods, ["GET", "GET", "POST", "PATCH", "DELETE"]);
    }
}
