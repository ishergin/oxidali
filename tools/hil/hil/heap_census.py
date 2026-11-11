from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Set, Tuple

CENSUS_RE = re.compile(
    r"boot heap census \[([^\]]+)\]: used=(\d+)/(\d+) free=(\d+)/(\d+) "
    r"ref=(\d+)/(\d+) us=(\d+)/(\d+) off=(\d+)/(\d+) sum=(\d+)/(\d+)"
)
SIZES_RE = re.compile(r"boot heap sizes \[([^\]]+)\] (\d+)/(\d+):((?: b\d+:\d+/\d+)*)")
TOP_RE = re.compile(r"boot heap top \[([^\]]+)\] (\d+)/(\d+):((?: \d+x\d+)*)")
SPANS_RE = re.compile(
    r"boot heap spans \[([^\]]+)\] (\d+)/(\d+) seen=(\d+) cut=(\d+) hold<=(\d+)us:"
    r"((?: \d+@[0-9a-f]+:\d+/\d+/\d+us)*)"
)
RUST_RE = re.compile(
    r"boot heap rust \[([^\]]+)\]: internal=(\d+) psram=(\d+)((?: c\d+:\d+)*)"
)
LARGE_RE = re.compile(
    r"boot heap large \[([^\]]+)\] (\d+)/(\d+) seen=(\d+) kept=(\d+):"
    r"((?: [0-9a-f]+\+\d+(?::\S+)?)*)"
)
NO_TABLE_RE = re.compile(r"boot heap census \[([^\]]+)\]: no PSRAM")

BUCKET_ENTRY_RE = re.compile(r"b(\d+):(\d+)/(\d+)")
TOP_ENTRY_RE = re.compile(r"(\d+)x(\d+)")
SPAN_ENTRY_RE = re.compile(r"(\d+)@([0-9a-f]+):(\d+)/(\d+)/(\d+)us")
CLASS_ENTRY_RE = re.compile(r"c(\d+):(\d+)")
LARGE_ENTRY_RE = re.compile(r"([0-9a-f]+)\+(\d+)(?::(\S+))?")

ARRIVAL_MIN_BYTES = 1024
TOP_EXACT_COUNT = 24


def _stage(raw: str) -> str:
    return raw.strip().replace(" ", "_")


@dataclass
class StageCensus:
    stage: str
    used_blocks: int = 0
    used_bytes: int = 0
    free_blocks: int = 0
    free_bytes: int = 0
    ref_before: int = 0
    ref_after: int = 0
    walk_us: int = 0
    info_us: int = 0
    off_blocks: int = 0
    off_bytes: int = 0
    sum_blocks: int = 0
    sum_bytes: int = 0
    refused: bool = False
    header_seen: bool = False
    buckets: Dict[int, Tuple[int, int]] = field(default_factory=dict)
    bucket_chunks: Set[int] = field(default_factory=set)
    bucket_chunks_total: Optional[int] = None
    top_by_chunk: Dict[int, List[Tuple[int, int]]] = field(default_factory=dict)
    top_chunks_total: Optional[int] = None
    spans: Dict[int, Tuple[int, int, int, int]] = field(default_factory=dict)
    spans_seen: Optional[int] = None
    cut_heaps: Optional[int] = None
    hold_budget_us: Optional[int] = None
    span_chunks: Set[int] = field(default_factory=set)
    span_chunks_total: Optional[int] = None
    rust_internal: Optional[int] = None
    rust_psram: Optional[int] = None
    rust_classes: Dict[int, int] = field(default_factory=dict)
    large: Dict[Tuple[int, int], str] = field(default_factory=dict)
    large_chunks: Set[int] = field(default_factory=set)
    large_chunks_total: Optional[int] = None
    large_seen: Optional[int] = None
    large_kept: Optional[int] = None

    def bucket_sums(self) -> Tuple[int, int]:
        blocks = sum(entry[0] for entry in self.buckets.values())
        byte_count = sum(entry[1] for entry in self.buckets.values())
        return blocks, byte_count

    def top(self) -> List[Tuple[int, int]]:
        out: List[Tuple[int, int]] = []
        for chunk in sorted(self.top_by_chunk):
            out += self.top_by_chunk[chunk]
        return out

    def raced(self) -> bool:
        return self.ref_before != self.ref_after

    def longest_hold_us(self) -> int:
        return max((entry[3] for entry in self.spans.values()), default=0)

    def c_bytes(self) -> Optional[int]:
        if self.rust_internal is None:
            return None
        return self.used_bytes - self.rust_internal


Boot = Tuple[List[str], Dict[str, StageCensus]]


