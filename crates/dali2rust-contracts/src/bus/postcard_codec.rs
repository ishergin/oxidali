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

fn encoded_len<T: Serialize + ?Sized>(value: &T) -> Result<usize, postcard::Error> {
    postcard::experimental::serialized_size(value)
}

pub fn encoded_len_command(value: &CommandEnvelope) -> Result<usize, postcard::Error> {
    encoded_len(value)
}

pub fn encoded_len_confirmation(value: &ConfirmationEnvelope) -> Result<usize, postcard::Error> {
    encoded_len(value)
}

pub fn encoded_len_event(value: &EventEnvelope) -> Result<usize, postcard::Error> {
    encoded_len(value)
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

#[cfg(test)]
mod encoded_len_tests {
    use super::*;
    use crate::bus::build_confirmation_envelope_with_product_error;
    use crate::msg::{DeliveryStatus, ErrorCode};

    const LONGEST_ERROR_MESSAGE: usize = 64;

    #[test]
    fn confirmation_length_matches_its_encoding() {
        let message = "x".repeat(LONGEST_ERROR_MESSAGE);
        let confirmation = build_confirmation_envelope_with_product_error(
            u64::MAX,
            DeliveryStatus::ExecutionFailed,
            u8::MAX,
            u16::MAX,
            Some((ErrorCode::InvalidValue, &message)),
        );
        let encoded = encode_confirmation_envelope(&confirmation).expect("encode");
        assert_eq!(encoded_len_confirmation(&confirmation), Ok(encoded.len()));
    }
}
