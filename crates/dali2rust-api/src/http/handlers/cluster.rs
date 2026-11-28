use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    fixed_text_64, BusCommandPayload, ClusterForwarding, ClusterSettingsCommitCommand,
    ClusterSettingsField as Field, ClusterSettingsUpdateCommand, ClusterTextField,
    ClusterTextUpdateCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{ClusterSettingsConflict, CLUSTER_NODE_COUNT, MAX_CLUSTER_NAME_BYTES};

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::cluster_settings_state::{ClusterSettingsApplyWatch, ClusterSettingsDto, ClusterSettingsHttpState};
use crate::http::cluster_status_state::ClusterStatusHttpState;
use crate::http::dispatcher::{publish_batch_and_wait_for_success, CorrelationIdAllocator};
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

use super::common::{
    check_body_keys, json_err, json_err_with_message, json_stream_dto, parse_bool_field, parse_json_body, require_get,
    wait_apply_counter, MutatingHandler, APPLY_WATCH_BUDGET_MS,
};

type JsonObject = serde_json::Map<String, serde_json::Value>;

const ONE_COMMIT: u32 = 1;
const PORT_LIMIT: u64 = 1 << u16::BITS;

const READ_ONLY_KEYS: &[&str] = &["read_only", "block"];

const WRITABLE_KEYS: &[&str] = &[
    "enabled",
    "cluster_name",
    "node_number",
    "port",
    "wire_same_as",
    "forward_operator",
    "forward_home_assistant",
    "forward_hcl",
    "forward_foreign_master",
    "accept_all_lamps",
    "send_all_lamps",
    "accept_single_lamps",
    "wire_protection",
];

const WIRE_GUARD_FIELDS: [Field; 3] = [Field::Wire, Field::ForwardForeignMaster, Field::WireProtection];

const FLAGS: [(&str, Field); 9] = [
    ("enabled", Field::Enabled),
    ("forward_operator", Field::ForwardOperator),
    ("forward_home_assistant", Field::ForwardHomeAssistant),
    ("forward_hcl", Field::ForwardHcl),
    ("forward_foreign_master", Field::ForwardForeignMaster),
    ("accept_all_lamps", Field::AcceptAllLamps),
    ("send_all_lamps", Field::SendAllLamps),
    ("accept_single_lamps", Field::AcceptSingleLamps),
    ("wire_protection", Field::WireProtection),
];

declare_handler_shell! {
    ClusterSettingsGetHandler {
        state: Arc<dyn ClusterSettingsHttpState>,
    }
}

impl ApiHandler for ClusterSettingsGetHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if let Err(error) = require_get(method) {
            return error;
        }
        json_stream_dto(self.state.cluster_settings_dto())
    }
}

declare_handler_shell!(ClusterSettingsPatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn ClusterSettingsHttpState>,
    apply_watch: Arc<dyn ClusterSettingsApplyWatch>,
    bus_id: BusId,
    timeout_ms: u64,
    presence: Arc<dyn ClusterStatusHttpState>,
});

pub struct ClusterPatchData {
    settings: ClusterSettingsUpdateCommand,
    cluster_name: Option<String>,
}

impl ClusterSettingsPatchHandler {
    fn frame(&self, corr: u64, payload: impl Into<BusCommandPayload>) -> (u64, BusFrame) {
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, self.bus_id.0, None, payload);
        (corr, BusFrame::command(cmd))
    }

    fn build_batch(&self, data: ClusterPatchData) -> Vec<(u64, BusFrame)> {
        let series = self.correlation.next_id();
        let settings = (data.settings.patch_mask != 0).then_some(ClusterSettingsUpdateCommand { series, ..data.settings });
        let text = data.cluster_name.map(|name| ClusterTextUpdateCommand {
            series,
            field: ClusterTextField::ClusterName,
            text: fixed_text_64(&name),
        });
        let mut batch: Vec<(u64, BusFrame)> = settings.map(BusCommandPayload::from).into_iter()
            .chain(text.map(BusCommandPayload::from))
            .map(|payload| self.frame(self.correlation.next_id(), payload))
            .collect();
        if !batch.is_empty() {
            let frames = u8::try_from(batch.len()).unwrap_or(u8::MAX);
            batch.push(self.frame(series, ClusterSettingsCommitCommand { series, frames }));
        }
        batch
    }
}

