use dali2rust_contracts::msg::{
    BusEventPayload, ColorValue, DaliInputEventObservedEvent, EventEnvelope, FailureStatus,
    InputEventKind, LightSetpoint, OperationStatusChangedEvent, RulesActivationEvent,
    RulesChangedEvent, RuntimeObservation, RuntimeStateChangedEvent, StatusFlags,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::bus_codec::product_error_code_snake;
use crate::http::physical_device_state::{
    FailureStatusDto, PhysicalDeviceStateDto, RgbDto, RuntimeErrorDto, StatusFlagsDto, WafDto,
    XyDto,
};
use crate::ws::protocol::{event_frame_with_payload, Channel};

const RUNTIME_STATE_CHANNELS: &[Channel] = &[Channel::VirtualLamps, Channel::PhysicalDevices];

struct Projection<'a> {
    event_type: &'static str,
    channels: &'static [Channel],
    body: ProjectionBody<'a>,
}

enum ProjectionBody<'a> {
    RuntimeState(&'a RuntimeStateChangedEvent),
    Operation(&'a OperationStatusChangedEvent),
    RulesChanged(&'a RulesChangedEvent),
    RulesActivation(&'a RulesActivationEvent),
    InputEvent(&'a DaliInputEventObservedEvent),
    Identity(IdentityIds),
}

impl ProjectionBody<'_> {
    fn to_payload(&self) -> Value {
        match self {
            Self::RuntimeState(ev) => runtime_state_payload(ev),
            Self::Operation(ev) => operation_payload(ev),
            Self::RulesChanged(ev) => json!({
                "revision": ev.revision,
                "rule_count": ev.rule_count,
                "lang_id": ev.lang_id,
            }),
            Self::RulesActivation(ev) => json!({
                "rule_name": ev.rule_name.as_str(),
                "dry": ev.dry,
                "effects": ev.effects,
                "partial": ev.partial,
                "trigger_to_publish_ms": ev.trigger_to_publish_ms,
            }),
            Self::InputEvent(ev) => input_event_payload(ev),
            Self::Identity(ids) => json!(ids),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct IdentityIds {
    adapter_id: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    virtual_lamp_id: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    short_address: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scene_id: Option<u8>,
}

impl IdentityIds {
    const fn adapter(adapter_id: u8) -> Self {
        Self {
            adapter_id,
            virtual_lamp_id: None,
            short_address: None,
            group_id: None,
            scene_id: None,
        }
    }
}

type IdentityRow = (&'static str, &'static [Channel], IdentityIds);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoalesceKey {
    kind: u16,
    detail: KeyDetail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum KeyDetail {
    RuntimeState {
        adapter_id: u8,
        virtual_lamp_id: Option<u8>,
        short_address: Option<u8>,
    },
    Operation {
        key: dali2rust_contracts::msg::FixedText32,
    },
    Occurrence {
        mono_ms: u32,
        event_info: u16,
    },
    Identity(IdentityIds),
}

pub fn coalesce_key(envelope: &EventEnvelope) -> Option<CoalesceKey> {
    let p = projection(&envelope.payload)?;
    let detail = match p.body {
        ProjectionBody::RuntimeState(ev) => KeyDetail::RuntimeState {
            adapter_id: ev.adapter_id,
            virtual_lamp_id: ev.virtual_lamp_id,
            short_address: ev.short_address,
        },
        ProjectionBody::Operation(ev) => KeyDetail::Operation {
            key: ev.operation_key.clone(),
        },
        ProjectionBody::RulesActivation(ev) => KeyDetail::Operation {
            key: dali2rust_contracts::msg::fixed_text_32(ev.rule_name.as_str()),
        },
        ProjectionBody::RulesChanged(_) => KeyDetail::Identity(IdentityIds::adapter(0)),
        ProjectionBody::InputEvent(ev) => KeyDetail::Occurrence {
            mono_ms: ev.observed_at_mono_ms,
            event_info: ev.event_info,
        },
        ProjectionBody::Identity(ids) => KeyDetail::Identity(ids),
    };
    Some(CoalesceKey {
        kind: envelope.payload.variant_index() as u16,
        detail,
    })
}

pub const WS_PROJECTED_EVENTS: &[&str] = &[
    "RuntimeStateChangedEvent",
    "OperationStatusChangedEvent",
    "AdapterSettingsChangedEvent",
    "PhysicalDeviceChangedEvent",
    "VirtualLampChangedEvent",
    "GroupChangedEvent",
    "GroupMatrixChangedEvent",
    "SceneChangedEvent",
    "SceneMatrixChangedEvent",
    "InputDeviceChangedEvent",
    "DaliInputEventObservedEvent",
    "DaliInputDeviceLifecycleEvent",
    "RulesChangedEvent",
    "RulesActivationEvent",
];

fn projection(payload: &BusEventPayload) -> Option<Projection<'_>> {
    let (event_type, channels, body): (&'static str, &'static [Channel], ProjectionBody<'_>) =
        match payload {
            BusEventPayload::RuntimeStateChangedEvent(ev) => (
                "RuntimeStateChangedEvent",
                RUNTIME_STATE_CHANNELS,
                ProjectionBody::RuntimeState(ev),
            ),
            BusEventPayload::OperationStatusChangedEvent(ev) => (
                "OperationStatusChangedEvent",
                &[Channel::Operations],
                ProjectionBody::Operation(ev),
            ),
            BusEventPayload::RulesChangedEvent(ev) => (
                "RulesChangedEvent",
                &[Channel::Rules],
                ProjectionBody::RulesChanged(ev),
            ),
            BusEventPayload::RulesActivationEvent(ev) => (
                "RulesActivationEvent",
                &[Channel::Rules],
                ProjectionBody::RulesActivation(ev),
            ),
            BusEventPayload::DaliInputEventObservedEvent(ev) => (
                "DaliInputEventObservedEvent",
                &[Channel::Input],
                ProjectionBody::InputEvent(ev),
            ),
            other => return identity_projection(other),
        };
    Some(Projection {
        event_type,
        channels,
        body,
    })
}

fn identity_projection(payload: &BusEventPayload) -> Option<Projection<'_>> {
    let (event_type, channels, ids) =
        device_identity(payload).or_else(|| collection_identity(payload))?;
    Some(Projection {
        event_type,
        channels,
        body: ProjectionBody::Identity(ids),
    })
}

fn device_identity(payload: &BusEventPayload) -> Option<IdentityRow> {
    if let Some(row) = input_device_identity(payload) {
        return Some(row);
    }
    Some(match payload {
        BusEventPayload::AdapterSettingsChangedEvent(ev) => (
            "AdapterSettingsChangedEvent",
            &[Channel::Adapters][..],
            IdentityIds::adapter(ev.adapter_id),
        ),
        BusEventPayload::PhysicalDeviceChangedEvent(ev) => (
            "PhysicalDeviceChangedEvent",
            &[Channel::PhysicalDevices][..],
            IdentityIds {
                short_address: Some(ev.short_address),
                ..IdentityIds::adapter(ev.adapter_id)
            },
        ),
        BusEventPayload::VirtualLampChangedEvent(ev) => (
            "VirtualLampChangedEvent",
            &[Channel::VirtualLamps][..],
            IdentityIds {
                virtual_lamp_id: Some(ev.virtual_lamp_id),
                ..IdentityIds::adapter(ev.adapter_id)
            },
        ),
        _ => return None,
    })
}

fn input_device_identity(payload: &BusEventPayload) -> Option<IdentityRow> {
    Some(match payload {
        BusEventPayload::InputDeviceChangedEvent(ev) => (
            "InputDeviceChangedEvent",
            &[Channel::Input][..],
            IdentityIds {
                short_address: Some(ev.short_address),
                ..IdentityIds::adapter(ev.adapter_id)
            },
        ),
        BusEventPayload::DaliInputDeviceLifecycleEvent(ev) => (
            "DaliInputDeviceLifecycleEvent",
            &[Channel::Input][..],
            IdentityIds {
                short_address: ev.short_address,
                ..IdentityIds::adapter(ev.registry_adapter_id)
            },
        ),
        _ => return None,
    })
}

fn collection_identity(payload: &BusEventPayload) -> Option<IdentityRow> {
    Some(match payload {
        BusEventPayload::GroupChangedEvent(ev) => (
            "GroupChangedEvent",
            &[Channel::Groups][..],
            IdentityIds {
                group_id: Some(ev.group_id),
                ..IdentityIds::adapter(ev.adapter_id)
            },
        ),
        BusEventPayload::GroupMatrixChangedEvent(ev) => (
            "GroupMatrixChangedEvent",
            &[Channel::Groups][..],
            IdentityIds::adapter(ev.adapter_id),
        ),
        BusEventPayload::SceneChangedEvent(ev) => (
            "SceneChangedEvent",
            &[Channel::Scenes][..],
            IdentityIds {
                scene_id: Some(ev.scene_id),
                ..IdentityIds::adapter(ev.adapter_id)
            },
        ),
        BusEventPayload::SceneMatrixChangedEvent(ev) => (
            "SceneMatrixChangedEvent",
            &[Channel::Scenes][..],
            IdentityIds {
                scene_id: Some(ev.scene_id),
                ..IdentityIds::adapter(ev.adapter_id)
            },
        ),
        _ => return None,
    })
}

pub fn event_channels(envelope: &EventEnvelope) -> &'static [Channel] {
    let p = projection(&envelope.payload);
    debug_assert_eq!(
        p.is_some(),
        WS_PROJECTED_EVENTS.contains(&envelope.payload.variant_name()),
        "WS_PROJECTED_EVENTS and the projection match disagree for {}",
        envelope.payload.variant_name()
    );
    p.map_or(&[], |p| p.channels)
}

pub fn project_event(envelope: &EventEnvelope) -> Vec<(Channel, String)> {
    project_event_where(envelope, &|_| true)
}

pub fn project_event_where(
    envelope: &EventEnvelope,
    wanted: &dyn Fn(Channel) -> bool,
) -> Vec<(Channel, String)> {
    let Some(projection) = projection(&envelope.payload) else {
        return Vec::new();
    };
    let channels: Vec<Channel> = projection
        .channels
        .iter()
        .copied()
        .filter(|channel| wanted(*channel))
        .collect();
    if channels.is_empty() {
        return Vec::new();
    }
    let payload = serde_json::to_string(&projection.body.to_payload())
        .unwrap_or_else(|_| "null".into());
    let ts = envelope.meta.timestamp_ms;
    channels
        .into_iter()
        .map(|channel| {
            (
                channel,
                event_frame_with_payload(projection.event_type, channel, ts, &payload),
            )
        })
        .collect()
}

fn input_event_payload(ev: &DaliInputEventObservedEvent) -> Value {
    json!({
        "adapter_id": ev.registry_adapter_id,
        "scheme": ev.scheme,
        "short_address": ev.short_address,
        "device_group": ev.device_group,
        "instance_group": ev.instance_group,
        "instance_number": ev.instance_number,
        "instance_type": ev.instance_type,
        "event": input_event_name(ev),
        "event_info": ev.event_info,
        "value": ev.typed_value,
    })
}

fn input_event_name(ev: &DaliInputEventObservedEvent) -> Option<&'static str> {
    use dali2rust_domain::dali::dev103::event::{ButtonEvent, OccupancyEvent};
    match ev.typed {
        InputEventKind::Button => ButtonEvent::from_info(ev.event_info).map(ButtonEvent::name),
        InputEventKind::Occupancy => OccupancyEvent::from_info(ev.event_info).map(|o| {
            match (o.occupied, o.still) {
                (true, false) => "becomes occupied",
                (true, true) => "still occupied",
                (false, false) => "becomes vacant",
                (false, true) => "still vacant",
            }
        }),
        InputEventKind::Position => Some("position"),
        InputEventKind::Illuminance => Some("illuminance"),
        InputEventKind::Generic => None,
    }
}

fn runtime_state_payload(ev: &RuntimeStateChangedEvent) -> Value {
    json!({
        "adapter_id": ev.adapter_id,
        "virtual_lamp_id": ev.virtual_lamp_id,
        "short_address": ev.short_address,
        "state": runtime_state_contract(&ev.state_setpoint, &ev.state_observation),
    })
}

fn runtime_state_contract(
    setpoint: &LightSetpoint,
    obs: &RuntimeObservation,
) -> PhysicalDeviceStateDto {
    let color = setpoint.color.as_ref();
    PhysicalDeviceStateDto {
        power: setpoint.power.rest_name().to_string(),
        level: Some(setpoint.level),
        color_mode: color.map_or("unknown", |c| c.mode.rest_name()).to_string(),
        color_temperature_kelvin: color.and_then(ColorValue::cct),
        xy: color
            .and_then(ColorValue::xy_wire)
            .map(|(x, y)| XyDto::from_wire(x, y)),
        rgb: color
            .and_then(ColorValue::rgb_channels)
            .map(|(r, g, b)| RgbDto { r, g, b }),
        waf: color
            .and_then(ColorValue::waf_channels)
            .map(|(w, a, f)| WafDto { w, a, f }),
        status: obs.status_flags.as_ref().map(status_dto),
        failure_status: obs.failure_status.as_ref().map(failure_dto),
        value_source: obs.value_source.map(|s| s.rest_name().to_string()),
        last_seen_ms: obs.last_seen_ms,
        last_dapc_source: obs.last_dapc_source.rest_name().map(str::to_string),
        error: obs.error.as_ref().map(|e| RuntimeErrorDto {
            code: product_error_code_snake(e.code),
        }),
    }
}

fn status_dto(flags: &StatusFlags) -> StatusFlagsDto {
    StatusFlagsDto {
        raw: flags.raw,
        lamp_failure: flags.lamp_failure,
        gear_failure: flags.gear_failure,
        lamp_on: flags.lamp_on,
        limit_error: flags.limit_error,
        fade_running: flags.fade_running,
        reset_state: flags.reset_state,
        missing_short_address: flags.missing_short_address,
        power_cycle_seen: flags.power_cycle_seen,
    }
}

fn failure_dto(status: &FailureStatus) -> FailureStatusDto {
    FailureStatusDto {
        raw: status.raw,
        lamp_failure: status.lamp_failure,
        gear_failure: status.gear_failure,
        communication_failure: status.communication_failure,
        source: status.source.rest_name().to_string(),
    }
}

fn operation_payload(ev: &OperationStatusChangedEvent) -> Value {
    let mut payload = json!({
        "operation_id": ev.operation_key.as_str(),
        "type": ev.operation_type.rest_name(),
        "status": ev.status.rest_name(),
    });
    if let Some(error) = ev.error.as_ref() {
        payload["error"] = json!({
            "code": product_error_code_snake(error.code),
            "message": error.message.as_str(),
        });
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::ColorValue;
    use dali2rust_contracts::bus::event_envelope;
    use dali2rust_contracts::msg::{
        fixed_text_32, ColorMode, CompactErrorPayload, ErrorCode, FailureStatus, LastDapcSource,
        OperationStatus, OperationType, Origin, PowerState, RuntimeSource, StatusFlags,
    };

    const BUS_ID: u16 = 0;
    const CORR: u64 = 42;

    fn envelope(payload: impl Into<BusEventPayload>) -> EventEnvelope {
        event_envelope(0, CORR, BUS_ID, Some(Origin::Api), payload)
    }

    fn input_event(
        scheme: u8,
        short_address: Option<u8>,
        event_info: u16,
        typed: InputEventKind,
        mono_ms: u32,
    ) -> DaliInputEventObservedEvent {
        DaliInputEventObservedEvent {
            registry_adapter_id: 0,
            scheme,
            short_address,
            device_group: None,
            instance_group: None,
            instance_number: Some(0),
            instance_type: Some(1),
            event_info,
            typed,
            typed_value: event_info,
            observed_at_ms: 1,
            observed_at_mono_ms: mono_ms,
        }
    }

    fn input_payload(ev: DaliInputEventObservedEvent) -> Value {
        let frames = project_event(&envelope(ev));
        assert_eq!(frames.len(), 1, "one channel");
        let frame: Value = serde_json::from_str(&frames[0].1).expect("frame is json");
        frame["payload"].clone()
    }

    #[test]
    fn an_observed_press_carries_its_product_name_and_its_scheme() {
        let p = input_payload(input_event(2, Some(3), 0x002, InputEventKind::Button, 100));
        assert_eq!(p["event"], "short_press");
        assert_eq!(p["scheme"], 2);
        assert_eq!(p["short_address"], 3);
        assert_eq!(p["instance_number"], 0);
        assert_eq!(p["event_info"], 0x002);
    }

    #[test]
    fn an_unattributable_event_still_reports_the_scheme_that_made_it_so() {
        let p = input_payload(input_event(0, None, 0x002, InputEventKind::Button, 100));
        assert_eq!(p["scheme"], 0);
        assert!(p["short_address"].is_null(), "scheme 0 names no device");
        assert_eq!(p["event"], "short_press", "the event is still known");
    }

    #[test]
    fn an_untyped_event_names_nothing_and_keeps_its_bits() {
        let p = input_payload(input_event(2, Some(3), 0x1A4, InputEventKind::Generic, 100));
        assert!(p["event"].is_null());
        assert_eq!(p["event_info"], 0x1A4);
    }

    #[test]
    fn two_presses_of_one_button_never_coalesce() {
        let first = coalesce_key(&envelope(input_event(
            2,
            Some(3),
            0x002,
            InputEventKind::Button,
            100,
        )))
        .expect("projected");
        let second = coalesce_key(&envelope(input_event(
            2,
            Some(3),
            0x002,
            InputEventKind::Button,
            140,
        )))
        .expect("projected");
        assert_ne!(first, second);

        let a = coalesce_key(&envelope(input_event(0, None, 0x002, InputEventKind::Button, 200)))
            .expect("projected");
        let b = coalesce_key(&envelope(input_event(0, None, 0x001, InputEventKind::Button, 200)))
            .expect("projected");
        assert_ne!(a, b);
    }

    fn cct_setpoint() -> LightSetpoint {
        LightSetpoint {
            power: PowerState::On,
            level: 180,
            color: Some(ColorValue {
                mode: ColorMode::Cct,
                color_temperature_kelvin: 3000,
                ..ColorValue::default()
            }),
        }
    }

    fn observation() -> RuntimeObservation {
        RuntimeObservation {
            status_flags: Some(StatusFlags {
                raw: 0,
                lamp_failure: false,
                gear_failure: false,
                lamp_on: false,
                limit_error: false,
                fade_running: false,
                reset_state: false,
                missing_short_address: false,
                power_cycle_seen: false,
            }),
            failure_status: Some(FailureStatus::default()),
            value_source: Some(RuntimeSource::Poller),
            last_seen_ms: Some(123_456),
            last_dapc_source: LastDapcSource::Unknown,
            error: None,
        }
    }

    fn runtime_event() -> EventEnvelope {
        envelope(RuntimeStateChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: Some(12),
            short_address: Some(17),
            state_setpoint: cct_setpoint(),
            state_observation: observation(),
            commit_source: RuntimeSource::Api,
            commit_dimensions: cct_setpoint().dimensions(),
        })
    }

    #[test]
    fn runtime_state_matches_the_canonical_fixture() {
        let frames = project_event(&runtime_event());
        let (_, frame) = frames
            .iter()
            .find(|(c, _)| *c == Channel::VirtualLamps)
            .expect("virtual_lamps frame");
        let v: Value = serde_json::from_str(frame).unwrap();
        assert_eq!(v["type"], "RuntimeStateChangedEvent");
        assert_eq!(v["channel"], "virtual_lamps");
        let p = &v["payload"];
        assert_eq!(p["adapter_id"], 0);
        assert_eq!(p["virtual_lamp_id"], 12);
        assert_eq!(p["short_address"], 17);
        let s = &p["state"];
        assert_eq!(s["power"], "on");
        assert_eq!(s["level"], 180);
        assert_eq!(s["color_mode"], "cct");
        assert_eq!(s["color_temperature_kelvin"], 3000);
        assert!(s["xy"].is_null());
        assert!(s["rgb"].is_null());
        assert_eq!(s["value_source"], "poller");
        assert_eq!(s["last_seen_ms"], 123_456);
        assert!(s["last_dapc_source"].is_null());
        assert!(s["error"].is_null());
        assert_eq!(s["status"]["raw"], 0);
        assert_eq!(s["failure_status"]["source"], "poller");
    }

    #[test]
    fn a_runtime_commit_reaches_both_views_of_the_same_gear() {
        let channels: Vec<Channel> = project_event(&runtime_event())
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert_eq!(
            channels,
            vec![Channel::VirtualLamps, Channel::PhysicalDevices]
        );
    }

    #[test]
    fn an_operation_frame_carries_the_string_key_and_no_correlation() {
        let ev = envelope(OperationStatusChangedEvent {
            operation_type: OperationType::AttributeRead,
            status: OperationStatus::Running,
            error: None,
            started_at_ms: 1,
            finished_at_ms: None,
            ttl_remaining_ms: 60_000,
            operation_key: fixed_text_32("pd-attr-0-17-42"),
        });
        let (channel, frame) = project_event(&ev).into_iter().next().unwrap();
        assert_eq!(channel, Channel::Operations);
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["payload"]["operation_id"], "pd-attr-0-17-42");
        assert_eq!(v["payload"]["type"], "attribute_read");
        assert_eq!(v["payload"]["status"], "running");
        assert!(v["payload"].get("correlation_id").is_none());
        assert!(v["payload"].get("error").is_none());
    }

    #[test]
    fn a_terminal_operation_carries_its_error() {
        let ev = envelope(OperationStatusChangedEvent {
            operation_type: OperationType::CommissioningAddressChange,
            status: OperationStatus::Failed,
            error: Some(CompactErrorPayload::new(
                ErrorCode::VerifyFailed,
                "readback mismatch",
            )),
            started_at_ms: 1,
            finished_at_ms: Some(2),
            ttl_remaining_ms: 0,
            operation_key: fixed_text_32("comm-addr-0-17"),
        });
        let (_, frame) = project_event(&ev).into_iter().next().unwrap();
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["payload"]["error"]["code"], "verify_failed");
        assert_eq!(v["payload"]["error"]["message"], "readback mismatch");
    }

    #[test]
    fn identity_events_are_thin_triggers_not_dtos() {
        let ev = envelope(dali2rust_contracts::msg::VirtualLampChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: 12,
        });
        let (channel, frame) = project_event(&ev).into_iter().next().unwrap();
        assert_eq!(channel, Channel::VirtualLamps);
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["payload"]["virtual_lamp_id"], 12);
        assert!(v["payload"].get("name").is_none());
    }

    fn one_of_every_projected_event() -> Vec<EventEnvelope> {
        use dali2rust_contracts::msg::{
            AdapterSettingsChangedEvent, GroupChangedEvent, GroupMatrixChangedEvent,
            PhysicalDeviceChangedEvent, SceneChangedEvent, SceneMatrixChangedEvent,
            VirtualLampChangedEvent,
        };
        vec![
            runtime_event(),
            envelope(dali2rust_contracts::msg::OperationStatusChangedEvent {
                operation_type: dali2rust_contracts::msg::OperationType::GroupApply,
                status: dali2rust_contracts::msg::OperationStatus::Running,
                error: None,
                started_at_ms: 1,
                finished_at_ms: None,
                ttl_remaining_ms: 1_000,
                operation_key: dali2rust_contracts::msg::fixed_text_32("grp-apply-0-1"),
            }),
            envelope(AdapterSettingsChangedEvent {
                adapter_id: 0,
                name: dali2rust_contracts::msg::fixed_text_64("bus a"),
                enabled: true,
            }),
            envelope(PhysicalDeviceChangedEvent {
                adapter_id: 0,
                short_address: 17,
            }),
            envelope(dali2rust_contracts::msg::InputDeviceChangedEvent {
                adapter_id: 0,
                short_address: 3,
            }),
            envelope(dali2rust_contracts::msg::DaliInputEventObservedEvent {
                registry_adapter_id: 0,
                scheme: 2,
                short_address: Some(3),
                device_group: None,
                instance_group: None,
                instance_number: Some(0),
                instance_type: None,
                event_info: 0x002,
                typed: dali2rust_contracts::msg::InputEventKind::Generic,
                typed_value: 0,
                observed_at_ms: 1,
                observed_at_mono_ms: 1,
            }),
            envelope(dali2rust_contracts::msg::DaliInputDeviceLifecycleEvent {
                registry_adapter_id: 0,
                kind: dali2rust_contracts::msg::InputDeviceLifecycleKind::PowerCycle,
                short_address: Some(3),
                device_group: None,
                observed_at_ms: 1,
                observed_at_mono_ms: 1,
            }),
            envelope(VirtualLampChangedEvent {
                adapter_id: 0,
                virtual_lamp_id: 12,
            }),
            envelope(GroupChangedEvent {
                adapter_id: 0,
                group_id: 3,
            }),
            envelope(GroupMatrixChangedEvent { adapter_id: 0 }),
            envelope(SceneChangedEvent {
                adapter_id: 0,
                scene_id: 4,
            }),
            envelope(SceneMatrixChangedEvent {
                adapter_id: 0,
                scene_id: 4,
            }),
            envelope(dali2rust_contracts::msg::RulesChangedEvent {
                revision: 1,
                rule_count: 1,
                lang_id: 1,
            }),
            envelope(dali2rust_contracts::msg::RulesActivationEvent {
                rule_name: dali2rust_contracts::msg::fixed_text_64("night"),
                dry: false,
                effects: 1,
                partial: 0,
                trigger_to_publish_ms: 5,
            }),
        ]
    }

    #[test]
    fn event_channels_names_exactly_the_channels_the_projection_produces() {
        for ev in one_of_every_projected_event() {
            let produced: Vec<Channel> = project_event(&ev).into_iter().map(|(c, _)| c).collect();
            assert_eq!(
                produced,
                event_channels(&ev).to_vec(),
                "projection and routing disagree for {:?}",
                ev.payload
            );
            assert!(!produced.is_empty(), "{:?} projects to nothing", ev.payload);
        }
    }

    #[test]
    fn ws_projected_events_matches_the_projection_table_exactly() {
        let mut sampled: Vec<&str> = one_of_every_projected_event()
            .iter()
            .map(|ev| ev.payload.variant_name())
            .collect();
        sampled.sort_unstable();
        let mut declared: Vec<&str> = WS_PROJECTED_EVENTS.to_vec();
        declared.sort_unstable();
        assert_eq!(
            sampled, declared,
            "WS_PROJECTED_EVENTS and the projected-sample inventory disagree"
        );
    }

    #[test]
    fn an_unprojected_event_reaches_no_channel_either() {
        let ev = envelope(dali2rust_contracts::msg::DaliDiscoveryFailedEvent {});
        assert!(event_channels(&ev).is_empty());
        assert!(project_event(&ev).is_empty());
    }

    #[test]
    fn every_projected_sample_has_a_coalesce_key_and_unprojected_have_none() {
        for ev in one_of_every_projected_event() {
            assert!(
                coalesce_key(&ev).is_some(),
                "projected kind without a key: {:?}",
                ev.payload
            );
        }
        let unprojected = envelope(dali2rust_contracts::msg::DaliDiscoveryFailedEvent {});
        assert!(coalesce_key(&unprojected).is_none());
    }

    #[test]
    fn coalesce_keys_merge_same_identity_and_separate_kinds() {
        use dali2rust_contracts::msg::{
            AdapterSettingsChangedEvent, GroupMatrixChangedEvent, VirtualLampChangedEvent,
        };
        assert_eq!(
            coalesce_key(&runtime_event()),
            coalesce_key(&runtime_event()),
            "same lamp, same kind: one client-side row"
        );
        let lamp_a = envelope(VirtualLampChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: 1,
        });
        let lamp_b = envelope(VirtualLampChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: 2,
        });
        assert_ne!(coalesce_key(&lamp_a), coalesce_key(&lamp_b));
        let matrix = envelope(GroupMatrixChangedEvent { adapter_id: 0 });
        let settings = envelope(AdapterSettingsChangedEvent {
            adapter_id: 0,
            name: dali2rust_contracts::msg::fixed_text_64("bus a"),
            enabled: true,
        });
        assert_ne!(coalesce_key(&matrix), coalesce_key(&settings));
    }

    #[test]
    fn a_channel_nobody_watches_is_not_serialized() {
        let frames = project_event_where(&runtime_event(), &|c| c == Channel::PhysicalDevices);
        let channels: Vec<Channel> = frames.iter().map(|(c, _)| *c).collect();
        assert_eq!(channels, vec![Channel::PhysicalDevices]);
        assert!(project_event_where(&runtime_event(), &|_| false).is_empty());
    }

    #[test]
    fn the_state_block_is_the_same_shape_the_rest_surface_serializes() {
        let (_, frame) = project_event(&runtime_event()).into_iter().next().unwrap();
        let ws: Value = serde_json::from_str(&frame).unwrap();
        let ws_keys: Vec<&String> = ws["payload"]["state"].as_object().unwrap().keys().collect();
        let rest = serde_json::to_value(crate::http::physical_device_state::state_view_to_dto(
            dali2rust_domain::registry::PhysicalDeviceStateView::default(),
        ))
        .unwrap();
        let rest_keys: Vec<&String> = rest.as_object().unwrap().keys().collect();
        assert_eq!(ws_keys, rest_keys);
    }

    #[test]
    fn a_runtime_error_is_the_product_code_not_a_discriminant() {
        let mut observation = observation();
        observation.error = Some(CompactErrorPayload::new(ErrorCode::VlUnbound, "unbound"));
        let ev = envelope(RuntimeStateChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: Some(12),
            short_address: None,
            state_setpoint: cct_setpoint(),
            state_observation: observation,
            commit_source: RuntimeSource::Api,
            commit_dimensions: cct_setpoint().dimensions(),
        });
        let (_, frame) = project_event(&ev).into_iter().next().unwrap();
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["payload"]["state"]["error"]["code"], "vl_unbound");
    }

    #[test]
    fn a_zeroed_colour_is_null_rather_than_zero_kelvin() {
        let ev = envelope(RuntimeStateChangedEvent {
            adapter_id: 0,
            virtual_lamp_id: Some(1),
            short_address: None,
            state_setpoint: LightSetpoint {
                power: PowerState::On,
                level: 100,
                color: Some(ColorValue::default()),
            },
            state_observation: RuntimeObservation::default(),
            commit_source: RuntimeSource::Api,
            commit_dimensions: LightSetpoint {
                power: PowerState::On,
                level: 100,
                color: Some(ColorValue::default()),
            }
            .dimensions(),
        });
        let (_, frame) = project_event(&ev).into_iter().next().unwrap();
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert!(v["payload"]["state"]["color_temperature_kelvin"].is_null());
        assert_eq!(v["payload"]["state"]["color_mode"], "unknown");
    }
}