def parse_boots(lines) -> List[Boot]:
    boots: List[Boot] = []
    order: List[str] = []
    found: Dict[str, StageCensus] = {}

    def at(stage: str, opens: bool) -> StageCensus:
        nonlocal order, found
        if opens and stage in found and found[stage].header_seen:
            boots.append((order, found))
            order, found = [], {}
        if stage not in found:
            found[stage] = StageCensus(stage=stage)
            order.append(stage)
        return found[stage]

    for line in lines:
        _read_line(line, at)
    if order:
        boots.append((order, found))
    return boots


def parse(lines) -> Boot:
    boots = parse_boots(lines)
    return boots[-1] if boots else ([], {})


def _read_line(line: str, at) -> None:
    header = CENSUS_RE.search(line)
    if header:
        entry = at(_stage(header.group(1)), True)
        entry.header_seen = True
        _read_header(entry, header)
        return
    refused = NO_TABLE_RE.search(line)
    if refused:
        entry = at(_stage(refused.group(1)), True)
        entry.header_seen = True
        entry.refused = True
        return
    sizes = SIZES_RE.search(line)
    if sizes:
        _read_sizes(at(_stage(sizes.group(1)), False), sizes)
        return
    top = TOP_RE.search(line)
    if top:
        _read_top(at(_stage(top.group(1)), False), top)
        return
    spans = SPANS_RE.search(line)
    if spans:
        _read_spans(at(_stage(spans.group(1)), False), spans)
        return
    rust = RUST_RE.search(line)
    if rust:
        _read_rust(at(_stage(rust.group(1)), False), rust)
        return
    large = LARGE_RE.search(line)
    if large:
        _read_large(at(_stage(large.group(1)), False), large)


def _read_header(entry: StageCensus, found) -> None:
    (
        entry.used_blocks,
        entry.used_bytes,
        entry.free_blocks,
        entry.free_bytes,
        entry.ref_before,
        entry.ref_after,
        entry.walk_us,
        entry.info_us,
        entry.off_blocks,
        entry.off_bytes,
        entry.sum_blocks,
        entry.sum_bytes,
    ) = [int(found.group(at)) for at in range(2, 14)]


def _read_sizes(entry: StageCensus, found) -> None:
    entry.bucket_chunks.add(int(found.group(2)))
    entry.bucket_chunks_total = int(found.group(3))
    for at, blocks, byte_count in BUCKET_ENTRY_RE.findall(found.group(4)):
        entry.buckets[int(at)] = (int(blocks), int(byte_count))


def _read_top(entry: StageCensus, found) -> None:
    entry.top_chunks_total = int(found.group(3))
    entry.top_by_chunk[int(found.group(2))] = [
        (int(size), int(blocks)) for size, blocks in TOP_ENTRY_RE.findall(found.group(4))
    ]


def _read_spans(entry: StageCensus, found) -> None:
    entry.span_chunks.add(int(found.group(2)))
    entry.span_chunks_total = int(found.group(3))
    entry.spans_seen = int(found.group(4))
    entry.cut_heaps = int(found.group(5))
    entry.hold_budget_us = int(found.group(6))
    for at, start, blocks, byte_count, hold in SPAN_ENTRY_RE.findall(found.group(7)):
        entry.spans[int(at)] = (
            int(start, 16),
            int(blocks),
            int(byte_count),
            int(hold),
        )


def _read_rust(entry: StageCensus, found) -> None:
    entry.rust_internal = int(found.group(2))
    entry.rust_psram = int(found.group(3))
    for at, byte_count in CLASS_ENTRY_RE.findall(found.group(4)):
        entry.rust_classes[int(at)] = int(byte_count)


def _read_large(entry: StageCensus, found) -> None:
    entry.large_chunks.add(int(found.group(2)))
    entry.large_chunks_total = int(found.group(3))
    entry.large_seen = int(found.group(4))
    entry.large_kept = int(found.group(5))
    for addr, size, owner in LARGE_ENTRY_RE.findall(found.group(6)):
        entry.large[(int(addr, 16), int(size))] = owner


