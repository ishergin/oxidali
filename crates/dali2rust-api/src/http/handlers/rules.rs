use std::sync::Arc;

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::msg::{
    OperationType, RuleCommitCommand, RuleEnableCommand, RuleStageCommand,
    MAX_RULES_SOURCE_BYTES, RULE_SOURCE_CHUNK_BYTES,
};
use dali2rust_rules_model::{NameResolver, RuleCompiler};
use serde_json::{json, Value};

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{accepted_operation_response, json_err};
use crate::http::rules_state::{RuleRuntimeView, RulesHttpState};
use crate::http::types::HttpResponse;

fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

pub enum RulesAction {
    Get,
    Parse,
    Put,
    PatchRule,
    RunRule,
}

pub struct RulesHandlerShared {
    state: Arc<dyn RulesHttpState>,
    compiler: Arc<dyn RuleCompiler>,
    resolver: Arc<dyn NameResolver>,
    publisher: BusPublisher,
    bus_id: BusId,
    correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
}

impl RulesHandlerShared {
    pub fn new(
        state: Arc<dyn RulesHttpState>,
        compiler: Arc<dyn RuleCompiler>,
        resolver: Arc<dyn NameResolver>,
        publisher: BusPublisher,
        bus_id: BusId,
        correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    ) -> Self {
        Self {
            state,
            compiler,
            resolver,
            publisher,
            bus_id,
            correlation,
        }
    }
}

pub struct RulesHandler {
    shared: Arc<RulesHandlerShared>,
    action: RulesAction,
}

impl RulesHandler {
    pub fn new(shared: Arc<RulesHandlerShared>, action: RulesAction) -> Self {
        Self { shared, action }
    }
}

impl ApiHandler for RulesHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        body: &[u8],
        params: &std::collections::HashMap<String, String>,
    ) -> HttpResponse {
        match self.action {
            RulesAction::Get => self.get(params),
            RulesAction::Parse => self.parse(body),
            RulesAction::Put => self.put(body),
            RulesAction::PatchRule => self.patch_rule(params, body),
            RulesAction::RunRule => self.run_rule(params, body),
        }
    }
}

impl RulesHandler {
    fn get(&self, params: &std::collections::HashMap<String, String>) -> HttpResponse {
        let doc = self.shared.state.document();
        let body = if params.get("format").map(String::as_str) == Some("json") {
            let mut rules = serde_json::to_value(&doc.compiled).unwrap_or(Value::Null);
            attach_runtime(&mut rules, &self.shared.state.rule_runtime());
            json!({
                "lang_id": doc.lang_id,
                "revision": doc.revision,
                "diagnostic": doc.diagnostic,
                "rules": rules,
            })
        } else {
            json!({
                "lang_id": doc.lang_id,
                "revision": doc.revision,
                "diagnostic": doc.diagnostic,
                "rule_count": doc.compiled.as_ref().map_or(0, |s| s.rules.len()),
                "source": doc.source,
            })
        };
        HttpResponse::json(200, serde_json::to_vec(&body).unwrap_or_default())
    }

    fn parse(&self, body: &[u8]) -> HttpResponse {
        let source = match source_of(body) {
            Ok(source) => source,
            Err(response) => return response,
        };
        match self.shared.compiler.compile(&source, self.shared.resolver.as_ref()) {
            Ok(set) => {
                let names: Vec<Value> = set
                    .rules
                    .iter()
                    .map(|r| json!({ "name": r.name, "enabled": r.enabled }))
                    .collect();
                HttpResponse::json(
                    200,
                    serde_json::to_vec(&json!({ "ok": true, "rules": names }))
                        .unwrap_or_default(),
                )
            }
            Err(error) => HttpResponse::json(
                400,
                serde_json::to_vec(&json!({
                    "error": "parse_error",
                    "line": error.line,
                    "column": error.column,
                    "message": error.message,
                }))
                .unwrap_or_default(),
            ),
        }
    }

