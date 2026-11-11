import pathlib
import re

from hil import heap_census

STAMP = "2026-11-08T05:00:0%d.000 W (%d) dali2rust_bsp::heap_walk: "


def _line(at, text):
    return (STAMP % (at, at * 1000)) + text


def _stage_lines(stage, used_blocks, used_bytes, buckets, seen=1, rust=24, cut=0):
    header = (
        "boot heap census [%s]: used=%d/%d free=800/400000 ref=%d/%d us=410/380 "
        "off=0/0 sum=%d/%d" % (stage, used_blocks, used_bytes, used_blocks,
                               used_blocks, used_blocks, used_bytes)
    )
    sizes = []
    for chunk in range(3):
        entries = "".join(
            " b%d:%d/%d" % (at, buckets.get(at, (0, 0))[0], buckets.get(at, (0, 0))[1])
            for at in range(chunk * 4, chunk * 4 + 4)
        )
        sizes.append("boot heap sizes [%s] %d/3:%s" % (stage, chunk, entries))
    tops = [
        "boot heap top [%s] 0/3: 232x14" % stage,
        "boot heap top [%s] 1/3:" % stage,
        "boot heap top [%s] 2/3:" % stage,
    ]
    spans = [
        "boot heap spans [%s] 0/1 seen=%d cut=%d hold<=900us: 0@4ff40000:%d/%d/120us"
        % (stage, seen, cut, used_blocks, used_bytes)
    ]
    rust_line = [
        "boot heap rust [%s]: internal=%d psram=9000 c0:%d c1:0 c2:0 c3:0 c4:0 c5:0"
        % (stage, rust, rust)
    ]
    return [header] + sizes + tops + spans + rust_line


def _log(*blocks):
    out = []
    for at, block in enumerate(blocks):
        for text in block:
            out.append(_line(at % 10, text))
    return out


FIRST = _stage_lines("composition start", 10, 1000, {0: (6, 96), 5: (4, 904)})
SECOND = _stage_lines("after hydrate", 14, 5000, {0: (6, 96), 5: (4, 904), 9: (4, 4000)})


def _only(lines, stage="composition_start"):
    return heap_census.parse(_log(lines))[1][stage]


def test_a_stage_is_read_back_with_its_buckets_heaps_rust_share_and_timings():
    order, found = heap_census.parse(_log(FIRST, SECOND))

    assert order == ["composition_start", "after_hydrate"]
    first = found["composition_start"]
    assert (first.used_blocks, first.used_bytes) == (10, 1000)
    assert (first.walk_us, first.info_us) == (410, 380)
    assert first.buckets[0] == (6, 96)
    assert first.buckets[5] == (4, 904)
    assert first.bucket_sums() == (10, 1000)
    assert first.top() == [(232, 14)]
    assert first.spans[0] == (0x4FF40000, 10, 1000, 120)
    assert first.longest_hold_us() == 120
    assert (first.rust_internal, first.rust_psram) == (24, 9000)
    assert first.rust_classes == {0: 24, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0}
    assert first.c_bytes() == 1000 - 24
    assert heap_census.verdicts(first) == []
    assert heap_census.believable(first)


def test_a_dropped_bucket_line_makes_the_stage_inconclusive_not_smaller():
    entry = _only([text for text in FIRST if " 1/3:" not in text or "sizes" not in text])

    notes = heap_census.verdicts(entry)
    assert len(entry.bucket_chunks) == 2
    assert any("a bucket line is missing" in note for note in notes), notes
    assert not heap_census.believable(entry)


def test_a_dropped_bucket_line_with_nothing_in_it_is_still_caught():
    entry = _only(
        [text for text in FIRST if " 2/3:" not in text or "sizes" not in text]
    )

    notes = heap_census.verdicts(entry)
    assert entry.bucket_sums() == (10, 1000)
    assert any("a bucket line is missing" in note for note in notes), notes


def test_a_dropped_top_line_is_named_rather_than_read_as_no_sizes():
    entry = _only([text for text in FIRST if "boot heap top" not in text or "1/3" not in text])

    notes = heap_census.verdicts(entry)
    assert entry.top() == [(232, 14)]
    assert any("a top-size line is missing" in note for note in notes), notes


def test_a_missing_rust_line_leaves_the_c_share_untaken():
    entry = _only([text for text in FIRST if "boot heap rust" not in text])

    notes = heap_census.verdicts(entry)
    assert entry.c_bytes() is None
    assert any("the C share cannot be taken" in note for note in notes), notes


def test_a_heap_that_moved_during_the_walk_is_called_raced_not_reconciled():
    entry = _only([
        text.replace("ref=10/10", "ref=10/12") if "census [" in text else text
        for text in FIRST
    ])

    notes = heap_census.verdicts(entry)
    assert entry.raced()
    assert any("raced" in note for note in notes), notes


