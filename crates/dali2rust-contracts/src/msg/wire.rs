use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaliCommandPayload {
    pub wire_address: u8,
    pub command: u8,
    pub repeat_count: u8,
    pub raw_mode: bool,
    pub raw_expects_backward: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaliConfirmationPayload {
    pub backward_frame: u8,
    pub backward_violation: bool,
    pub error: Option<super::errors::ErrorPayload>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaliEventPayload {
    pub wire_address: u8,
    pub command: u8,
    pub repeat_count: u8,
}
