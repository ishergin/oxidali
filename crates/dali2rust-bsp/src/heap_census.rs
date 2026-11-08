use core::fmt::{Display, Formatter, Result as FmtResult};

pub const BUCKET_COUNT: usize = 12;
pub const SMALLEST_BUCKET_BYTES: u32 = 16;
pub const EXACT_STEP_BYTES: u32 = 4;
pub const EXACT_SLOT_COUNT: usize = 256;
pub const EXACT_MAX_BYTES: u32 = (EXACT_SLOT_COUNT as u32 - 1) * EXACT_STEP_BYTES;
pub const SPAN_COUNT: usize = 6;
pub const TOP_EXACT_COUNT: usize = 24;
pub const TOPS_PER_LINE: usize = 8;
pub const BUCKETS_PER_LINE: usize = 4;
pub const SPANS_PER_LINE: usize = 2;
pub const STAGE_SLOTS: usize = 8;
pub const STAGE_NAME_BYTES: usize = 24;
pub const RUST_CLASSES: usize = crate::rust_heap::SIZE_CLASSES;
pub const HOLD_BUDGET_US: u32 = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpanTally {
    pub span: Span,
    pub used_blocks: u32,
    pub used_bytes: u32,
    pub free_blocks: u32,
    pub free_bytes: u32,
    pub opened_us: u32,
    pub hold_us: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapCensus {
    pub used_blocks: u32,
    pub used_bytes: u32,
    pub free_blocks: u32,
    pub free_bytes: u32,
    pub bucket_blocks: [u32; BUCKET_COUNT],
    pub bucket_bytes: [u32; BUCKET_COUNT],
    pub exact_blocks: [u32; EXACT_SLOT_COUNT],
    pub off_table_blocks: u32,
    pub off_table_bytes: u32,
    pub spans: [SpanTally; SPAN_COUNT],
    pub spans_seen: u32,
    pub last_span: Span,
    pub seen_blocks: u32,
    pub cut_heaps: u32,
    pub hold_budget_us: u32,
    pub top: [(u32, u32); TOP_EXACT_COUNT],
}

impl Default for HeapCensus {
    fn default() -> Self {
        Self {
            used_blocks: 0,
            used_bytes: 0,
            free_blocks: 0,
            free_bytes: 0,
            bucket_blocks: [0; BUCKET_COUNT],
            bucket_bytes: [0; BUCKET_COUNT],
            exact_blocks: [0; EXACT_SLOT_COUNT],
            off_table_blocks: 0,
            off_table_bytes: 0,
            spans: [SpanTally::default(); SPAN_COUNT],
            spans_seen: 0,
            last_span: Span { start: 0, end: 0 },
            seen_blocks: 0,
            cut_heaps: 0,
            hold_budget_us: HOLD_BUDGET_US,
            top: [(0, 0); TOP_EXACT_COUNT],
        }
    }
}

impl HeapCensus {
    pub fn record(&mut self, span: Span, size: u32, used: bool) {
        self.seen_blocks = self.seen_blocks.wrapping_add(1);
        let at = self.span_slot(span);
        if used {
            self.add_used(at, size);
        } else {
            self.add_free(at, size);
        }
    }

    pub fn budget_us(&self) -> u32 {
        if self.hold_budget_us == 0 {
            HOLD_BUDGET_US
        } else {
            self.hold_budget_us
        }
    }

    pub fn hold_is_spent(&self, now_us: u32) -> bool {
        let open = self.span_count();
        let Some(slot) = self.spans.get(open.wrapping_sub(1)) else {
            return false;
        };
        open > 0 && now_us.wrapping_sub(slot.opened_us) > self.budget_us()
    }

    pub fn cut_this_heap(&mut self) {
        self.cut_heaps = self.cut_heaps.wrapping_add(1);
    }

    pub fn stamp_span_open(&mut self, at_us: u32) {
        let open = self.span_count();
        if open > 0 {
            if let Some(slot) = self.spans.get_mut(open - 1) {
                slot.opened_us = at_us;
            }
        }
    }

    pub fn close_span_holds(&mut self, end_us: u32) {
        for at in 0..self.span_count() {
            let next = self
                .spans
                .get(at + 1)
                .filter(|_| at + 1 < self.span_count())
                .map(|slot| slot.opened_us)
                .unwrap_or(end_us);
            let opened = self.spans.get(at).map(|slot| slot.opened_us).unwrap_or(0);
            if let Some(slot) = self.spans.get_mut(at) {
                slot.hold_us = next.wrapping_sub(opened);
            }
        }
    }

    pub fn longest_hold_us(&self) -> u32 {
        self.spans
            .iter()
            .take(self.span_count())
            .map(|slot| slot.hold_us)
            .max()
            .unwrap_or(0)
    }

    pub fn rank_top(&mut self) {
        self.top = [(0, 0); TOP_EXACT_COUNT];
        for slot in 0..EXACT_SLOT_COUNT {
            let blocks = self.exact_blocks.get(slot).copied().unwrap_or(0);
            let size = (slot as u32).wrapping_mul(EXACT_STEP_BYTES);
            rank_in(&mut self.top, size, blocks);
        }
    }

    pub fn bucket_sums(&self) -> (u32, u32) {
        let blocks = self.bucket_blocks.iter().fold(0u32, |a, b| a.wrapping_add(*b));
        let bytes = self.bucket_bytes.iter().fold(0u32, |a, b| a.wrapping_add(*b));
        (blocks, bytes)
    }

    pub fn span_count(&self) -> usize {
        (self.spans_seen as usize).min(SPAN_COUNT)
    }

    fn span_slot(&mut self, span: Span) -> usize {
        let open = (self.spans_seen as usize).min(SPAN_COUNT);
        if self.spans_seen > 0 && self.last_span == span {
            return open - 1;
        }
        self.last_span = span;
        self.spans_seen = self.spans_seen.wrapping_add(1);
        let at = (self.spans_seen as usize).min(SPAN_COUNT) - 1;
        if self.spans_seen as usize <= SPAN_COUNT {
            if let Some(slot) = self.spans.get_mut(at) {
                *slot = SpanTally { span, ..SpanTally::default() };
            }
        }
        at
    }

    fn add_used(&mut self, at: usize, size: u32) {
        self.used_blocks = self.used_blocks.wrapping_add(1);
        self.used_bytes = self.used_bytes.wrapping_add(size);
        if let Some(slot) = self.spans.get_mut(at) {
            slot.used_blocks = slot.used_blocks.wrapping_add(1);
            slot.used_bytes = slot.used_bytes.wrapping_add(size);
        }
        let bucket = bucket_of(size);
        if let Some(slot) = self.bucket_blocks.get_mut(bucket) {
            *slot = slot.wrapping_add(1);
        }
        if let Some(slot) = self.bucket_bytes.get_mut(bucket) {
            *slot = slot.wrapping_add(size);
        }
        self.note_exact(size);
    }

    fn add_free(&mut self, at: usize, size: u32) {
        self.free_blocks = self.free_blocks.wrapping_add(1);
        self.free_bytes = self.free_bytes.wrapping_add(size);
        if let Some(slot) = self.spans.get_mut(at) {
            slot.free_blocks = slot.free_blocks.wrapping_add(1);
            slot.free_bytes = slot.free_bytes.wrapping_add(size);
        }
    }

    fn note_exact(&mut self, size: u32) {
        match exact_slot(size).and_then(|at| self.exact_blocks.get_mut(at)) {
            Some(slot) => *slot = slot.wrapping_add(1),
            None => {
                self.off_table_blocks = self.off_table_blocks.wrapping_add(1);
                self.off_table_bytes = self.off_table_bytes.wrapping_add(size);
            }
        }
    }
}

pub fn exact_slot(size: u32) -> Option<usize> {
    if size > EXACT_MAX_BYTES || size % EXACT_STEP_BYTES != 0 {
        return None;
    }
    Some((size / EXACT_STEP_BYTES) as usize)
}

pub fn bucket_of(size: u32) -> usize {
    let last = BUCKET_COUNT - 1;
    if size > SMALLEST_BUCKET_BYTES << (last - 1) {
        return last;
    }
    let upper = size.max(1).next_power_of_two().max(SMALLEST_BUCKET_BYTES);
    (upper.trailing_zeros() - SMALLEST_BUCKET_BYTES.trailing_zeros()) as usize
}

fn rank_in(top: &mut [(u32, u32); TOP_EXACT_COUNT], size: u32, blocks: u32) {
    let bytes = size.saturating_mul(blocks);
    if bytes == 0 {
        return;
    }
    let Some(at) = top.iter().position(|(s, b)| bytes > s.saturating_mul(*b)) else {
        return;
    };
    let mut carry = (size, blocks);
    for cell in top.iter_mut().skip(at) {
        core::mem::swap(cell, &mut carry);
    }
}

#[derive(Clone, Copy)]
pub struct StageRecord {
    pub name: [u8; STAGE_NAME_BYTES],
    pub name_len: u32,
    pub blocks_before: u32,
    pub blocks_after: u32,
    pub walk_us: u32,
    pub info_us: u32,
    pub rust_internal_bytes: u32,
    pub rust_psram_bytes: u32,
    pub rust_by_class: [u32; RUST_CLASSES],
    pub census: HeapCensus,
}

impl StageRecord {
    pub fn blank() -> Self {
        Self {
            name: [0; STAGE_NAME_BYTES],
            name_len: 0,
            blocks_before: 0,
            blocks_after: 0,
            walk_us: 0,
            info_us: 0,
            rust_internal_bytes: 0,
            rust_psram_bytes: 0,
            rust_by_class: [0; RUST_CLASSES],
            census: HeapCensus::default(),
        }
    }

    pub fn stage(&self) -> &str {
        let len = (self.name_len as usize).min(STAGE_NAME_BYTES);
        core::str::from_utf8(self.name.get(..len).unwrap_or(&[])).unwrap_or("?")
    }

    pub fn set_stage(&mut self, stage: &str) {
        self.name = [0; STAGE_NAME_BYTES];
        let bytes = stage.as_bytes();
        let len = bytes.len().min(STAGE_NAME_BYTES);
        if let (Some(into), Some(from)) = (self.name.get_mut(..len), bytes.get(..len)) {
            into.copy_from_slice(from);
        }
        self.name_len = len as u32;
    }
}

pub struct BootCensusLog {
    pub records: [StageRecord; STAGE_SLOTS],
    pub used: u32,
    pub emitted: u32,
}

impl BootCensusLog {
    pub fn open_stage(&mut self, stage: &str) -> Option<&mut StageRecord> {
        let at = self.used as usize;
        let record = self.records.get_mut(at)?;
        self.used = self.used.wrapping_add(1);
        *record = StageRecord::blank();
        record.set_stage(stage);
        Some(record)
    }

    pub fn pending(&self) -> usize {
        let total = (self.used as usize).saturating_mul(lines_per_stage());
        total.saturating_sub(self.emitted as usize)
    }

    pub fn take_line(&mut self) -> Option<(&StageRecord, usize)> {
        if self.pending() == 0 {
            return None;
        }
        let at = self.emitted as usize;
        self.emitted = self.emitted.wrapping_add(1);
        let record = self.records.get(at / lines_per_stage())?;
        Some((record, at % lines_per_stage()))
    }
}

pub fn lines_per_stage() -> usize {
    2 + bucket_line_count() + top_line_count() + SPAN_COUNT.div_ceil(SPANS_PER_LINE)
}

pub enum CensusText<'a> {
    Census(CensusLine<'a>),
    Buckets(BucketLine<'a>),
    Top(TopLine<'a>),
    Spans(SpanLine<'a>),
    Rust(RustLine<'a>),
    Nothing,
}

impl Display for CensusText<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Census(line) => line.fmt(f),
            Self::Buckets(line) => line.fmt(f),
            Self::Top(line) => line.fmt(f),
            Self::Spans(line) => line.fmt(f),
            Self::Rust(line) => line.fmt(f),
            Self::Nothing => Ok(()),
        }
    }
}

pub fn line_of(record: &StageRecord, at: usize) -> CensusText<'_> {
    let stage = record.stage();
    let census = &record.census;
    let buckets = bucket_line_count();
    let tops = top_line_count();
    if at == 0 {
        return CensusText::Census(CensusLine { stage, record });
    }
    if at <= buckets {
        return CensusText::Buckets(BucketLine { stage, census, chunk: at - 1 });
    }
    if at <= buckets + tops {
        return CensusText::Top(TopLine { stage, census, chunk: at - 1 - buckets });
    }
    if at == buckets + tops + 1 {
        return CensusText::Rust(rust_line_of(record));
    }
    span_line_at(record, at - buckets - tops - 2)
}

fn rust_line_of(record: &StageRecord) -> RustLine<'_> {
    RustLine {
        stage: record.stage(),
        internal_bytes: record.rust_internal_bytes,
        psram_bytes: record.rust_psram_bytes,
        by_class: &record.rust_by_class,
    }
}