def test_a_walk_that_counted_other_blocks_than_the_reference_is_named():
    entry = _only([
        text.replace("ref=10/10", "ref=11/11") if "census [" in text else text
        for text in FIRST
    ])

    notes = heap_census.verdicts(entry)
    assert not entry.raced()
    assert any("disagree on the block count" in note for note in notes), notes


def test_buckets_that_do_not_add_up_to_the_header_are_named_on_their_own():
    entry = _only([
        text.replace(" b5:4/904", " b5:4/900") if "sizes" in text and " 1/3:" in text
        else text
        for text in FIRST
    ])

    notes = heap_census.verdicts(entry)
    assert len(entry.bucket_chunks) == 3
    assert not any("a bucket line is missing" in note for note in notes), notes
    assert any("do not add up to the header" in note for note in notes), notes


def test_a_cut_heap_traversal_makes_the_stage_partial():
    entry = _only([
        text.replace("cut=0", "cut=1") if "spans [" in text else text for text in FIRST
    ])

    notes = heap_census.verdicts(entry)
    assert entry.cut_heaps == 1
    assert any("cut at the hold budget" in note for note in notes), notes
    assert not heap_census.believable(entry)


def test_more_heaps_than_the_table_holds_is_reported():
    entry = _only([
        text.replace("seen=1", "seen=7") if "spans [" in text else text for text in FIRST
    ])

    notes = heap_census.verdicts(entry)
    assert any("more internal heaps than the table holds" in note for note in notes), notes


def test_a_refused_table_is_not_measured_rather_than_zero():
    entry = _only(
        ["boot heap census [composition start]: no PSRAM for the census table"]
    )

    assert entry.refused
    assert heap_census.verdicts(entry) == [
        "not measured: PSRAM refused the census table"
    ]
    assert entry.used_bytes == 0


def test_the_phase_delta_names_the_buckets_that_moved_between_two_stages():
    boots = heap_census.parse_boots(_log(FIRST, SECOND))
    order, found = boots[-1]
    moved = heap_census.bucket_deltas(found["composition_start"], found["after_hydrate"])

    assert moved == {9: (4, 4000)}
    report = "\n".join(heap_census.report_lines(boots))
    assert "composition_start -> after_hydrate  b9:+4/+4000" in report
    assert "walk 410 us" in report
    assert "longest single heap hold 120 us of 410 us walked (budget 900 us)" in report
    assert "C about 976, Rust 24" in report


def test_a_stage_that_cannot_be_believed_is_left_out_of_the_deltas():
    dropped = [text for text in SECOND if " 1/3:" not in text or "sizes" not in text]
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(FIRST, dropped))))

    assert "no two neighbouring stages are both believable" in report
    assert "a bucket line is missing" in report


def test_a_second_boot_in_the_window_is_a_second_boot_and_not_merged():
    boots = heap_census.parse_boots(_log(FIRST, SECOND, FIRST, SECOND))

    assert len(boots) == 2
    assert [order for order, _ in boots] == [
        ["composition_start", "after_hydrate"],
        ["composition_start", "after_hydrate"],
    ]
    report = "\n".join(heap_census.report_lines(boots))
    assert "last of 2 boots in the window" in report


def test_a_boot_cut_off_mid_stage_keeps_the_stages_it_did_reach():
    boots = heap_census.parse_boots(_log(FIRST, SECOND, FIRST))

    assert len(boots) == 2
    assert boots[-1][0] == ["composition_start"]
    assert heap_census.believable(boots[-1][1]["composition_start"])


def test_a_stage_with_only_its_chunked_lines_is_not_measured():
    entry = _only([text for text in FIRST if "boot heap census" not in text])

    assert not entry.header_seen
    assert heap_census.verdicts(entry) == [
        "not measured: no census header reached the log for this stage"
    ]


def test_an_empty_window_says_not_measured_rather_than_reporting_zeroes():
    report = heap_census.report_lines(heap_census.parse_boots([]))

    assert len(report) == 1
    assert "not measured" in report[0]


def _with_large(lines, stage, entries, seen=None):
    kept = len(entries)
    seen = kept if seen is None else seen
    out = [
        text.replace("off=0/0", "off=%d/%d" % (seen, sum(size for _, size, _ in entries)))
        if "census [" in text else text
        for text in lines
    ]
    chunks = [entries[at:at + 2] for at in range(0, max(kept, 1), 2)] or [[]]
    for at, chunk in enumerate(chunks):
        spelled = "".join(
            " %x+%d%s" % (addr, size, ":" + owner if owner else "")
            for addr, size, owner in chunk
        )
        out.append(
            "boot heap large [%s] %d/%d seen=%d kept=%d:%s"
            % (stage, at, len(chunks), seen, kept, spelled)
        )
    return out


