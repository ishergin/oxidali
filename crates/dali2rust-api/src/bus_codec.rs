use dali2rust_contracts::msg::{
    BusEventPayload, ConfirmationEnvelope, DeliveryStatus, ErrorCode, EventEnvelope,
};
use serde_json;

pub use dali2rust_contracts::bus::{parse_command_envelope, ParsedDaliCommandEnvelope};
pub use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedBusEvent {
    Dali {
        wire_address: u8,
        command: u8,
        repeat_count: u8,
    },
    IpAssigned {
        ip: String,
    },
    RuntimeStateChanged {
        adapter_id: u8,
        correlation_id: u64,
        virtual_lamp_id: Option<u8>,
        short_address: Option<u8>,
        level: u8,
        communication_failure: bool,
    },
}

pub fn parse_event_envelope_from_typed(ev: &EventEnvelope) -> Option<ParsedBusEvent> {
    match &ev.payload {
        BusEventPayload::DaliEventPayload(p) => Some(ParsedBusEvent::Dali {
            wire_address: p.wire_address,
            command: p.command,
            repeat_count: p.repeat_count,
        }),
        BusEventPayload::IpAddressAssignedEvent(p) => Some(ParsedBusEvent::IpAssigned {
            ip: format!(
                "{}.{}.{}.{}",
                p.ip_v4[0], p.ip_v4[1], p.ip_v4[2], p.ip_v4[3]
            ),
        }),
        BusEventPayload::RuntimeStateChangedEvent(p) => {
            let communication_failure = p
                .state_observation
                .failure_status
                .as_ref()
                .is_some_and(|f| f.communication_failure);
            Some(ParsedBusEvent::RuntimeStateChanged {
                adapter_id: p.adapter_id,
                correlation_id: ev.meta.correlation_id,
                virtual_lamp_id: p.virtual_lamp_id,
                short_address: p.short_address,
                level: p.state_setpoint.level,
                communication_failure,
            })
        }
        _ => None,
    }
}

pub fn parse_event_envelope(data: &[u8]) -> Option<ParsedBusEvent> {
    let ev = dali2rust_contracts::bus::decode_event_envelope(data).ok()?;
    parse_event_envelope_from_typed(&ev)
}

fn confirmation_json_body_from_native(ce: &ConfirmationEnvelope) -> Option<Vec<u8>> {
    let status = ce.status;
    let backward = ce.confirmation.backward_frame;
    let success = status == DeliveryStatus::Ok;
    let error: Option<&str> = if success {
        None
    } else {
        Some(match status {
            DeliveryStatus::DeliveryRejected => "delivery_rejected",
            DeliveryStatus::ExecutionFailed => "execution_failed",
            DeliveryStatus::TimedOut => "timeout",
            _ => "unknown_error",
        })
    };
    let product = ce.confirmation.error.as_ref();
    let error_code = product.map(|e| product_error_code_snake(e.code));
    let message = product.map(|e| e.message.as_str());

    let mut response = serde_json::Map::new();
    response.insert("success".into(), success.into());
    response.insert("backward_frame".into(), backward.into());
    response.insert("error".into(), error.into());
    if ce.confirmation.backward_violation {
        response.insert("backward_violation".into(), true.into());
    }
    if let Some(ec) = error_code {
        response.insert("error_code".into(), ec.into());
    }
    if let Some(msg) = message {
        response.insert("message".into(), msg.into());
    }
    serde_json::to_vec(&serde_json::Value::Object(response)).ok()
}

pub fn product_error_code_snake(code: ErrorCode) -> &'static str {
    code.rest_name()
}

