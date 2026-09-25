use core::fmt::Write as _;

use dali2rust_contracts::msg::ErrorCode;
use dali2rust_platform::logs::LogLevel;
use serde::Serialize;

use crate::bus_codec::product_error_code_snake;

pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Channel {
    Adapters,
    PhysicalDevices,
    VirtualLamps,
    Groups,
    Scenes,
    Operations,
    Stats,
    Diagnostics,
    Sniffer,
    Input,
    Rules,
    Logs,
}

pub const V1_CHANNELS: &[Channel] = &[
    Channel::Adapters,
    Channel::PhysicalDevices,
    Channel::VirtualLamps,
    Channel::Groups,
    Channel::Scenes,
    Channel::Operations,
    Channel::Stats,
    Channel::Diagnostics,
    Channel::Sniffer,
    Channel::Input,
    Channel::Rules,
    Channel::Logs,
];

impl Channel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Adapters => "adapters",
            Self::PhysicalDevices => "physical_devices",
            Self::VirtualLamps => "virtual_lamps",
            Self::Groups => "groups",
            Self::Scenes => "scenes",
            Self::Operations => "operations",
            Self::Stats => "stats",
            Self::Diagnostics => "diagnostics",
            Self::Sniffer => "sniffer",
            Self::Input => "input",
            Self::Rules => "rules",
            Self::Logs => "logs",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        V1_CHANNELS.iter().copied().find(|c| c.as_str() == name)
    }

    pub const fn bit(self) -> u16 {
        1u16 << (self as u16)
    }

    pub fn slot(self) -> usize {
        let found = V1_CHANNELS.iter().position(|c| *c == self);
        debug_assert!(found.is_some(), "{self:?} is missing from V1_CHANNELS");
        found.unwrap_or(0)
    }
}