RESERVE = (0x4FF30000, 32768, "")
MAIN = (0x4FF21000, 12800, "main")
TIMER = (0x4FF40000, 4096, "Tmr_Svc")


def test_large_blocks_are_read_back_with_their_addresses_and_owners():
    entry = _only(_with_large(FIRST, "composition start", [RESERVE, MAIN, TIMER]))

    assert entry.large == {
        (0x4FF30000, 32768): "",
        (0x4FF21000, 12800): "main",
        (0x4FF40000, 4096): "Tmr_Svc",
    }
    assert (entry.large_seen, entry.large_kept, entry.large_chunks_total) == (3, 3, 2)
    assert heap_census.large_verdicts(entry) == []
    assert heap_census.believable(entry)


def test_a_dropped_large_line_is_named_rather_than_read_as_fewer_blocks():
    lines = _with_large(FIRST, "composition start", [RESERVE, MAIN, TIMER])
    entry = _only([text for text in lines if "large [" not in text or " 1/2 " not in text])

    notes = heap_census.large_verdicts(entry)
    assert len(entry.large) == 2
    assert any("a large-block line is missing" in note for note in notes), notes


def test_more_large_blocks_than_the_table_holds_is_reported():
    entry = _only(_with_large(FIRST, "composition start", [RESERVE, MAIN], seen=40))

    notes = heap_census.large_verdicts(entry)
    assert any("more large blocks than the table holds (40 seen, 2 kept)" in note
               for note in notes), notes


def test_a_header_whose_kept_count_the_lines_do_not_carry_is_named():
    lines = _with_large(FIRST, "composition start", [RESERVE, MAIN, TIMER])
    entry = _only([text.replace("kept=3:", "kept=4:") for text in lines])

    notes = heap_census.large_verdicts(entry)
    assert any("the large-block lines carry 3 blocks, their header says 4 kept" in note
               for note in notes), notes


def test_a_cut_heap_hides_large_blocks():
    lines = _with_large(FIRST, "composition start", [RESERVE])
    entry = _only([text.replace("cut=0", "cut=1") if "spans [" in text else text
                   for text in lines])

    assert heap_census.large_verdicts(entry) == ["a cut heap traversal hides its blocks"]


def test_a_raced_walk_lends_none_of_its_large_blocks():
    lines = _with_large(FIRST, "composition start", [RESERVE])
    entry = _only([text.replace("ref=10/10", "ref=10/11") if "census [" in text else text
                   for text in lines])

    assert heap_census.large_verdicts(entry) == ["the heap moved during the walk"]


def test_the_report_names_each_large_block_by_the_stage_that_first_shows_it():
    first = _with_large(FIRST, "composition start", [RESERVE, MAIN])
    second = _with_large(SECOND, "after hydrate", [MAIN, TIMER])
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(first, second))))

    assert "composition_start  4ff30000  32768  (unnamed)  gone after composition_start" in report
    assert "composition_start  4ff21000  12800  main\n" in report
    assert "after_hydrate      4ff40000   4096  Tmr_Svc" in report
    assert "***" not in report.split("large blocks")[1], report


def test_a_boot_whose_firmware_spells_no_large_line_says_so_and_stays_believable():
    boots = heap_census.parse_boots(_log(FIRST, SECOND))
    report = "\n".join(heap_census.report_lines(boots))

    assert "large blocks: no stage reported its blocks past 1020 B" in report
    assert heap_census.believable(boots[-1][1]["after_hydrate"])


def test_a_size_family_arriving_between_two_stages_is_named_with_its_count():
    grown = [
        text.replace("0/3: 232x14", "0/3: 768x45 232x15") if "top [" in text else text
        for text in SECOND
    ]
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(FIRST, grown))))

    assert "composition_start -> after_hydrate  768x+45 (new in top)" in report
    assert "232x" not in report.split("top-size arrivals")[1], report


def test_a_stage_with_a_lost_large_line_lends_none_of_its_blocks_to_the_table():
    first = _with_large(FIRST, "composition start", [RESERVE, MAIN, TIMER])
    first = [text for text in first if "large [" not in text or " 1/2 " not in text]
    second = _with_large(SECOND, "after hydrate", [RESERVE, MAIN, TIMER])
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(first, second))))

    assert "after_hydrate      4ff30000  32768  (unnamed)" in report
    assert "after_hydrate      4ff40000   4096  Tmr_Svc" in report
    assert "composition_start  " not in report.split("first whole stage")[1].split("***")[0]
    assert "*** composition_start: a large-block line is missing" in report


def test_a_stage_whose_every_large_table_is_partial_reports_no_table():
    lines = _with_large(FIRST, "composition start", [RESERVE, MAIN], seen=40)
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(lines))))

    assert "no stage carried its whole large-block table" in report
    assert "*** composition_start: more large blocks than the table holds" in report


