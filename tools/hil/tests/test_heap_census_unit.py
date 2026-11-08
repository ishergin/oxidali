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
