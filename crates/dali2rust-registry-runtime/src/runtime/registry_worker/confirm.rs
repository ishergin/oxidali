use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, BusPublisher};
use dali2rust_contracts::bus::{
    build_confirmation_envelope, build_confirmation_envelope_with_product_error,
};
use dali2rust_contracts::msg::{DeliveryStatus, ErrorCode};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};

pub(super) fn publish_correlation_ok(publisher: &BusPublisher, correlation_id: u64) {
    if correlation_id == CORRELATION_NONE {
        return;
    }
    let bf = BusFrame::confirmation(build_confirmation_envelope(
        correlation_id,
        DeliveryStatus::Ok,
        0,
        SOURCE_ID_UNSPECIFIED,
    ));
    publish_or_drop(publisher, BusChannel::Confirmations, bf, "registry-confirm");
}

pub(super) fn publish_correlation_failed(
    publisher: &BusPublisher,
    correlation_id: u64,
    code: ErrorCode,
    message: &str,
) {
    if correlation_id == CORRELATION_NONE {
        return;
    }
    let bf = BusFrame::confirmation(build_confirmation_envelope_with_product_error(
        correlation_id,
        DeliveryStatus::ExecutionFailed,
        0,
        SOURCE_ID_UNSPECIFIED,
        Some((code, message)),
    ));
    publish_or_drop(publisher, BusChannel::Confirmations, bf, "registry-confirm");
}
