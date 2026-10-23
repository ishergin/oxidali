use super::*;

pub(super) const MEMORY_BANK_CHUNK_LOST: &str = "memory_bank_chunk_lost";

const MEMORY_BANK_CHUNK_BYTES: usize = 24;

#[derive(Clone, Copy)]
pub(super) struct BankRead<'a> {
    pub(super) publisher: &'a BusPublisher,
    pub(super) adapter_id: BusId,
    pub(super) correlation_id: u64,
    pub(super) registry_adapter_id: u8,
    pub(super) short_address: u8,
    pub(super) counters: &'a DaliWorkerCounters,
}

pub(super) fn publish_memory_bank_preset(
    controller: &mut impl DaliApplicationController,
    read: &BankRead<'_>,
    preset: dali2rust_contracts::msg::MemoryBankReadPreset,
) -> Result<bool, crate::runtime::executor::SemanticDaliError> {
    let mut delivered = true;
    crate::runtime::executor::execute_memory_bank_preset(
        controller,
        read.short_address,
        preset,
        |plan, execution| {
            note_short_read(plan.length, execution, plan.bank, read.short_address, read.counters);
            delivered =
                delivered && publish_memory_bank_execution(read, plan.bank, plan.start, execution);
        },
    )?;
    Ok(delivered)
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

pub(super) fn publish_memory_bank_execution(
    read: &BankRead<'_>,
    bank: u8,
    start_offset: u16,
    execution: &crate::runtime::executor::MemoryReadExecution,
) -> bool {
    let BankRead { publisher, adapter_id, correlation_id, registry_adapter_id, short_address, counters } = *read;
    let chunks = execution.bytes.chunks(MEMORY_BANK_CHUNK_BYTES);
    let count = chunks.len();
    let mut delivered = true;
    for (index, chunk) in chunks.enumerate() {
        let last_chunk = index + 1 == count && execution.error.is_none();
        let chunk_index = u16::try_from(index).unwrap_or(u16::MAX);
        let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { registry_adapter_id, short_address, bank, start_offset, chunk_index, last_chunk, data: dali2rust_contracts::msg::FixedBytes24::from_slice(chunk) });
        delivered = publish_event_required(publisher, ev, counters, PublishKind::Series, "memory-bank-chunk");
        if !delivered {
            break;
        }
        counters
            .memory_bank_chunk_events_published
            .fetch_add(1, Ordering::Relaxed);
    }
    if !delivered || execution.error.is_some() {
        let aborted =
            dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadAbortedEvent {});
        publish_event_required(publisher, aborted, counters, PublishKind::Series, "memory-bank-aborted");
    }
    delivered
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
    let sa = mb.short_address;
    let bank = mb.bank;
    let execution = read_memory_bank(controller, sa, bank, mb.start, mb.length);
    note_short_read(mb.length, &execution, bank, sa, counters);
    let read = BankRead {
        publisher,
        adapter_id,
        correlation_id: w,
        registry_adapter_id: mb.registry_adapter_id,
        short_address: sa,
        counters,
    };
    let delivered = publish_memory_bank_execution(&read, bank, mb.start, &execution);
    close_memory_bank_read(publisher, adapter_id, w, delivered, execution.error, counters);
    counters
        .semantic_read_memory_bank_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn close_memory_bank_read(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    delivered: bool,
    error: Option<crate::runtime::executor::SemanticDaliError>,
    counters: &DaliWorkerCounters,
) {
    match error {
        Some(error) => publish_worker_failed(
            publisher,
            adapter_id,
            w,
            Origin::Internal,
            error.code(),
            error.message(),
            counters,
        ),
        None if !delivered => fail_operation_over_lost_fact(
            publisher,
            adapter_id,
            w,
            Origin::Internal,
            MEMORY_BANK_CHUNK_LOST,
            counters,
        ),
        None => publish_worker_succeeded(publisher, adapter_id, w, Origin::Internal, counters),
    }
}