def verdicts(entry: StageCensus) -> List[str]:
    if entry.refused:
        return ["not measured: PSRAM refused the census table"]
    if not entry.header_seen:
        return ["not measured: no census header reached the log for this stage"]
    out: List[str] = []
    if entry.cut_heaps:
        out.append(
            "%d heap traversal(s) were cut at the hold budget, so this stage is partial"
            % entry.cut_heaps
        )
    if entry.raced():
        out.append(
            "raced: the heap moved during the walk (%d blocks before, %d after)"
            % (entry.ref_before, entry.ref_after)
        )
    elif not entry.cut_heaps and entry.used_blocks != entry.ref_before:
        out.append(
            "the walk and heap_caps_get_info disagree on the block count (%d vs %d)"
            % (entry.used_blocks, entry.ref_before)
        )
    out += _buckets_complete(entry)
    out += _chunked_complete(entry, "top-size", len(entry.top_by_chunk),
                             entry.top_chunks_total)
    out += _chunked_complete(entry, "heap", len(entry.span_chunks),
                             entry.span_chunks_total)
    if entry.rust_internal is None:
        out.append("no Rust-share line reached the log: the C share cannot be taken")
    if (
        entry.span_chunks_total is not None
        and entry.spans_seen is not None
        and entry.spans_seen > len(entry.spans)
    ):
        out.append(
            "more internal heaps than the table holds (%d seen, %d kept)"
            % (entry.spans_seen, len(entry.spans))
        )
    return out


def _chunked_complete(entry: StageCensus, what: str, seen: int,
                      total: Optional[int]) -> List[str]:
    if total is None:
        return ["no %s line reached the log for this stage" % what]
    if seen != total:
        return ["a %s line is missing from the log (%d of %d)" % (what, seen, total)]
    return []


def _buckets_complete(entry: StageCensus) -> List[str]:
    out = _chunked_complete(entry, "bucket", len(entry.bucket_chunks),
                            entry.bucket_chunks_total)
    if out:
        return out
    blocks, byte_count = entry.bucket_sums()
    if (blocks, byte_count) != (entry.sum_blocks, entry.sum_bytes):
        out.append(
            "the bucket lines do not add up to the header (%d/%d against %d/%d)"
            % (blocks, byte_count, entry.sum_blocks, entry.sum_bytes)
        )
    return out


def large_verdicts(entry: StageCensus) -> List[str]:
    if entry.refused or not entry.header_seen:
        return verdicts(entry)[:1]
    out = _chunked_complete(entry, "large-block", len(entry.large_chunks),
                            entry.large_chunks_total)
    if out:
        return out
    if entry.large_kept is not None and len(entry.large) != entry.large_kept:
        out.append(
            "the large-block lines carry %d blocks, their header says %d kept"
            % (len(entry.large), entry.large_kept)
        )
    if entry.large_seen is not None and entry.large_seen > (entry.large_kept or 0):
        out.append(
            "more large blocks than the table holds (%d seen, %d kept)"
            % (entry.large_seen, entry.large_kept or 0)
        )
    out += _walk_doubts(entry)
    return out


def _walk_doubts(entry: StageCensus) -> List[str]:
    out = []
    if entry.cut_heaps:
        out.append("a cut heap traversal hides its blocks")
    if entry.raced():
        out.append("the heap moved during the walk")
    return out


def large_whole(entry: StageCensus) -> bool:
    return not large_verdicts(entry)


def top_doubts(entry: StageCensus) -> List[str]:
    if entry.refused or not entry.header_seen:
        return verdicts(entry)[:1]
    out = _chunked_complete(entry, "top-size", len(entry.top_by_chunk),
                            entry.top_chunks_total)
    return out + _walk_doubts(entry)


def believable(entry: StageCensus) -> bool:
    return not verdicts(entry)


def bucket_deltas(before: StageCensus, after: StageCensus) -> Dict[int, Tuple[int, int]]:
    out: Dict[int, Tuple[int, int]] = {}
    for at in sorted(set(before.buckets) | set(after.buckets)):
        was = before.buckets.get(at, (0, 0))
        now = after.buckets.get(at, (0, 0))
        moved = (now[0] - was[0], now[1] - was[1])
        if moved != (0, 0):
            out[at] = moved
    return out


def report_lines(boots: List[Boot]) -> List[str]:
    if not boots:
        return [
            "heap census: no boot-stage census in this run's serial window "
            "(nothing rebooted, or the monitor was down) — not measured"
        ]
    order, found = boots[-1]
    out = [
        "heap census (internal, used blocks/bytes at each boot stage%s):"
        % ("" if len(boots) == 1 else "; last of %d boots in the window" % len(boots))
    ]
    for stage in order:
        out += _stage_lines(found[stage])
    out += _delta_lines(order, found)
    out += large_lines(order, found)
    out += arrival_lines(order, found)
    return out


def _stage_lines(entry: StageCensus) -> List[str]:
    if entry.refused or not entry.header_seen:
        return ["  %-20s %s" % (entry.stage, verdicts(entry)[0])]
    rust = entry.c_bytes()
    out = [
        "  %-20s %5d/%7d  walk %d us (info %d us)  off-table %d/%d%s"
        % (
            entry.stage,
            entry.used_blocks,
            entry.used_bytes,
            entry.walk_us,
            entry.info_us,
            entry.off_blocks,
            entry.off_bytes,
            "" if rust is None else "  C about %d, Rust %d" % (rust, entry.rust_internal),
        ),
        "    longest single heap hold %d us of %d us walked (budget %s us)"
        % (entry.longest_hold_us(), entry.walk_us, entry.hold_budget_us),
    ]
    for note in verdicts(entry):
        out.append("    *** %s" % note)
    return out


