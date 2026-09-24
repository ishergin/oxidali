use crate::bus::meta::envelope_confirmations;
use crate::msg::{
    CommandEnvelope, ConfirmationEnvelope, DaliConfirmationPayload, DeliveryStatus, ErrorCode,
    ErrorPayload,
};

pub fn synthetic_delivery_rejected_envelope(cmd: &CommandEnvelope) -> Option<ConfirmationEnvelope> {
    let correlation_id = cmd.meta.correlation_id;
    let sender_id = cmd.meta.sender_id;
    Some(build_confirmation_envelope(
        correlation_id,
        DeliveryStatus::DeliveryRejected,
        0,
        sender_id,
    ))
}

pub fn build_confirmation_envelope(
    correlation_id: u64,
    status: DeliveryStatus,
    backward_frame: u8,
    sender_id: u16,
) -> ConfirmationEnvelope {
    build_confirmation_envelope_with_product_error(
        correlation_id,
        status,
        backward_frame,
        sender_id,
        None,
    )
}

pub fn build_confirmation_envelope_violation(
    correlation_id: u64,
    status: DeliveryStatus,
    sender_id: u16,
) -> ConfirmationEnvelope {
    build_confirmation(correlation_id, status, 0, true, sender_id, None)
}

pub fn build_confirmation_envelope_with_product_error(
    correlation_id: u64,
    status: DeliveryStatus,
    backward_frame: u8,
    sender_id: u16,
    product_error: Option<(ErrorCode, &str)>,
) -> ConfirmationEnvelope {
    build_confirmation(
        correlation_id,
        status,
        backward_frame,
        false,
        sender_id,
        product_error,
    )
}

fn build_confirmation(
    correlation_id: u64,
    status: DeliveryStatus,
    backward_frame: u8,
    backward_violation: bool,
    sender_id: u16,
    product_error: Option<(ErrorCode, &str)>,
) -> ConfirmationEnvelope {
    let meta = envelope_confirmations(
        sender_id,
        correlation_id,
        Some(crate::msg::Origin::Internal),
    );
    let err = product_error.map(|(code, msg)| ErrorPayload::new(code, msg));
    let confirmation = DaliConfirmationPayload {
        backward_frame,
        backward_violation,
        error: err,
    };
    ConfirmationEnvelope {
        meta,
        status,
        confirmation,
    }
}
