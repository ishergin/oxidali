use std::sync::atomic::Ordering;

use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, BusPublisher};
use dali2rust_contracts::bus::{
    build_confirmation_envelope, build_confirmation_envelope_with_product_error,
};
use dali2rust_contracts::msg::{DeliveryStatus, ErrorCode};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};

use super::RegistryCommandCounters;

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

pub(super) fn check_primary_adapter(
    publisher: &BusPublisher,
    target_id: u16,
    primary: dali2rust_bus::BusId,
    corr: u64,
) -> Result<(), ()> {
    if dali2rust_bus::BusId(target_id) != primary {
        publish_correlation_failed(
            publisher,
            corr,
            ErrorCode::InvalidResourceId,
            "wrong_target_adapter",
        );
        Err(())
    } else {
        Ok(())
    }
}

pub(super) fn handle_global_settings_update<A>(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    counters: &RegistryCommandCounters,
    applied_counter: &std::sync::atomic::AtomicU32,
    apply: impl FnOnce() -> A,
    publish_changed: impl FnOnce(&BusPublisher, u64, u16, &A),
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err() {
        return;
    }
    let applied = apply();
    publish_changed(publisher, corr, tid, &applied);
    applied_counter.fetch_add(1, Ordering::Relaxed);
    counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}

pub(super) fn check_adapter_id(
    publisher: &BusPublisher,
    counters: &RegistryCommandCounters,
    aid: u8,
    adapter_count: u8,
    corr: u64,
) -> Result<(), ()> {
    if aid >= adapter_count {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        publish_correlation_failed(
            publisher,
            corr,
            ErrorCode::InvalidResourceId,
            "adapter_id_out_of_range",
        );
        Err(())
    } else {
        Ok(())
    }
}
