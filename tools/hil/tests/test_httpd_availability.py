import statistics
import threading
import time

import pytest

MAX_HEALTH_LATENCY_S = 0.30

MIN_PROBE_COVERAGE = 0.8


def _full_scene_rows(snapshot):
    return [
        {"virtual_lamp_id": row["virtual_lamp_id"], "desired": {"included": False}}
        for row in snapshot["rows"]
    ]


IDLE_PROBES = 10


def _probe_health(api):
    started = time.monotonic()
    api.http.get(api._url("health"), timeout=api.timeout_s).raise_for_status()
    return time.monotonic() - started


def _idle_health_latency(api):
    return max(_probe_health(api) for _ in range(IDLE_PROBES))


@pytest.mark.hil_id("HIL-HTTP-01")
@pytest.mark.slow
def test_health_stays_served_during_a_64_row_scene_write(api, scene_matrix_guard,
                                                         test_artifacts):
    scene_id = 3
    snapshot = scene_matrix_guard(scene_id)
    rows = _full_scene_rows(snapshot)
    assert len(rows) == 64, "the point of this test is the full 16-chunk series"
    idle_s = _idle_health_latency(api)

    latencies = []
    stop = threading.Event()
    probe_error = []
    probe_span = []

    def probe():
        while not stop.is_set():
            started = time.monotonic()
            try:
                latency = _probe_health(api)
            except Exception as exc:
                probe_error.append(repr(exc))
                return
            done = started + latency
            latencies.append(latency)
            if not probe_span:
                probe_span.append(started)
            probe_span[1:] = [done]

    prober = threading.Thread(target=probe, daemon=True)
    prober.start()
    write_started = time.monotonic()
    try:
        api.scenes.matrix_put(scene_id, rows)
    finally:
        write_elapsed = time.monotonic() - write_started
        stop.set()
        prober.join(timeout=10)

    covered = (probe_span[1] - probe_span[0]) if len(probe_span) == 2 else 0.0
    test_artifacts.attach_json("httpd_availability", {
        "write_elapsed_s": write_elapsed,
        "health_probes": len(latencies),
        "health_covered_s": covered,
        "health_max_s": max(latencies) if latencies else None,
        "health_median_s": statistics.median(latencies) if latencies else None,
        "health_idle_max_s": idle_s,
        "probe_error": probe_error,
    })

    assert not probe_error, probe_error
    assert latencies, "no health probe completed at all"
    assert covered >= MIN_PROBE_COVERAGE * write_elapsed, (
        "health probing covered only %.3f s of a %.3f s write (%d probes): the "
        "latency assertion below would be about a window nobody watched"
        % (covered, write_elapsed, len(latencies))
    )
    blocked = max(latencies) - idle_s
    assert blocked < MAX_HEALTH_LATENCY_S, (
        "health blocked for %.2f s beyond its idle %.3f s while a 64-row scene "
        "write ran; the httpd task was monopolised (ISSUE-25 defect 2). probes=%d "
        "median=%.3f write=%.2f s"
        % (blocked, idle_s, len(latencies), statistics.median(latencies), write_elapsed)
    )


READ_SURFACE = ("list", "core", "attributes", "memory-banks")


def _read_all_surfaces(api, short):
    return {
        "list": api.devices(),
        "core": api.state(short),
        "attributes": api.attributes(short),
        "memory-banks": api.memory_banks(short),
    }


@pytest.mark.smoke
def test_every_read_surface_answers_without_faulting_the_httpd_stack(
    api, serial_log, test_artifacts
):
    shorts = api.addrs()
    if not shorts:
        pytest.skip("empty registry — nothing to build a response from")

    sizes = {}
    with serial_log.window() as serial_win:
        for short in shorts:
            for name, body in _read_all_surfaces(api, short).items():
                assert body is not None, "%s answered nothing for short %d" % (name, short)
                sizes.setdefault(name, []).append(len(str(body)))
            one = api.attributes(short, ["memory_diagnostics"])
            sizes.setdefault("one-section", []).append(len(str(one)))
        rebooted = serial_win.reboot_detected()

    test_artifacts.attach_json("read_surface_sizes", {
        name: {"n": len(v), "max": max(v), "total": sum(v)}
        for name, v in sizes.items()
    })
    assert not rebooted, (
        "the DUT rebooted while building read responses — this is the ISSUE-49 "
        "shape (a response assembled on the httpd task's 36 KiB stack), and the "
        "serial log holds the fault"
    )
    assert max(sizes["one-section"]) < max(sizes["attributes"]), (
        "a single-section response is no smaller than the full one; "
        "`?sections=` is not filtering"
    )


@pytest.mark.smoke
def test_the_device_list_stays_a_summary(api, test_artifacts):
    MAX_SUMMARY_BYTES = 2048

    body = api.devices()
    rows = body["physical_devices"]
    if not rows:
        pytest.skip("empty registry")
    widest = max(len(str(row)) for row in rows)
    test_artifacts.attach_json("list_row_sizes", {
        "devices": len(rows), "widest_row_bytes": widest,
        "total_bytes": len(str(body)),
    })
    assert widest < MAX_SUMMARY_BYTES, (
        "a list row is %d B: an attribute section has moved back onto the list, "
        "which is what made this response 111 KB and faulted the httpd stack"
        % widest
    )
    for row in rows:
        assert "attributes" not in row, (
            "device %r carries attributes in the LIST" % row.get("short_address")
        )
        assert "memory_banks" not in row, (
            "device %r carries bank coverage in the LIST" % row.get("short_address")
        )