impl ClusterSettingsPatchHandler {
    fn refuse_unprotecting_a_heard_wire(&self, patch: &ClusterSettingsUpdateCommand) -> Result<(), HttpResponse> {
        if !WIRE_GUARD_FIELDS.into_iter().any(|field| patch.patches(field)) {
            return Ok(());
        }
        let Some(wire) = unprotected_own_wire(&self.state.cluster_settings_dto(), patch) else {
            return Ok(());
        };
        if !self.presence.neighbour_names_wire(wire, wire) {
            return Ok(());
        }
        let message = ClusterSettingsConflict::WireProtectionRequired.message();
        Err(json_err_with_message(409, "conflict", message))
    }
}

fn unprotected_own_wire(stored: &ClusterSettingsDto, patch: &ClusterSettingsUpdateCommand) -> Option<u8> {
    let pick = |field, patched: bool, stored: bool| if patch.patches(field) { patched } else { stored };
    let own_wire = pick(Field::Wire, patch.wire_same_as.is_none(), stored.wire_same_as.is_none());
    let foreign = pick(Field::ForwardForeignMaster, patch.forwarding.foreign_master, stored.forward_foreign_master);
    let protected = pick(Field::WireProtection, patch.wire_protection, stored.wire_protection);
    let node = if patch.patches(Field::NodeNumber) { patch.node_number } else { stored.node_number };
    (own_wire && foreign && !protected).then_some(node)
}

impl MutatingHandler for ClusterSettingsPatchHandler {
    type Validated = ClusterPatchData;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(&self, _params: &HashMap<String, String>, body: &[u8]) -> Result<ClusterPatchData, HttpResponse> {
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        check_body_keys(obj, WRITABLE_KEYS, READ_ONLY_KEYS)?;
        parse_cluster_patch(obj)
    }

    fn execute(&self, data: ClusterPatchData) -> Result<(), HttpResponse> {
        self.refuse_unprotecting_a_heard_wire(&data.settings)?;
        let batch = self.build_batch(data);
        if batch.is_empty() {
            return Ok(());
        }
        let before = self.apply_watch.cluster_settings_applied_load();
        publish_batch_and_wait_for_success(&self.publisher, &self.slots, self.timeout_ms, batch)?;
        let watch = Arc::clone(&self.apply_watch);
        wait_apply_counter(
            move || watch.cluster_settings_applied_load(),
            before,
            ONE_COMMIT,
            APPLY_WATCH_BUDGET_MS.min(self.timeout_ms),
        )
    }

    fn respond(&self, (): ()) -> HttpResponse {
        json_stream_dto(self.state.cluster_settings_dto())
    }
}

fn invalid() -> HttpResponse {
    json_err(422, "invalid_value")
}

const fn empty_settings() -> ClusterSettingsUpdateCommand {
    ClusterSettingsUpdateCommand {
        series: 0,
        patch_mask: 0,
        enabled: false,
        node_number: 0,
        port: 0,
        wire_same_as: None,
        forwarding: ClusterForwarding { operator: false, home_assistant: false, hcl: false, foreign_master: false },
        accept_all_lamps: false,
        send_all_lamps: false,
        accept_single_lamps: false,
        wire_protection: false,
    }
}

fn parse_cluster_patch(obj: &JsonObject) -> Result<ClusterPatchData, HttpResponse> {
    let mut settings = empty_settings();
    parse_flags(obj, &mut settings)?;
    parse_numbers(obj, &mut settings)?;
    parse_wire(obj, &mut settings)?;
    Ok(ClusterPatchData { settings, cluster_name: parse_cluster_name(obj)? })
}

