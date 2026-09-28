use dali2rust_contracts::msg::{ConfirmationEnvelope, DeliveryStatus, ErrorCode};
use serde_json;

pub use dali2rust_contracts::bus::{parse_command_envelope, ParsedDaliCommandEnvelope};
pub use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

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
        decode_command_envelope, encode_command_envelope,
    };
    use dali2rust_contracts::msg::{DaliCommandPayload, DaliSetTargetStateCommand, Origin};
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
            level: None,
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
}