    fn put(&self, body: &[u8]) -> HttpResponse {
        let json: Value = match serde_json::from_slice(body) {
            Ok(json) => json,
            Err(_) => return json_err(400, "invalid_json"),
        };
        let Some(source) = json.get("source").and_then(Value::as_str) else {
            return json_err(400, "missing_source");
        };
        let Some(base_revision) = json.get("base_revision").and_then(Value::as_u64) else {
            return json_err(400, "missing_base_revision");
        };
        if source.len() > MAX_RULES_SOURCE_BYTES {
            return json_err(413, "rule_document_too_large");
        }
        if let Err(error) = self.shared.compiler.compile(source, self.shared.resolver.as_ref()) {
            return HttpResponse::json(
                400,
                serde_json::to_vec(&json!({
                    "error": "parse_error",
                    "line": error.line,
                    "column": error.column,
                    "message": error.message,
                }))
                .unwrap_or_default(),
            );
        }
        if base_revision != u64::from(self.shared.state.revision()) {
            return json_err(409, "rule_set_conflict");
        }
        self.publish_document(source, base_revision as u32)
    }

    fn publish_document(&self, source: &str, base_revision: u32) -> HttpResponse {
        let corr = self.shared.correlation.next_id();
        let key = format!("rules-{corr}");
        let frames = document_frames(source, base_revision, self.shared.compiler.lang_id(), corr, self.shared.bus_id);
        if let Err(response) = crate::http::handlers::operation_dispatch::publish_begin_then_paced_series(
            &self.shared.publisher,
            self.shared.bus_id,
            corr,
            &key,
            frames,
        ) {
            return response;
        }
        accepted_operation_response(key, OperationType::ConfigWrite)
    }

    fn run_rule(
        &self,
        params: &std::collections::HashMap<String, String>,
        _body: &[u8],
    ) -> HttpResponse {
        let Some(name) = params.get("rule_name") else {
            return json_err(400, "invalid_resource_id");
        };
        let known = self.shared
            .state
            .document()
            .compiled
            .as_ref()
            .is_some_and(|set| set.rule(name).is_some());
        if !known {
            return json_err(404, "rule_not_found");
        }
        let dry = params.get("dry").map(String::as_str) == Some("1");
        let corr = self.shared.correlation.next_id();
        let key = format!("rule-run-{corr}");
        let semantic = dali2rust_contracts::bus::command_envelope(
            crate::bus_codec::SOURCE_ID_UNSPECIFIED,
            corr,
            self.shared.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::RuleRunCommand {
                name: dali2rust_contracts::msg::fixed_text_64(name),
                dry,
            },
        );
        if let Err(response) =
            crate::http::handlers::operation_dispatch::publish_begin_then_semantic_command_pair(
                &self.shared.publisher,
                self.shared.bus_id,
                corr,
                &key,
                OperationType::ConfigWrite,
                semantic,
            )
        {
            return response;
        }
        accepted_operation_response(key, OperationType::ConfigWrite)
    }

    fn patch_rule(
        &self,
        params: &std::collections::HashMap<String, String>,
        body: &[u8],
    ) -> HttpResponse {
        let Some(name) = params.get("rule_name") else {
            return json_err(400, "invalid_resource_id");
        };
        let json: Value = match serde_json::from_slice(body) {
            Ok(json) => json,
            Err(_) => return json_err(400, "invalid_json"),
        };
        let Some(enabled) = json.get("enabled").and_then(Value::as_bool) else {
            return json_err(400, "missing_enabled");
        };
        let doc = self.shared.state.document();
        if doc.compiled.is_none() && doc.diagnostic.is_some() {
            return json_err(409, "rules_not_compiled");
        }
        if doc.compiled.as_ref().and_then(|set| set.rule(name)).is_none() {
            return json_err(404, "rule_not_found");
        }
        let before = self.shared.state.revision();
        let env = dali2rust_contracts::bus::command_envelope(
            crate::bus_codec::SOURCE_ID_UNSPECIFIED,
            self.shared.correlation.next_id(),
            self.shared.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            RuleEnableCommand {
                name: dali2rust_contracts::msg::fixed_text_64(name),
                enabled,
            },
        );
        if self.shared.publisher.try_publish(BusChannel::Commands, BusFrame::command(env))
            != PublishResult::Queued
        {
            return json_err(503, "commands_ingress_overload");
        }
        wait_revision_past(self.shared.state.as_ref(), before);
        HttpResponse::json(
            200,
            serde_json::to_vec(&json!({ "name": name, "enabled": enabled }))
                .unwrap_or_default(),
        )
    }
}