fn flag_slot(settings: &mut ClusterSettingsUpdateCommand, field: Field) -> Option<&mut bool> {
    let forwarding = &mut settings.forwarding;
    match field {
        Field::Enabled => Some(&mut settings.enabled),
        Field::ForwardOperator => Some(&mut forwarding.operator),
        Field::ForwardHomeAssistant => Some(&mut forwarding.home_assistant),
        Field::ForwardHcl => Some(&mut forwarding.hcl),
        Field::ForwardForeignMaster => Some(&mut forwarding.foreign_master),
        Field::AcceptAllLamps => Some(&mut settings.accept_all_lamps),
        Field::SendAllLamps => Some(&mut settings.send_all_lamps),
        Field::AcceptSingleLamps => Some(&mut settings.accept_single_lamps),
        Field::WireProtection => Some(&mut settings.wire_protection),
        Field::NodeNumber | Field::Port | Field::Wire => None,
    }
}

fn parse_flags(obj: &JsonObject, settings: &mut ClusterSettingsUpdateCommand) -> Result<(), HttpResponse> {
    for (key, field) in FLAGS {
        let Some(value) = parse_bool_field(obj, key)? else { continue };
        if let Some(slot) = flag_slot(settings, field) {
            *slot = value;
        }
        settings.patch(field);
    }
    Ok(())
}

fn parse_below(obj: &JsonObject, key: &str, low: u64, limit: u64) -> Result<Option<u64>, HttpResponse> {
    obj.get(key)
        .map(|value| value.as_u64().filter(|n| (low..limit).contains(n)).ok_or_else(invalid))
        .transpose()
}

fn parse_numbers(obj: &JsonObject, settings: &mut ClusterSettingsUpdateCommand) -> Result<(), HttpResponse> {
    if let Some(node) = parse_below(obj, "node_number", 0, u64::from(CLUSTER_NODE_COUNT))? {
        settings.node_number = u8::try_from(node).map_err(|_| invalid())?;
        settings.patch(Field::NodeNumber);
    }
    if let Some(port) = parse_below(obj, "port", 1, PORT_LIMIT)? {
        settings.port = u16::try_from(port).map_err(|_| invalid())?;
        settings.patch(Field::Port);
    }
    Ok(())
}

fn parse_wire(obj: &JsonObject, settings: &mut ClusterSettingsUpdateCommand) -> Result<(), HttpResponse> {
    let Some(value) = obj.get("wire_same_as") else {
        return Ok(());
    };
    settings.wire_same_as = if value.is_null() {
        None
    } else {
        let node = value.as_u64().filter(|n| *n < u64::from(CLUSTER_NODE_COUNT)).ok_or_else(invalid)?;
        Some(u8::try_from(node).map_err(|_| invalid())?)
    };
    settings.patch(Field::Wire);
    Ok(())
}

