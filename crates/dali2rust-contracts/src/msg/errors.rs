use serde::{Deserialize, Serialize};

use super::bounded::{fixed_text_32, fixed_text_64, FixedBytes32, FixedText32, FixedText64};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ErrorCode {
    InvalidJson = 0,
    UnknownField = 1,
    InvalidEnum = 2,
    InvalidValue = 3,
    InvalidResourceId = 4,
    NotFound = 5,
    Conflict = 6,
    UnsupportedCapability = 7,
    UnsupportedField = 8,
    CommandsIngressOverload = 9,
    ConfirmationTimeout = 10,
    OperationFailed = 11,
    Superseded = 12,
    VlUnbound = 13,
    RegistryReset = 14,
    ExecutionFailed = 15,
    VerifyFailed = 16,
    InvalidChannel = 17,
    WsClientsExhausted = 18,
    WsOriginRejected = 19,
    DeviceAbsent = 20,
    Preempted = 21,
    VerifyUnanswered = 22,
    VerifyContended = 23,
    ReadContended = 24,
}

impl ErrorCode {
    pub const fn rest_name(self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid_json",
            Self::UnknownField => "unknown_field",
            Self::InvalidEnum => "invalid_enum",
            Self::InvalidValue => "invalid_value",
            Self::InvalidResourceId => "invalid_resource_id",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::UnsupportedCapability => "unsupported_capability",
            Self::UnsupportedField => "unsupported_field",
            Self::CommandsIngressOverload => "commands_ingress_overload",
            Self::ConfirmationTimeout => "confirmation_timeout",
            Self::OperationFailed => "operation_failed",
            Self::Superseded => "superseded",
            Self::VlUnbound => "vl_unbound",
            Self::RegistryReset => "registry_reset",
            Self::ExecutionFailed => "execution_failed",
            Self::VerifyFailed => "verify_failed",
            Self::InvalidChannel => "invalid_channel",
            Self::WsClientsExhausted => "ws_clients_exhausted",
            Self::WsOriginRejected => "ws_origin_rejected",
            Self::DeviceAbsent => "device_absent",
            Self::Preempted => "preempted",
            Self::VerifyUnanswered => "verify_unanswered",
            Self::VerifyContended => "verify_contended",
            Self::ReadContended => "read_contended",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: FixedText64,
    pub details: FixedBytes32,
}

impl ErrorPayload {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: fixed_text_64(&message.into()),
            details: FixedBytes32::default(),
        }
    }

    pub fn with_details(code: ErrorCode, message: impl Into<String>, details: &[u8]) -> Self {
        Self {
            code,
            message: fixed_text_64(&message.into()),
            details: FixedBytes32::from_slice(details),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactErrorPayload {
    pub code: ErrorCode,
    pub message: FixedText32,
}

impl CompactErrorPayload {
    #[must_use]
    pub fn is_device_absent(&self) -> bool {
        self.code == ErrorCode::DeviceAbsent
    }

    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: fixed_text_32(&message.into()),
        }
    }
}

impl From<&ErrorPayload> for CompactErrorPayload {
    fn from(full: &ErrorPayload) -> Self {
        Self {
            code: full.code,
            message: fixed_text_32(full.message.as_str()),
        }
    }
}