FULL_TOP = [(232, 14)] + [(400 + 4 * at, 10) for at in range(heap_census.TOP_EXACT_COUNT - 1)]


def _with_top(lines, entries):
    spelled = ["".join(" %dx%d" % entry for entry in entries[at:at + 8]) for at in (0, 8, 16)]
    out = []
    for text in lines:
        for chunk in range(3):
            if "top [" in text and " %d/3:" % chunk in text:
                text = text.split(" %d/3:" % chunk)[0] + " %d/3:%s" % (chunk, spelled[chunk])
        out.append(text)
    return out


def _arrival_report(before, after):
    return "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(before, after))))


def test_a_size_that_climbs_into_a_full_top_is_not_read_as_wholly_new():
    climbed = FULL_TOP[1:] + [(512, 7)]
    report = _arrival_report(_with_top(FIRST, FULL_TOP), _with_top(SECOND, climbed))

    assert "512x+" not in report, report


def test_a_size_that_entered_a_full_top_counts_only_what_it_must_have_gained():
    entered = FULL_TOP[1:] + [(1000, 5)]
    report = _arrival_report(_with_top(FIRST, FULL_TOP), _with_top(SECOND, entered))

    assert "1000x+2 (entered the top: at least)" in report, report


def test_a_size_absent_from_a_top_with_room_had_no_blocks_and_counts_whole():
    report = _arrival_report(FIRST, _with_top(SECOND, [(232, 14), (512, 7)]))

    assert "composition_start -> after_hydrate  512x+7 (new in top)" in report, report


def test_the_top_capacity_matches_the_firmware_table():
    source = pathlib.Path(__file__).resolve().parents[3] / "crates/dali2rust-bsp/src/heap_census.rs"
    spelled = re.search(r"pub const TOP_EXACT_COUNT: usize = (\d+);", source.read_text())

    assert spelled and int(spelled.group(1)) == heap_census.TOP_EXACT_COUNT


def test_a_lost_top_line_leaves_the_pair_uncompared_rather_than_inventing_an_arrival():
    grown = [text.replace("0/3: 232x14", "0/3: 232x14 768x45") if "top [" in text else text
             for text in SECOND]
    first = [text for text in FIRST if "top [" not in text or " 1/3:" not in text]
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(first, grown))))

    assert ("composition_start -> after_hydrate  not compared: "
            "composition_start: a top-size line is missing from the log (2 of 3)") in report
    assert "768x+" not in report


def test_a_top_line_lost_from_the_later_stage_also_leaves_the_pair_uncompared():
    grown = [text.replace("0/3: 232x14", "0/3: 232x14 768x45") if "top [" in text else text
             for text in SECOND]
    grown = [text for text in grown if "top [" not in text or " 2/3:" not in text]
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(FIRST, grown))))

    assert ("composition_start -> after_hydrate  not compared: "
            "after_hydrate: a top-size line is missing from the log (2 of 3)") in report
    assert "768x+" not in report


def test_a_cut_walk_leaves_its_pair_uncompared_rather_than_inventing_an_arrival():
    cut = [text.replace("cut=0", "cut=1") if "spans [" in text else text for text in FIRST]
    cut = [text.replace("0/3: 232x14", "0/3: 232x2") if "top [" in text else text for text in cut]
    report = _arrival_report(cut, SECOND)

    assert ("composition_start -> after_hydrate  not compared: "
            "composition_start: a cut heap traversal hides its blocks") in report, report
    assert "232x+" not in report


def test_a_raced_walk_leaves_its_pair_uncompared():
    raced = [text.replace("ref=14/14", "ref=14/15") if "census [" in text else text
             for text in _with_top(SECOND, [(232, 14), (768, 45)])]
    report = _arrival_report(FIRST, raced)

    assert "after_hydrate: the heap moved during the walk" in report, report
    assert "768x+" not in report


def test_a_stage_whose_every_large_line_was_lost_is_named_not_skipped():
    before = _with_large(FIRST, "composition start", [RESERVE])
    middle = [text for text in _with_large(SECOND, "after hydrate", [RESERVE, TIMER])
              if "large [" not in text]
    after = _with_large(_stage_lines("before httpd", 14, 5000, {0: (6, 96), 5: (4, 904),
                                                                9: (4, 4000)}),
                        "before httpd", [RESERVE, TIMER])
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(before, middle, after))))

    assert "before_httpd       4ff40000   4096  Tmr_Svc" in report
    assert "*** after_hydrate: no large-block line reached the log for this stage" in report


def test_the_large_table_names_the_earliest_whole_stage_it_starts_from():
    first = _with_large(FIRST, "composition start", [RESERVE])
    second = _with_large(SECOND, "after hydrate", [RESERVE, TIMER])
    report = "\n".join(heap_census.report_lines(heap_census.parse_boots(_log(first, second))))

    assert "those under composition_start were there before it:" in report