fn parse_cluster_name(obj: &JsonObject) -> Result<Option<String>, HttpResponse> {
    let Some(value) = obj.get("cluster_name") else {
        return Ok(None);
    };
    let name = value.as_str().filter(|name| name.len() <= MAX_CLUSTER_NAME_BYTES).ok_or_else(invalid)?;
    Ok(Some(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(body: serde_json::Value) -> Result<ClusterPatchData, HttpResponse> {
        parse_cluster_patch(body.as_object().expect("object"))
    }

    type Reader = fn(&ClusterSettingsUpdateCommand) -> bool;

    const READERS: [(&str, Field, Reader); 9] = [
        ("enabled", Field::Enabled, |c| c.enabled),
        ("forward_operator", Field::ForwardOperator, |c| c.forwarding.operator),
        ("forward_home_assistant", Field::ForwardHomeAssistant, |c| c.forwarding.home_assistant),
        ("forward_hcl", Field::ForwardHcl, |c| c.forwarding.hcl),
        ("forward_foreign_master", Field::ForwardForeignMaster, |c| c.forwarding.foreign_master),
        ("accept_all_lamps", Field::AcceptAllLamps, |c| c.accept_all_lamps),
        ("send_all_lamps", Field::SendAllLamps, |c| c.send_all_lamps),
        ("accept_single_lamps", Field::AcceptSingleLamps, |c| c.accept_single_lamps),
        ("wire_protection", Field::WireProtection, |c| c.wire_protection),
    ];

    #[test]
    fn every_flag_names_its_own_field_and_only_that_field() {
        assert_eq!(READERS.map(|(key, field, _)| (key, field)), FLAGS);
        for (key, field, read) in READERS {
            let settings = parsed(serde_json::json!({ key: true })).ok().expect("parses").settings;
            assert_eq!(settings.patch_mask, field.bit(), "{key}");
            assert!(read(&settings), "{key} did not land in its field");
            let set = READERS.iter().filter(|(_, _, other)| other(&settings)).count();
            assert_eq!(set, 1, "{key} moved another field");
        }
    }

    fn stored(foreign: bool, protected: bool) -> ClusterSettingsDto {
        let mut dto = crate::http::cluster_settings_state::cluster_settings_view_to_dto(Default::default(), false);
        dto.forward_foreign_master = foreign;
        dto.wire_protection = protected;
        dto
    }

    fn unprotected(stored: &ClusterSettingsDto, body: serde_json::Value) -> Option<u8> {
        unprotected_own_wire(stored, &parsed(body).ok().expect("parses").settings)
    }

    fn unprotects(stored: &ClusterSettingsDto, body: serde_json::Value) -> bool {
        unprotected(stored, body).is_some()
    }

    #[test]
    fn only_an_own_wire_with_the_foreign_master_forwarded_and_protection_off_is_unprotected() {
        use serde_json::json;
        assert!(unprotects(&stored(true, true), json!({ "wire_protection": false })));
        assert!(!unprotects(&stored(false, true), json!({ "wire_protection": false })), "the foreign master stays home");
        assert!(!unprotects(&stored(true, true), json!({ "wire_same_as": 1, "wire_protection": false })), "another node's wire");
        assert!(unprotects(&stored(false, false), json!({ "forward_foreign_master": true })));
        assert!(!unprotects(&stored(true, false), json!({ "wire_protection": true })));
        assert!(unprotects(&stored(true, false), json!({ "wire_same_as": null })));
    }

    #[test]
    fn the_wire_asked_about_is_the_one_the_patch_leaves_this_node_on() {
        use serde_json::json;
        let mut on_another = stored(true, false);
        on_another.wire_same_as = Some(1);
        on_another.node_number = 3;
        assert_eq!(unprotected(&on_another, json!({ "wire_same_as": null })), Some(3), "back to its own wire");
        assert_eq!(unprotected(&on_another, json!({ "wire_same_as": null, "node_number": 5 })), Some(5), "a new number");
        assert_eq!(unprotected(&stored(true, true), json!({ "wire_protection": false, "node_number": 6 })), Some(6));
    }

    #[test]
    fn a_null_wire_is_the_node_s_own_wire_and_still_a_patch() {
        let data = parsed(serde_json::json!({ "wire_same_as": null })).ok().expect("parses");
        assert!(data.settings.patches(Field::Wire));
        assert_eq!(data.settings.wire_same_as, None);
    }

    #[test]
    fn the_edges_of_each_number_are_kept_and_one_past_them_refused() {
        let kept = parsed(serde_json::json!({ "node_number": 7, "port": 65_535, "wire_same_as": 7 }));
        let kept = kept.ok().expect("parses").settings;
        assert_eq!((kept.node_number, kept.port, kept.wire_same_as), (7, u16::MAX, Some(7)));
        for body in [
            serde_json::json!({ "node_number": 8 }),
            serde_json::json!({ "port": 0 }),
            serde_json::json!({ "port": 65_536 }),
            serde_json::json!({ "wire_same_as": 8 }),
        ] {
            assert!(parsed(body.clone()).is_err(), "{body}");
        }
    }

    #[test]
    fn a_name_of_the_full_length_is_kept_and_one_byte_more_refused() {
        let full = "n".repeat(MAX_CLUSTER_NAME_BYTES);
        let kept = parsed(serde_json::json!({ "cluster_name": full })).ok().expect("parses");
        let kept = kept.cluster_name;
        assert_eq!(kept.as_deref(), Some(full.as_str()));
        let longer = "n".repeat(MAX_CLUSTER_NAME_BYTES + 1);
        assert!(parsed(serde_json::json!({ "cluster_name": longer })).is_err());
    }
}
