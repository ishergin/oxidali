use crate::http::diagnostics_state::{DiagnosticsDto, DiagnosticsHttpState};
use crate::http::stats_state::{StatsHttpState, StatsReportDto};
use crate::ws::protocol::{event_frame_reserving, Channel};

pub const DIAGNOSTICS_SNAPSHOT_CEILING_BYTES: usize = 8960;

pub const STATS_SNAPSHOT_RESERVE_BYTES: usize = 3072;

pub fn stats_snapshot_frame(state: &dyn StatsHttpState, ts_ms: u64) -> String {
    let dto = boxed_stats(state);
    event_frame_reserving(
        "StatsSnapshot",
        Channel::Stats,
        ts_ms,
        dto.as_ref(),
        STATS_SNAPSHOT_RESERVE_BYTES,
    )
}

#[inline(never)]
fn boxed_stats(state: &dyn StatsHttpState) -> Box<StatsReportDto> {
    let mut dto: Box<StatsReportDto> = Box::default();
    state.stats_dto_into(&mut dto);
    dto
}

pub fn diagnostics_snapshot_frame(state: &dyn DiagnosticsHttpState, ts_ms: u64) -> String {
    let dto = boxed_diagnostics(state);
    event_frame_reserving(
        "DiagnosticsSnapshot",
        Channel::Diagnostics,
        ts_ms,
        &*dto,
        DIAGNOSTICS_SNAPSHOT_CEILING_BYTES,
    )
}

#[inline(never)]
fn boxed_diagnostics(state: &dyn DiagnosticsHttpState) -> Box<DiagnosticsDto> {
    let mut dto: Box<DiagnosticsDto> = Box::default();
    state.diagnostics_dto_into(&mut dto);
    dto
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::diagnostics_state::{DiagnosticsDto, DiagnosticsHttpState};

    struct WorstCase;
    impl DiagnosticsHttpState for WorstCase {
        fn diagnostics_dto(&self) -> DiagnosticsDto {
            DiagnosticsDto::widest()
        }
    }

    #[test]
    fn the_diagnostics_snapshot_worst_case_is_measured_and_frozen() {
        let frame = diagnostics_snapshot_frame(&WorstCase, u64::MAX);
        eprintln!(
            "worst-case DiagnosticsSnapshot: {} B of {} B ceiling, DTO {} B by value",
            frame.len(),
            DIAGNOSTICS_SNAPSHOT_CEILING_BYTES,
            core::mem::size_of::<DiagnosticsDto>()
        );
        assert!(
            frame.len() <= DIAGNOSTICS_SNAPSHOT_CEILING_BYTES,
            "worst-case DiagnosticsSnapshot measures {} B against the frozen \
             {} B ceiling — every added byte is periodic load on the single \
             httpd task, so shrink the DTO or argue the ceiling in a PR",
            frame.len(),
            DIAGNOSTICS_SNAPSHOT_CEILING_BYTES
        );
        assert!(
            core::mem::size_of::<DiagnosticsDto>() < 1024,
            "DiagnosticsDto is {} B by value — approaching the point where \
             returning it on the ws_worker stack needs a rethink",
            core::mem::size_of::<DiagnosticsDto>()
        );
    }

    #[test]
    fn the_diagnostics_snapshot_is_built_without_a_growing_buffer() {
        use crate::ws::protocol::ENVELOPE_RESERVE_BYTES;

        let frame = diagnostics_snapshot_frame(&WorstCase, u64::MAX);
        assert!(
            frame.capacity() <= frame.len() + ENVELOPE_RESERVE_BYTES,
            "envelope capacity {} B for a {} B frame: the buffer grew instead of \
             being reserved",
            frame.capacity(),
            frame.len()
        );
    }
}
