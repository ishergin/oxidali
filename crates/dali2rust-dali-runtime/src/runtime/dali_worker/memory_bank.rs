use super::*;

pub(super) fn publish_memory_bank_preset(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    registry_adapter_id: u8,
    short_address: u8,
    preset: dali2rust_contracts::msg::MemoryBankReadPreset,
    counters: &DaliWorkerCounters,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    crate::runtime::executor::execute_memory_bank_preset(
        controller,
        short_address,
        preset,
        |plan, execution| {
            note_short_read(plan.length, execution, plan.bank, short_address, counters);
            publish_memory_bank_execution(
                publisher,
                adapter_id,
                correlation_id,
                registry_adapter_id,
                short_address,
                plan.bank,
                plan.start,
                execution,
                counters,
            );
        },
    )
}

fn note_short_read(
    planned: u16,
    execution: &crate::runtime::executor::MemoryReadExecution,
    bank: u8,
    short_address: u8,
    counters: &DaliWorkerCounters,
) {
    let read = execution.bytes.len() as u16;
    if execution.error.is_some() || read >= planned {
        return;
    }
    counters
        .memory_bank_short_reads
        .fetch_add(1, Ordering::Relaxed);
    log::info!(
        "memory bank {bank} of short {short_address} ended at {read} of {planned} planned bytes"
    );
}

#[allow(clippy::too_many_arguments, reason = "one param per field of the event it emits")]
fn publish_memory_bank_execution(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    registry_adapter_id: u8,
    short_address: u8,
    bank: u8,
    start_offset: u16,
    execution: &crate::runtime::executor::MemoryReadExecution,
    counters: &DaliWorkerCounters,
) {
    const CHUNK: usize = 24;
    let total = execution.bytes.len();
    let mut off = 0usize;
    let mut idx: u16 = 0;
    while off < total {
        let end = (off + CHUNK).min(total);
        let chunk = &execution.bytes[off..end];
        let last = end >= total;
        let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { registry_adapter_id, short_address, bank, start_offset, chunk_index: idx, last_chunk: last && execution.error.is_none(), data: dali2rust_contracts::msg::FixedBytes24::from_slice(chunk) });
        publish_event_required(publisher, ev, counters, PublishKind::Series, "memory-bank-chunk");
        counters
            .memory_bank_chunk_events_published
            .fetch_add(1, Ordering::Relaxed);
        idx = idx.saturating_add(1);
        off = end;
    }
    if execution.error.is_some() {
        let aborted =
            dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadAbortedEvent {});
        publish_event_required(publisher, aborted, counters, PublishKind::Series, "memory-bank-aborted");
    }
}

pub(super) fn handle_read_memory_bank(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    mb: &dali2rust_contracts::msg::DaliReadMemoryBankCommand,
    counters: &DaliWorkerCounters,
) {
    let w = correlation_id;
    publish_worker_started(publisher, adapter_id, w, Origin::Internal, counters);
    let reg_aid = mb.registry_adapter_id;
    let sa = mb.short_address;
    let bank = mb.bank;
    let execution = read_memory_bank(controller, sa, bank, mb.start, mb.length);
    note_short_read(mb.length, &execution, bank, sa, counters);
    publish_memory_bank_execution(
        publisher,
        adapter_id,
        w,
        reg_aid,
        sa,
        bank,
        mb.start,
        &execution,
        counters,
    );
    match execution.error {
        Some(error) => publish_worker_failed(
            publisher,
            adapter_id,
            w,
            Origin::Internal,
            error.code(),
            error.message(),
            counters,
        ),
        None => publish_worker_succeeded(publisher, adapter_id, w, Origin::Internal, counters),
    }
    counters
        .semantic_read_memory_bank_handled
        .fetch_add(1, Ordering::Relaxed);
}
