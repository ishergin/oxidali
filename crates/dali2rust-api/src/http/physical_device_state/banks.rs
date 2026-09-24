use dali2rust_domain::registry::{MemoryBankRangeView, MemoryBankSummaryView};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct MemoryBankRangeDto {
    pub start: u16,
    pub length: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryBankSummaryDto {
    pub bank: u8,
    pub total_bytes_read: u16,
    pub last_read_ms: u64,
    pub ranges: Vec<MemoryBankRangeDto>,
}

fn memory_bank_range_view_to_dto(view: &MemoryBankRangeView) -> MemoryBankRangeDto {
    MemoryBankRangeDto {
        start: view.start,
        length: view.length,
    }
}

pub(crate) fn memory_bank_view_to_dto(view: &MemoryBankSummaryView) -> MemoryBankSummaryDto {
    MemoryBankSummaryDto {
        bank: view.bank,
        total_bytes_read: view.total_bytes_read,
        last_read_ms: view.last_read_ms,
        ranges: view.ranges.iter().map(memory_bank_range_view_to_dto).collect(),
    }
}