fn span_line_at(record: &StageRecord, chunk: usize) -> CensusText<'_> {
    if chunk >= span_line_count(&record.census) {
        return CensusText::Nothing;
    }
    CensusText::Spans(SpanLine {
        stage: record.stage(),
        census: &record.census,
        chunk,
    })
}

pub struct CensusLine<'a> {
    pub stage: &'a str,
    pub record: &'a StageRecord,
}

impl Display for CensusLine<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let census = &self.record.census;
        write!(
            f,
            "boot heap census [{}]: used={}/{} free={}/{}",
            self.stage,
            census.used_blocks,
            census.used_bytes,
            census.free_blocks,
            census.free_bytes
        )?;
        let (sum_blocks, sum_bytes) = census.bucket_sums();
        write!(
            f,
            " ref={}/{} us={}/{} off={}/{} sum={}/{}",
            self.record.blocks_before,
            self.record.blocks_after,
            self.record.walk_us,
            self.record.info_us,
            census.off_table_blocks,
            census.off_table_bytes,
            sum_blocks,
            sum_bytes
        )
    }
}

pub struct RustLine<'a> {
    pub stage: &'a str,
    pub internal_bytes: u32,
    pub psram_bytes: u32,
    pub by_class: &'a [u32],
}

impl Display for RustLine<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "boot heap rust [{}]: internal={} psram={}",
            self.stage, self.internal_bytes, self.psram_bytes
        )?;
        for (at, bytes) in self.by_class.iter().enumerate() {
            write!(f, " c{at}:{bytes}")?;
        }
        Ok(())
    }
}