pub fn confirmation_to_json_body(ce: &ConfirmationEnvelope) -> Option<Vec<u8>> {
    confirmation_json_body_from_native(ce)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_bus::BusId;
    use dali2rust_contracts::bus::{
        build_confirmation_envelope_with_product_error, command_envelope,
        decode_command_envelope, encode_command_envelope, encode_event_envelope, event_envelope,
    };
    use dali2rust_contracts::msg::{
        DaliCommandPayload, DaliEventPayload, DaliSetTargetStateCommand, IpAddressAssignedEvent,
        Origin,
    };
    use dali2rust_contracts::msg::{
        BusCommandPayload, ColorMode, ColorValue, DeliveryStatus, LightSetpoint, PowerState,
    };

    #[test]
    fn confirmation_to_json_preserves_vl_unbound_product_error() {
        let ce = build_confirmation_envelope_with_product_error(
            77,
            DeliveryStatus::ExecutionFailed,
            0,
            SOURCE_ID_UNSPECIFIED,
            Some((ErrorCode::VlUnbound, "virtual lamp not bound")),
        );
        let json = confirmation_to_json_body(&ce).expect("json body");
        let v: serde_json::Value = serde_json::from_slice(&json).expect("parse json");
        assert_eq!(v["success"], false);
        assert_eq!(v["backward_frame"], 0);
        assert_eq!(v["error_code"].as_str(), Some("vl_unbound"));
        assert_eq!(v["message"].as_str(), Some("virtual lamp not bound"));
    }

    #[test]
    fn confirmation_to_json_preserves_superseded_product_error() {
        let ce = build_confirmation_envelope_with_product_error(
            88,
            DeliveryStatus::ExecutionFailed,
            0,
            SOURCE_ID_UNSPECIFIED,
            Some((ErrorCode::Superseded, "superseded by newer request")),
        );
        let json = confirmation_to_json_body(&ce).expect("json body");
        let v: serde_json::Value = serde_json::from_slice(&json).expect("parse json");
        assert_eq!(v["success"], false);
        assert_eq!(v["backward_frame"], 0);
        assert_eq!(v["error_code"].as_str(), Some("superseded"));
        assert_eq!(v["message"].as_str(), Some("superseded by newer request"));
    }

    #[test]
    fn confirmation_to_json_preserves_execution_failed_product_error() {
        let ce = build_confirmation_envelope_with_product_error(
            99,
            DeliveryStatus::ExecutionFailed,
            0,
            SOURCE_ID_UNSPECIFIED,
            Some((ErrorCode::ExecutionFailed, "execution_failed")),
        );
        let json = confirmation_to_json_body(&ce).expect("json body");
        let v: serde_json::Value = serde_json::from_slice(&json).expect("parse json");
        assert_eq!(v["success"], false);
        assert_eq!(v["error_code"].as_str(), Some("execution_failed"));
        assert_eq!(v["message"].as_str(), Some("execution_failed"));
    }

    #[test]
    fn roundtrip_dali_set_target_state_short_command_envelope() {
        let sp = LightSetpoint {
            power: PowerState::On,
            level: 0,
            color: Some(ColorValue {
                mode: ColorMode::None,
                color_temperature_kelvin: 0,
                x: 0,
                y: 0,
                r: 0,
                g: 0,
                b: 0,
                w: 0,
                a: 0,
                f: 0,
            }),
        };
        let ce =
            command_envelope(
            SOURCE_ID_UNSPECIFIED,
            42,
            1,
            Some(Origin::Api),
            DaliSetTargetStateCommand::for_short(0, 17, &sp),
        );
        let bytes = encode_command_envelope(&ce).expect("encode");
        let ce2 = decode_command_envelope(&bytes).expect("root envelope");
        assert!(matches!(
            ce2.payload,
            BusCommandPayload::DaliSetTargetStateCommand(_)
        ));
    }

    #[test]
    fn command_envelope_routes_by_target_adapter_id_not_sender_id() {
        let ce = command_envelope(
            7,
            42,
            BusId(3).0,
            None,
            DaliCommandPayload {
                wire_address: 2,
                command: 5,
                repeat_count: 1,
                raw_mode: false,
                raw_expects_backward: false,
            },
        );
        let bytes = encode_command_envelope(&ce).expect("encode");
        let command = decode_command_envelope(&bytes).expect("command envelope");
        assert_eq!(command.meta.sender_id, 7);
        assert_eq!(command.meta.target_adapter_id, 3);
        let env = parse_command_envelope(&bytes).expect("parse command envelope");
        assert_eq!(BusId(env.target_adapter_id), BusId(3));
    }

    #[test]
    fn parse_dali_bus_event_from_envelope() {
        let ev = event_envelope(
            SOURCE_ID_UNSPECIFIED,
            42,
            0,
            Some(Origin::Internal),
            DaliEventPayload {
                wire_address: 3,
                command: 9,
                repeat_count: 1,
            },
        );
        let bytes = encode_event_envelope(&ev).expect("encode");
        match parse_event_envelope(&bytes).expect("parse") {
            ParsedBusEvent::Dali {
                wire_address,
                command,
                repeat_count,
            } => {
                assert_eq!(wire_address, 3);
                assert_eq!(command, 9);
                assert_eq!(repeat_count, 1);
            }
            ParsedBusEvent::IpAssigned { .. } => panic!("expected DALI variant"),
            ParsedBusEvent::RuntimeStateChanged { .. } => panic!("expected DALI variant"),
        }
    }

    #[test]
    fn parse_ip_assigned_event_from_envelope() {
        let ev = event_envelope(
            SOURCE_ID_UNSPECIFIED,
            7,
            0,
            Some(Origin::Internal),
            IpAddressAssignedEvent::from_ip_text("192.168.4.2"),
        );
        let bytes = encode_event_envelope(&ev).expect("encode");
        match parse_event_envelope(&bytes).expect("parse") {
            ParsedBusEvent::IpAssigned { ip } => assert_eq!(ip, "192.168.4.2"),
            ParsedBusEvent::Dali { .. } => panic!("expected IpAssigned variant"),
            ParsedBusEvent::RuntimeStateChanged { .. } => panic!("expected IpAssigned variant"),
        }
    }
}
