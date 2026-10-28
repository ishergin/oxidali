use super::*;
use crate::runtime::executor::write_memory_luminaire;
use dali2rust_domain::dali::banks::luminaire_write::planned_writes;
use dali2rust_domain::dali::banks::{DeviceIdentity, BANK_LUMINAIRE_INFO};

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
        let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliMemoryBankReadEvent { registry_adapter_id, short_address, bank, start_offset, chunk_index, last_chunk, data: dali2rust_contracts::msg::FixedBytes24::from_slice(chunk), indicator_unread: execution.indicator_unread });
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
    read: &BankRead<'_>,
    mb: &dali2rust_contracts::msg::DaliReadMemoryBankCommand,
) {
    let BankRead { publisher, adapter_id, correlation_id: w, short_address: sa, counters, .. } = *read;
    publish_worker_started(publisher, adapter_id, w, Origin::Internal, counters);
    let bank = mb.bank;
    let execution = read_memory_bank(controller, sa, bank, mb.start, mb.length);
    note_short_read(mb.length, &execution, bank, sa, counters);
    let delivered = publish_memory_bank_execution(read, bank, mb.start, &execution);
    let failure = execution.error.map(|error| (error.code(), error.message()));
    close_memory_bank_read(publisher, adapter_id, w, delivered, failure, counters);
    counters
        .semantic_read_memory_bank_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn close_memory_bank_read(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    delivered: bool,
    failure: Option<(ErrorCode, &'static str)>,
    counters: &DaliWorkerCounters,
) {
    match failure {
        Some((code, message)) => {
            publish_worker_failed(publisher, adapter_id, w, Origin::Internal, code, message, counters)
        }
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

// IEC 62386-102 §9.10.5
// DiiA 251 §9.2.3
pub(super) fn handle_write_memory_luminaire(
    controller: &mut impl DaliApplicationController,
    read: &BankRead<'_>,
    identity: Option<DeviceIdentity>,
    command: &dali2rust_contracts::msg::DaliWriteMemoryLuminaireCommand,
) {
    let BankRead { publisher, adapter_id, correlation_id: w, counters, .. } = *read;
    publish_worker_started(publisher, adapter_id, w, Origin::Internal, counters);
    let writes = planned_writes(&command.numbers, command.text.as_ref());
    let mut delivered = true;
    let target = (command.short_address, identity);
    let outcome = write_memory_luminaire(controller, target, &writes, |execution| {
        delivered = publish_memory_bank_execution(read, BANK_LUMINAIRE_INFO, 0, execution);
    });
    let failure = outcome.err().map(|failure| (failure.code, failure.subject));
    close_memory_bank_read(publisher, adapter_id, w, delivered, failure, counters);
}