pub struct BucketLine<'a> {
    pub stage: &'a str,
    pub census: &'a HeapCensus,
    pub chunk: usize,
}

impl Display for BucketLine<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "boot heap sizes [{}] {}/{}:",
            self.stage,
            self.chunk,
            bucket_line_count()
        )?;
        let from = self.chunk * BUCKETS_PER_LINE;
        for at in from..(from + BUCKETS_PER_LINE).min(BUCKET_COUNT) {
            let blocks = self.census.bucket_blocks.get(at).copied().unwrap_or(0);
            let bytes = self.census.bucket_bytes.get(at).copied().unwrap_or(0);
            write!(f, " b{at}:{blocks}/{bytes}")?;
        }
        Ok(())
    }
}

pub struct SpanLine<'a> {
    pub stage: &'a str,
    pub census: &'a HeapCensus,
    pub chunk: usize,
}

impl Display for SpanLine<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "boot heap spans [{}] {}/{} seen={} cut={} hold<={}us:",
            self.stage,
            self.chunk,
            span_line_count(self.census),
            self.census.spans_seen,
            self.census.cut_heaps,
            self.census.budget_us()
        )?;
        let from = self.chunk * SPANS_PER_LINE;
        for at in from..(from + SPANS_PER_LINE).min(self.census.span_count()) {
            let Some(tally) = self.census.spans.get(at) else {
                continue;
            };
            write!(
                f,
                " {at}@{:x}:{}/{}/{}us",
                tally.span.start, tally.used_blocks, tally.used_bytes, tally.hold_us
            )?;
        }
        Ok(())
    }
}

