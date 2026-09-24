use serde::Serialize;

use crate::msg::{CommandEnvelope, ConfirmationEnvelope, EventEnvelope};

pub const MAX_BUS_WIRE_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireBudgetError {
    TooLarge { len: usize, max: usize },
}

pub fn enforce_bus_wire_budget(len: usize) -> Result<(), WireBudgetError> {
    if len > MAX_BUS_WIRE_BYTES {
        Err(WireBudgetError::TooLarge {
            len,
            max: MAX_BUS_WIRE_BYTES,
        })
    } else {
        Ok(())
    }
}

fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(value)
}

pub fn encode_command_envelope(value: &CommandEnvelope) -> Result<Vec<u8>, postcard::Error> {
    encode(value)
}

pub fn encode_confirmation_envelope(
    value: &ConfirmationEnvelope,
) -> Result<Vec<u8>, postcard::Error> {
    encode(value)
}

pub fn encode_event_envelope(value: &EventEnvelope) -> Result<Vec<u8>, postcard::Error> {
    encode(value)
}

const ENCODED_LEN_BUF: usize = 512;

pub fn encoded_len_command(value: &CommandEnvelope) -> Result<usize, postcard::Error> {
    let mut buf = [0u8; ENCODED_LEN_BUF];
    postcard::to_slice(value, &mut buf).map(|s| s.len())
}

pub fn encoded_len_confirmation(value: &ConfirmationEnvelope) -> Result<usize, postcard::Error> {
    let mut buf = [0u8; ENCODED_LEN_BUF];
    postcard::to_slice(value, &mut buf).map(|s| s.len())
}

pub fn encoded_len_event(value: &EventEnvelope) -> Result<usize, postcard::Error> {
    let mut buf = [0u8; ENCODED_LEN_BUF];
    postcard::to_slice(value, &mut buf).map(|s| s.len())
}

pub fn decode_command_envelope(bytes: &[u8]) -> Result<CommandEnvelope, postcard::Error> {
    postcard::from_bytes(bytes)
}

pub fn decode_confirmation_envelope(bytes: &[u8]) -> Result<ConfirmationEnvelope, postcard::Error> {
    postcard::from_bytes(bytes)
}

pub fn decode_event_envelope(bytes: &[u8]) -> Result<EventEnvelope, postcard::Error> {
    postcard::from_bytes(bytes)
}