def _delta_lines(order: List[str], found: Dict[str, StageCensus]) -> List[str]:
    pairs = [
        (order[at], order[at + 1])
        for at in range(len(order) - 1)
        if believable(found[order[at]]) and believable(found[order[at + 1]])
    ]
    if not pairs:
        return ["  phase deltas: no two neighbouring stages are both believable"]
    out = ["  phase deltas (used blocks/bytes by size bucket):"]
    for was, now in pairs:
        moved = bucket_deltas(found[was], found[now])
        spelled = " ".join(
            "b%d:%+d/%+d" % (at, blocks, byte_count)
            for at, (blocks, byte_count) in sorted(moved.items())
        )
        out.append("    %s -> %s  %s" % (was, now, spelled or "nothing moved"))
    return out


def top_lines(entry: StageCensus) -> List[str]:
    top = entry.top()
    if not top:
        return ["  top sizes at %s: none recorded" % entry.stage]
    out = ["  top sizes at %s (size x blocks = bytes):" % entry.stage]
    for size, blocks in top:
        out.append("    %6d x %5d = %8d" % (size, blocks, size * blocks))
    return out


LargeKey = Tuple[int, int]


def _large_spans(order: List[str], found: Dict[str, StageCensus]):
    first: Dict[LargeKey, str] = {}
    last: Dict[LargeKey, str] = {}
    owner: Dict[LargeKey, str] = {}
    for stage in order:
        for key, name in found[stage].large.items():
            first.setdefault(key, stage)
            last[key] = stage
            if name:
                owner[key] = name
    return first, last, owner


def large_lines(order: List[str], found: Dict[str, StageCensus]) -> List[str]:
    if all(found[stage].large_chunks_total is None for stage in order):
        return ["  large blocks: no stage reported its blocks past 1020 B"]
    whole = [stage for stage in order if large_whole(found[stage])]
    out = _large_table(whole, found)
    for stage in order:
        for note in large_verdicts(found[stage]):
            out.append("    *** %s: %s; left out, so a block first shown after it "
                       "may have arrived at it" % (stage, note))
    return out


def _large_table(whole: List[str], found: Dict[str, StageCensus]) -> List[str]:
    if not whole:
        return ["  large blocks: no stage carried its whole large-block table"]
    first, last, owner = _large_spans(whole, found)
    rank = {stage: at for at, stage in enumerate(whole)}
    out = [
        "  large blocks (used, past 1020 B) by the first whole stage that shows them; "
        "those under %s were there before it:" % whole[0]
    ]
    for key in sorted(first, key=lambda key: (rank[first[key]], key[0])):
        gone = "" if last[key] == whole[-1] else "  gone after %s" % last[key]
        out.append(
            "    %-18s %8x %6d  %s%s"
            % (first[key], key[0], key[1], owner.get(key, "(unnamed)"), gone)
        )
    return out


def arrivals(before: StageCensus, after: StageCensus) -> List[Tuple[int, int, str]]:
    was = dict(before.top())
    full = len(was) >= TOP_EXACT_COUNT
    floor = min((size * blocks for size, blocks in was.items()), default=0)
    out = []
    for size, blocks in after.top():
        earlier = was.get(size, floor // size if full else 0)
        grown = blocks - earlier
        if grown > 0 and size * grown >= ARRIVAL_MIN_BYTES:
            out.append((size, grown, _arrival_kind(size in was, full)))
    return out


def _arrival_kind(present: bool, full: bool) -> str:
    if present:
        return ""
    return " (entered the top: at least)" if full else " (new in top)"


def arrival_lines(order: List[str], found: Dict[str, StageCensus]) -> List[str]:
    out = ["  top-size arrivals between neighbouring stages (%d B or more):" % ARRIVAL_MIN_BYTES]
    for was, now in zip(order, order[1:]):
        doubts = ["%s: %s" % (stage, note) for stage in (was, now)
                  for note in top_doubts(found[stage])]
        if doubts:
            out.append("    %s -> %s  not compared: %s" % (was, now, "; ".join(doubts)))
            continue
        moved = arrivals(found[was], found[now])
        if not moved:
            continue
        spelled = " ".join("%dx+%d%s" % entry for entry in moved)
        out.append("    %s -> %s  %s" % (was, now, spelled))
    return out if len(out) > 1 else []