pub struct TopLine<'a> {
    pub stage: &'a str,
    pub census: &'a HeapCensus,
    pub chunk: usize,
}

impl Display for TopLine<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "boot heap top [{}] {}/{}:",
            self.stage,
            self.chunk,
            top_line_count()
        )?;
        let from = self.chunk * TOPS_PER_LINE;
        for at in from..(from + TOPS_PER_LINE).min(TOP_EXACT_COUNT) {
            let Some(&(size, blocks)) = self.census.top.get(at) else {
                continue;
            };
            if blocks == 0 {
                continue;
            }
            write!(f, " {size}x{blocks}")?;
        }
        Ok(())
    }
}

pub fn top_line_count() -> usize {
    TOP_EXACT_COUNT.div_ceil(TOPS_PER_LINE)
}

pub fn bucket_line_count() -> usize {
    BUCKET_COUNT.div_ceil(BUCKETS_PER_LINE)
}

pub fn span_line_count(census: &HeapCensus) -> usize {
    census.span_count().div_ceil(SPANS_PER_LINE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONGEST_STAGE: &str = "composition start";

    fn saturated_record(census: HeapCensus) -> StageRecord {
        StageRecord {
            name: [0; STAGE_NAME_BYTES],
            name_len: 0,
            blocks_before: INTERNAL_BLOCK_CEILING,
            blocks_after: INTERNAL_BLOCK_CEILING,
            walk_us: US_CEILING,
            info_us: US_CEILING,
            rust_internal_bytes: INTERNAL_BYTE_CEILING,
            rust_psram_bytes: u32::MAX,
            rust_by_class: [INTERNAL_BYTE_CEILING; RUST_CLASSES],
            census,
        }
    }

    fn saturated() -> HeapCensus {
        let tally = SpanTally {
            span: Span { start: 0x4ff4_0000, end: 0x4ff8_0000 },
            used_blocks: INTERNAL_BLOCK_CEILING,
            used_bytes: INTERNAL_BYTE_CEILING,
            free_blocks: INTERNAL_BLOCK_CEILING,
            free_bytes: INTERNAL_BYTE_CEILING,
            opened_us: US_CEILING,
            hold_us: US_CEILING,
        };
        HeapCensus {
            used_blocks: INTERNAL_BLOCK_CEILING,
            used_bytes: INTERNAL_BYTE_CEILING,
            free_blocks: INTERNAL_BLOCK_CEILING,
            free_bytes: INTERNAL_BYTE_CEILING,
            off_table_blocks: INTERNAL_BLOCK_CEILING,
            off_table_bytes: INTERNAL_BYTE_CEILING,
            bucket_blocks: [INTERNAL_BLOCK_CEILING; BUCKET_COUNT],
            bucket_bytes: [INTERNAL_BYTE_CEILING; BUCKET_COUNT],
            exact_blocks: [INTERNAL_BLOCK_CEILING; EXACT_SLOT_COUNT],
            spans_seen: SPAN_COUNT as u32,
            spans: [tally; SPAN_COUNT],
            last_span: tally.span,
            seen_blocks: INTERNAL_BLOCK_CEILING,
            cut_heaps: SPAN_COUNT as u32,
            hold_budget_us: US_CEILING,
            top: [(EXACT_MAX_BYTES, INTERNAL_BLOCK_CEILING); TOP_EXACT_COUNT],
        }
    }

    const INTERNAL_BYTE_CEILING: u32 = 786_432;
    const INTERNAL_BLOCK_CEILING: u32 = INTERNAL_BYTE_CEILING / SMALLEST_BUCKET_BYTES;
    const US_CEILING: u32 = 99_999;

    #[test]
    fn no_line_of_the_census_outgrows_what_a_log_slot_holds() {
        let census = saturated();
        let cap = dali2rust_platform::logs::LOG_TEXT_BYTES;
        let mut widest = 0;
        let record = saturated_record(census);
        let mut rendered = vec![CensusLine { stage: LONGEST_STAGE, record: &record }
            .to_string()];
        for chunk in 0..top_line_count() {
            rendered.push(
                TopLine { stage: LONGEST_STAGE, census: &census, chunk }.to_string(),
            );
        }
        rendered.push(
            RustLine {
                stage: LONGEST_STAGE,
                internal_bytes: INTERNAL_BYTE_CEILING,
                psram_bytes: u32::MAX,
                by_class: &[INTERNAL_BYTE_CEILING; RUST_CLASSES],
            }
            .to_string(),
        );
        for chunk in 0..bucket_line_count() {
            rendered.push(BucketLine { stage: LONGEST_STAGE, census: &census, chunk }.to_string());
        }
        for chunk in 0..span_line_count(&census) {
            rendered.push(SpanLine { stage: LONGEST_STAGE, census: &census, chunk }.to_string());
        }
        for line in &rendered {
            widest = widest.max(line.len());
            assert!(line.len() <= cap, "{} bytes: {line}", line.len());
        }
        assert!(widest > cap / 2, "the saturated lines are too short to be a real check: {widest}");
    }

    #[test]
    fn the_census_line_carries_the_figures_a_reader_cross_checks() {
        let span = Span { start: 0x4ff4_0000, end: 0x4ff8_0000 };
        let mut census = HeapCensus::default();
        census.record(span, 32, true);
        census.record(span, 64, false);
        let mut record = saturated_record(census);
        record.blocks_before = 1;
        record.blocks_after = 1;
        record.walk_us = 410;
        record.info_us = 380;
        let line = CensusLine { stage: "after hydrate", record: &record }.to_string();
        assert_eq!(
            line,
            "boot heap census [after hydrate]: used=1/32 free=1/64 ref=1/1 us=410/380 \
off=0/0 sum=1/32"
        );
    }

    #[test]
    fn the_bucket_and_span_lines_name_every_entry_they_carry() {
        let span = Span { start: 0x4ff4_0000, end: 0x4ff8_0000 };
        let mut census = HeapCensus::default();
        census.record(span, 24, true);
        assert_eq!(
            BucketLine { stage: "s", census: &census, chunk: 0 }.to_string(),
            "boot heap sizes [s] 0/3: b0:0/0 b1:1/24 b2:0/0 b3:0/0"
        );
        assert_eq!(
            SpanLine { stage: "s", census: &census, chunk: 0 }.to_string(),
            "boot heap spans [s] 0/1 seen=1 cut=0 hold<=900us: 0@4ff40000:1/24/0us"
        );
        assert_eq!(span_line_count(&census), 1);
        assert_eq!(bucket_line_count(), 3);
    }

    #[test]
    fn a_bucket_holds_the_sizes_up_to_its_own_power_of_two() {
        assert_eq!(bucket_of(0), 0);
        assert_eq!(bucket_of(1), 0);
        assert_eq!(bucket_of(16), 0);
        assert_eq!(bucket_of(17), 1);
        assert_eq!(bucket_of(32), 1);
        assert_eq!(bucket_of(33), 2);
        assert_eq!(bucket_of(16384), BUCKET_COUNT - 2);
        assert_eq!(bucket_of(16385), BUCKET_COUNT - 1);
        assert_eq!(bucket_of(u32::MAX), BUCKET_COUNT - 1);
    }

    #[test]
    fn the_exact_table_takes_aligned_sizes_and_refuses_the_rest() {
        assert_eq!(exact_slot(0), Some(0));
        assert_eq!(exact_slot(4), Some(1));
        assert_eq!(exact_slot(EXACT_MAX_BYTES), Some(EXACT_SLOT_COUNT - 1));
        assert_eq!(exact_slot(EXACT_MAX_BYTES + EXACT_STEP_BYTES), None);
        assert_eq!(exact_slot(6), None);
    }

    #[test]
    fn the_buckets_sum_to_the_used_totals_and_the_free_side_stays_out() {
        let span = Span { start: 1, end: 2 };
        let mut census = HeapCensus::default();
        for size in [16u32, 24, 24, 4096, 999_999] {
            census.record(span, size, true);
        }
        census.record(span, 512, false);
        assert_eq!(census.seen_blocks, 6);
        assert_eq!(census.used_blocks, 5);
        assert_eq!(census.used_bytes, 16 + 24 + 24 + 4096 + 999_999);
        assert_eq!(census.bucket_sums(), (census.used_blocks, census.used_bytes));
        assert_eq!(census.free_blocks, 1);
        assert_eq!(census.free_bytes, 512);
        assert_eq!(census.bucket_blocks[0], 1);
        assert_eq!(census.bucket_blocks[1], 2);
    }

    #[test]
    fn a_size_past_the_exact_table_is_counted_off_table_not_dropped() {
        let span = Span { start: 1, end: 2 };
        let mut census = HeapCensus::default();
        census.record(span, EXACT_MAX_BYTES, true);
        census.record(span, EXACT_MAX_BYTES + EXACT_STEP_BYTES, true);
        census.record(span, 6, true);
        assert_eq!(census.exact_blocks[EXACT_SLOT_COUNT - 1], 1);
        assert_eq!(census.off_table_blocks, 2);
        assert_eq!(census.off_table_bytes, EXACT_MAX_BYTES + EXACT_STEP_BYTES + 6);
        assert_eq!(census.bucket_sums(), (3, EXACT_MAX_BYTES * 2 + EXACT_STEP_BYTES + 6));
    }

    #[test]
    fn each_heap_gets_its_own_tally_and_a_seventh_only_raises_the_count() {
        let mut census = HeapCensus::default();
        for n in 0..(SPAN_COUNT as u32 + 1) {
            let span = Span { start: n * 16, end: n * 16 + 8 };
            census.record(span, 32, true);
            census.record(span, 64, false);
        }
        assert_eq!(census.spans_seen, SPAN_COUNT as u32 + 1);
        assert_eq!(census.span_count(), SPAN_COUNT);
        assert_eq!(census.spans[0].span, Span { start: 0, end: 8 });
        assert_eq!(census.spans[0].used_bytes, 32);
        assert_eq!(census.spans[0].free_bytes, 64);
        assert_eq!(census.used_blocks, SPAN_COUNT as u32 + 1);
    }

    #[test]
    fn a_heap_whose_hold_outran_its_budget_is_cut_and_says_so() {
        let mut census = HeapCensus::default();
        census.record(Span { start: 0, end: 8 }, 32, true);
        census.stamp_span_open(1_000);
        assert!(!census.hold_is_spent(1_000 + HOLD_BUDGET_US));
        assert!(census.hold_is_spent(1_001 + HOLD_BUDGET_US));
        census.hold_budget_us = 0;
        assert_eq!(census.budget_us(), HOLD_BUDGET_US);
        census.hold_budget_us = 2_500;
        assert!(!census.hold_is_spent(1_000 + 2_500));
        assert!(census.hold_is_spent(1_001 + 2_500));
        census.cut_this_heap();
        assert_eq!(census.cut_heaps, 1);
    }

    #[test]
    fn a_walk_that_has_opened_no_heap_yet_has_no_hold_to_spend() {
        assert!(!HeapCensus::default().hold_is_spent(u32::MAX));
    }

    #[test]
    fn each_heap_hold_runs_to_the_next_heap_and_the_last_to_the_walk_end() {
        let mut census = HeapCensus::default();
        for (at, opened) in [(0u32, 100u32), (16, 400), (32, 1000)] {
            census.record(Span { start: at, end: at + 8 }, 32, true);
            census.stamp_span_open(opened);
        }
        census.close_span_holds(1600);
        assert_eq!(census.spans[0].hold_us, 300);
        assert_eq!(census.spans[1].hold_us, 600);
        assert_eq!(census.spans[2].hold_us, 600);
        assert_eq!(census.longest_hold_us(), 600);
    }

    #[test]
    fn a_hold_that_straddles_the_microsecond_wrap_is_a_duration_not_a_negative() {
        let mut census = HeapCensus::default();
        census.record(Span { start: 0, end: 8 }, 32, true);
        census.stamp_span_open(u32::MAX - 100);
        census.close_span_holds(200);
        assert_eq!(census.spans[0].hold_us, 301);
    }

    #[test]
    fn a_heap_seen_again_after_another_one_does_not_reuse_its_tally() {
        let mut census = HeapCensus::default();
        let first = Span { start: 0, end: 8 };
        let second = Span { start: 16, end: 24 };
        census.record(first, 32, true);
        census.record(second, 32, true);
        census.record(first, 32, true);
        assert_eq!(census.spans_seen, 3);
        assert_eq!(census.spans[0].used_blocks, 1);
        assert_eq!(census.spans[2].span, first);
    }

    #[test]
    fn the_rust_line_carries_the_placement_split_and_every_size_class() {
        assert_eq!(
            RustLine {
                stage: "s",
                internal_bytes: 40,
                psram_bytes: 900,
                by_class: &[16, 24, 0, 0, 0, 0],
            }
            .to_string(),
            "boot heap rust [s]: internal=40 psram=900 c0:16 c1:24 c2:0 c3:0 c4:0 c5:0"
        );
    }

    #[test]
    fn the_top_table_ranks_by_bytes_and_ignores_the_sizes_with_no_blocks() {
        let span = Span { start: 1, end: 2 };
        let mut census = HeapCensus::default();
        for _ in 0..100 {
            census.record(span, 8, true);
        }
        for _ in 0..3 {
            census.record(span, 512, true);
        }
        census.record(span, 1020, true);
        census.rank_top();
        assert_eq!(census.top[0], (512, 3));
        assert_eq!(census.top[1], (1020, 1));
        assert_eq!(census.top[2], (8, 100));
        assert_eq!(census.top[3], (0, 0));
        assert_eq!(
            TopLine { stage: "s", census: &census, chunk: 0 }.to_string(),
            "boot heap top [s] 0/3: 512x3 1020x1 8x100"
        );
        assert_eq!(
            TopLine { stage: "s", census: &census, chunk: 2 }.to_string(),
            "boot heap top [s] 2/3:"
        );
    }

    #[test]
    fn the_top_table_keeps_the_largest_when_more_sizes_compete_than_it_holds() {
        let span = Span { start: 1, end: 2 };
        let mut census = HeapCensus::default();
        for step in 1..=(TOP_EXACT_COUNT as u32 + 2) {
            census.record(span, step * EXACT_STEP_BYTES, true);
        }
        census.rank_top();
        let top = census.top;
        let smallest_kept = (TOP_EXACT_COUNT as u32 + 2 - TOP_EXACT_COUNT as u32 + 1)
            * EXACT_STEP_BYTES;
        assert_eq!(top[0].0, (TOP_EXACT_COUNT as u32 + 2) * EXACT_STEP_BYTES);
        assert_eq!(top[TOP_EXACT_COUNT - 1].0, smallest_kept);
        assert!(top.iter().all(|(_, blocks)| *blocks == 1));
    }

    fn empty_log() -> BootCensusLog {
        BootCensusLog {
            records: [saturated_record(HeapCensus::default()); STAGE_SLOTS],
            used: 0,
            emitted: 0,
        }
    }

    #[test]
    fn a_stage_hands_out_each_of_its_lines_once_and_then_stops() {
        let span = Span { start: 0x4ff4_0000, end: 0x4ff8_0000 };
        let mut log = empty_log();
        for stage in ["composition start", "after hydrate"] {
            let record = log.open_stage(stage).expect("a free record");
            record.census.record(span, 24, true);
            record.census.rank_top();
            record.rust_internal_bytes = 24;
        }
        assert_eq!(log.used, 2);
        assert_eq!(log.pending(), 2 * lines_per_stage());

        let mut spelled = Vec::new();
        while let Some((record, at)) = log.take_line() {
            spelled.push(line_of(record, at).to_string());
        }
        assert_eq!(log.pending(), 0);
        assert!(log.take_line().is_none());
        assert_eq!(spelled.len(), 2 * lines_per_stage());
        let spoken = 2 + bucket_line_count() + top_line_count() + 1;
        for stage in ["composition start", "after hydrate"] {
            let mine: Vec<&String> = spelled.iter().filter(|line| line.contains(stage)).collect();
            assert_eq!(mine.len(), spoken, "{mine:?}");
            assert_eq!(mine.iter().filter(|l| l.starts_with("boot heap census")).count(), 1);
            assert_eq!(mine.iter().filter(|l| l.starts_with("boot heap sizes")).count(), 3);
            assert_eq!(mine.iter().filter(|l| l.starts_with("boot heap top")).count(), 3);
            assert_eq!(mine.iter().filter(|l| l.starts_with("boot heap rust")).count(), 1);
            assert_eq!(mine.iter().filter(|l| l.starts_with("boot heap spans")).count(), 1);
        }
        assert_eq!(
            spelled.iter().filter(|line| line.is_empty()).count(),
            2 * (lines_per_stage() - spoken)
        );
    }

    #[test]
    fn a_ninth_stage_finds_no_record_rather_than_overwriting_the_first() {
        let mut log = empty_log();
        for at in 0..STAGE_SLOTS {
            assert!(log.open_stage(&format!("stage {at}")).is_some());
        }
        assert!(log.open_stage("one too many").is_none());
        assert_eq!(log.used, STAGE_SLOTS as u32);
        assert_eq!(log.records[0].stage(), "stage 0");
    }

    #[test]
    fn a_stage_name_longer_than_the_record_is_cut_and_stays_valid_text() {
        let mut record = saturated_record(HeapCensus::default());
        record.set_stage("a stage name far longer than the record holds");
        assert_eq!(record.stage().len(), STAGE_NAME_BYTES);
        assert!("a stage name far longer than the record holds".starts_with(record.stage()));
    }

    #[test]
    fn every_chunked_line_count_covers_its_whole_table() {
        assert_eq!(bucket_line_count(), 3);
        assert!(bucket_line_count() * BUCKETS_PER_LINE >= BUCKET_COUNT);
        assert_eq!(top_line_count(), 3);
        assert!(top_line_count() * TOPS_PER_LINE >= TOP_EXACT_COUNT);
        assert!(span_line_count(&saturated()) * SPANS_PER_LINE >= SPAN_COUNT);
    }
}