fn attach_runtime(rules: &mut Value, runtime: &[RuleRuntimeView]) {
    let Some(list) = rules.get_mut("rules").and_then(Value::as_array_mut) else {
        return;
    };
    for rule in list {
        let name = rule.get("name").and_then(Value::as_str).unwrap_or_default();
        let block = runtime_json(runtime.iter().find(|row| row.name == name));
        if let Some(object) = rule.as_object_mut() {
            object.insert("runtime".to_owned(), block);
        }
    }
}

fn runtime_json(row: Option<&RuleRuntimeView>) -> Value {
    match row {
        Some(row) => json!({
            "fire_count": row.fire_count,
            "last_fired_at_ms": row.last_fired_at_ms,
            "last_latency_ms": row.last_latency_ms,
            "last_outcome": row.last_outcome,
            "last_error": row.last_error,
        }),
        None => json!({
            "fire_count": 0,
            "last_fired_at_ms": null,
            "last_latency_ms": null,
            "last_outcome": null,
            "last_error": null,
        }),
    }
}

fn source_of(body: &[u8]) -> Result<String, HttpResponse> {
    let json: Value = serde_json::from_slice(body).map_err(|_| json_err(400, "invalid_json"))?;
    let source = json
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| json_err(400, "missing_source"))?;
    if source.len() > MAX_RULES_SOURCE_BYTES {
        return Err(json_err(413, "rule_document_too_large"));
    }
    Ok(source.to_string())
}

fn document_frames(
    source: &str,
    base_revision: u32,
    lang_id: u8,
    corr: u64,
    bus_id: BusId,
) -> Vec<BusFrame> {
    let bytes = source.as_bytes();
    let chunks: Vec<&[u8]> = if bytes.is_empty() {
        vec![b"" as &[u8]]
    } else {
        bytes.chunks(RULE_SOURCE_CHUNK_BYTES).collect()
    };
    let mut frames = Vec::with_capacity(chunks.len() + 1);
    for (index, chunk) in chunks.iter().enumerate() {
        let mut fixed = dali2rust_contracts::msg::FixedItems::new();
        for byte in *chunk {
            let _ = fixed.push(*byte);
        }
        frames.push(BusFrame::command(dali2rust_contracts::bus::command_envelope(
            crate::bus_codec::SOURCE_ID_UNSPECIFIED,
            corr,
            bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            RuleStageCommand {
                chunk_index: u8::try_from(index).unwrap_or(u8::MAX),
                chunk_count: u8::try_from(chunks.len()).unwrap_or(u8::MAX),
                bytes: fixed,
            },
        )));
    }
    frames.push(BusFrame::command(dali2rust_contracts::bus::command_envelope(
        crate::bus_codec::SOURCE_ID_UNSPECIFIED,
        corr,
        bus_id.0,
        Some(dali2rust_contracts::msg::Origin::Api),
        RuleCommitCommand {
            chunk_count: u8::try_from(chunks.len()).unwrap_or(u8::MAX),
            total_len: u16::try_from(bytes.len()).unwrap_or(u16::MAX),
            source_hash: fnv1a32(bytes),
            base_revision,
            lang_id,
        },
    )));
    frames
}

fn wait_revision_past(state: &dyn RulesHttpState, before: u32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < deadline {
        if state.revision() != before {
            return;
        }
        // sleep-ok: bounded apply-watch read-after-write poll (>= 10 ms step)
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