const _: () = assert!(
    V1_CHANNELS.len() <= u16::BITS as usize,
    "Client::subscriptions is u16; a 17th channel aliases bit 0"
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientRequest {
    Subscribe {
        channels: Vec<Channel>,
        log_level: Option<LogLevel>,
    },
    Unsubscribe(Vec<Channel>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestError {
    pub code: ErrorCode,
    pub message: String,
}

pub fn parse_client_request(text: &str) -> Result<ClientRequest, RequestError> {
    if crate::json_depth::json_too_deep(text.as_bytes()) {
        return Err(RequestError {
            code: ErrorCode::InvalidJson,
            message: "frame nests too deep".to_string(),
        });
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| RequestError {
        code: ErrorCode::InvalidJson,
        message: "frame is not JSON".to_string(),
    })?;
    let op = value.get("op").and_then(|v| v.as_str()).unwrap_or_default();
    match op {
        "subscribe" => Ok(ClientRequest::Subscribe {
            channels: parse_channels(&value)?,
            log_level: parse_log_level(&value)?,
        }),
        "unsubscribe" => parse_channels(&value).map(ClientRequest::Unsubscribe),
        other => Err(RequestError {
            code: ErrorCode::InvalidValue,
            message: format!("unknown op: {other}"),
        }),
    }
}

fn parse_channels(value: &serde_json::Value) -> Result<Vec<Channel>, RequestError> {
    let Some(list) = value.get("channels").and_then(|v| v.as_array()) else {
        return Err(RequestError {
            code: ErrorCode::InvalidValue,
            message: "channels must be an array".to_string(),
        });
    };
    let mut out = Vec::with_capacity(list.len());
    for entry in list {
        let name = entry.as_str().unwrap_or_default();
        let Some(channel) = Channel::parse(name) else {
            return Err(RequestError {
                code: ErrorCode::InvalidChannel,
                message: format!("unknown channel: {name}"),
            });
        };
        out.push(channel);
    }
    Ok(out)
}

fn parse_log_level(value: &serde_json::Value) -> Result<Option<LogLevel>, RequestError> {
    let Some(requested) = value.get("logs").and_then(|logs| logs.get("min_level")) else {
        return Ok(None);
    };
    let name = requested.as_str().unwrap_or_default();
    LogLevel::parse(name).map(Some).ok_or_else(|| RequestError {
        code: ErrorCode::InvalidValue,
        message: format!("unknown log level: {name}"),
    })
}

fn channel_names(channels: &[Channel]) -> Vec<&'static str> {
    channels.iter().map(|c| c.as_str()).collect()
}

pub fn hello_frame() -> String {
    serde_json::json!({
        "op": "hello",
        "protocol": PROTOCOL_VERSION,
        "channels": channel_names(V1_CHANNELS),
    })
    .to_string()
}

#[derive(Clone, Copy, Debug)]
pub enum SubscriptionAck {
    Subscribed,
    Unsubscribed,
}

impl SubscriptionAck {
    const fn op(self) -> &'static str {
        match self {
            Self::Subscribed => "subscribed",
            Self::Unsubscribed => "unsubscribed",
        }
    }
}

pub fn subscription_ack_frame(ack: SubscriptionAck, channels: &[Channel]) -> String {
    serde_json::json!({
        "op": ack.op(),
        "channels": channel_names(channels),
    })
    .to_string()
}

pub fn error_frame(code: ErrorCode, message: &str) -> String {
    serde_json::json!({
        "op": "error",
        "error": { "code": product_error_code_snake(code), "message": message },
    })
    .to_string()
}

#[derive(Clone, Copy, Debug)]
pub struct DropNotice {
    pub channel: Option<Channel>,
    pub dropped_count: u32,
}

impl DropNotice {
    pub fn to_frame(self) -> String {
        serde_json::json!({
            "type": "DropNotice",
            "channel": self.channel.map_or("*", Channel::as_str),
            "dropped_count": self.dropped_count,
        })
        .to_string()
    }
}

const DEFAULT_PAYLOAD_RESERVE_BYTES: usize = 512;

pub(crate) const ENVELOPE_RESERVE_BYTES: usize = 128;

pub fn event_frame<T: Serialize>(event_type: &str, channel: Channel, ts_ms: u64, payload: T) -> String {
    event_frame_reserving(event_type, channel, ts_ms, payload, DEFAULT_PAYLOAD_RESERVE_BYTES)
}

pub fn event_frame_reserving<T: Serialize>(
    event_type: &str,
    channel: Channel,
    ts_ms: u64,
    payload: T,
    payload_reserve_bytes: usize,
) -> String {
    let mut buf = Vec::with_capacity(payload_reserve_bytes);
    let payload = match serde_json::to_writer(&mut buf, &payload) {
        Ok(()) => String::from_utf8(buf).unwrap_or_else(|_| "null".into()),
        Err(_) => "null".into(),
    };
    event_frame_with_payload(event_type, channel, ts_ms, &payload)
}

pub fn event_frame_with_payload(
    event_type: &str,
    channel: Channel,
    ts_ms: u64,
    payload_json: &str,
) -> String {
    let mut frame = String::with_capacity(payload_json.len() + ENVELOPE_RESERVE_BYTES);
    let _ = write!(
        frame,
        r#"{{"channel":"{}","payload":{},"ts_ms":{},"type":"{}"}}"#,
        channel.as_str(),
        payload_json,
        ts_ms,
        event_type
    );
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channels_bit_is_its_position_in_the_served_list() {
        for (index, channel) in V1_CHANNELS.iter().enumerate() {
            assert_eq!(
                channel.bit(),
                1u16 << index,
                "{} sits at V1_CHANNELS[{index}] but its bit is the enum's {:?}",
                channel.as_str(),
                channel.bit(),
            );
            assert_eq!(channel.slot(), index, "{}", channel.as_str());
        }
    }

    #[test]
    fn a_hand_built_envelope_matches_the_serializer_byte_for_byte() {
        let payload = serde_json::json!({
            "adapter_id": 0,
            "text": "a \"quoted\" value, and a comma",
            "nested": {"level": 254, "list": [1, 2, 3]},
            "null_field": serde_json::Value::Null,
        });
        let payload_json = serde_json::to_string(&payload).expect("payload");
        for channel in V1_CHANNELS {
            let want = serde_json::json!({
                "type": "RuntimeStateChanged",
                "channel": channel.as_str(),
                "ts_ms": 1_234_567_890_u64,
                "payload": payload,
            })
            .to_string();
            let got = event_frame_with_payload(
                "RuntimeStateChanged",
                *channel,
                1_234_567_890,
                &payload_json,
            );
            assert_eq!(got, want, "channel {}", channel.as_str());
        }
    }

    #[test]
    fn hello_lists_exactly_the_channels_this_build_serves() {
        let v: serde_json::Value = serde_json::from_str(&hello_frame()).unwrap();
        assert_eq!(v["op"], "hello");
        assert_eq!(v["protocol"], 1);
        let names: Vec<String> = serde_json::from_value(v["channels"].clone()).unwrap();
        assert_eq!(
            names,
            vec![
                "adapters",
                "physical_devices",
                "virtual_lamps",
                "groups",
                "scenes",
                "operations",
                "stats",
                "diagnostics",
                "sniffer",
                "input",
                "rules",
                "logs",
            ]
        );
    }

    #[test]
    fn subscribe_parses_known_channels() {
        let req = parse_client_request(r#"{"op":"subscribe","channels":["virtual_lamps","groups"]}"#)
            .unwrap();
        assert_eq!(
            req,
            ClientRequest::Subscribe {
                channels: vec![Channel::VirtualLamps, Channel::Groups],
                log_level: None,
            }
        );
    }

    #[test]
    fn a_logs_subscription_carries_the_level_it_asked_for() {
        let req = parse_client_request(
            r#"{"op":"subscribe","channels":["logs"],"logs":{"min_level":"debug"}}"#,
        )
        .unwrap();
        assert_eq!(
            req,
            ClientRequest::Subscribe {
                channels: vec![Channel::Logs],
                log_level: Some(LogLevel::Debug),
            }
        );
    }

    #[test]
    fn an_unknown_level_rejects_the_whole_request() {
        let err = parse_client_request(
            r#"{"op":"subscribe","channels":["logs"],"logs":{"min_level":"waming"}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidValue);
    }

    #[test]
    fn one_unknown_channel_rejects_the_whole_request() {
        let err =
            parse_client_request(r#"{"op":"subscribe","channels":["groups","settings_network"]}"#)
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidChannel);
        assert!(err.message.contains("settings_network"));
    }

    #[test]
    fn a_non_json_frame_is_an_error_not_a_panic() {
        let err = parse_client_request("not json at all").unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidJson);
    }

    #[test]
    fn an_over_deep_frame_is_refused_before_parsing() {
        let deep = crate::json_depth::nested_json(crate::json_depth::MAX_JSON_DEPTH + 1);
        let err = parse_client_request(std::str::from_utf8(&deep).unwrap()).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidJson);
    }

    #[test]
    fn unknown_ops_are_named_in_the_error() {
        let err = parse_client_request(r#"{"op":"publish","channels":[]}"#).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidValue);
        assert!(err.message.contains("publish"));
    }

    #[test]
    fn error_frames_use_the_shared_snake_case_code_names() {
        let f = error_frame(ErrorCode::WsClientsExhausted, "cap reached");
        let v: serde_json::Value = serde_json::from_str(&f).unwrap();
        assert_eq!(v["op"], "error");
        assert_eq!(v["error"]["code"], "ws_clients_exhausted");
    }

    #[test]
    fn inbox_overflow_reports_the_wildcard_channel() {
        let v: serde_json::Value = serde_json::from_str(
            &DropNotice {
                channel: None,
                dropped_count: 7,
            }
            .to_frame(),
        )
        .unwrap();
        assert_eq!(v["type"], "DropNotice");
        assert_eq!(v["channel"], "*");
        assert_eq!(v["dropped_count"], 7);
    }

}
