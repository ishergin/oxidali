import pytest

from hil.wait import wait_until

pytestmark = pytest.mark.sniffer


@pytest.mark.hil_id("HIL-DIAG-02")
def test_raw_frame_exact_echo_and_backward(api, sniffer, paced, lamps,
                                           state_snapshot, test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    dapc_frame = (short << 9) | 120
    with sniffer.window() as win:
        paced(1.0)
        resp = api.raw(dapc_frame)
        assert resp.get("success") is True, resp
        win.expect_frame("DAPC short %d -> level 120" % short,
                         resend=lambda: api.raw(dapc_frame))

    query_frame = (((short << 1) | 1) << 8) | 0xA0
    with sniffer.window() as win:
        paced(1.0)
        resp = api.raw(query_frame, expects_backward=True)
        test_artifacts.attach_json("raw_query", resp)
        assert resp.get("success") is True, resp
        assert resp.get("backward_frame") is not None, resp
        win.expect_frame("backward")
    api.off(short)


@pytest.mark.hil_id("HIL-DIAG-03")
def test_command_repeat_reaches_wire_twice(api, sniffer, paced, lamps,
                                           free_group, state_snapshot,
                                           test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    expect = "ADD TO GROUP %d (short %d)" % (free_group, short)
    try:
        with sniffer.window() as win:
            paced(1.0)
            api.cmd(short, 0x60 + free_group, repeat=2)
            twins = []

            def _pair_captured():
                nonlocal twins
                twins = [f for f in win.frames() if expect in f["decoded"]]
                return len(twins) >= 2

            wait_until(_pair_captured, 8.0, interval_s=0.4)
        test_artifacts.attach_json(
            "twins", [{"counter": f["counter"], "ts": f["ts"]} for f in twins])
        assert len(twins) >= 2, \
            "send-twice pair not captured: %s" % [f["decoded"]
                                                 for f in win.frames()][-8:]
        c1, c2 = sorted(f["counter"] for f in twins[:2])
        assert (c2 - c1) % 65536 == 1, (c1, c2)
    finally:
        api.cmd(short, 0x70 + free_group, repeat=2)


QUERY_STATUS = 0x90
QUERY_ACTUAL_LEVEL = 0xA0
FADE_RUNNING = 0x10
FADE_SETTLE_S = 20.0
STATUS_POLL_S = 0.3


def _fade_settled(api, short):
    status = api.cmd(short, QUERY_STATUS)
    return status.get("success") is True and not status.get("backward_frame", 0) & FADE_RUNNING


@pytest.mark.hil_id("HIL-DIAG-04")
def test_query_actual_level_matches_state(api, lamps, wait_state,
                                          state_snapshot, test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    api.ts(short, {"power": "on", "level": 130})
    last = wait_state(short, lambda s: s.get("level") == 130)
    assert last.get("level") == 130, last
    settled = wait_until(lambda: _fade_settled(api, short), FADE_SETTLE_S,
                         interval_s=STATUS_POLL_S)
    resp = api.cmd(short, QUERY_ACTUAL_LEVEL)
    test_artifacts.attach_json("query_actual_level",
                               {"fade_settled": bool(settled), "answer": resp})
    assert resp.get("success") is True, resp
    assert resp.get("backward_frame") == 130, (
        resp, "fade settled" if settled else
        "fadeRunning still set after %.0f s" % FADE_SETTLE_S)
    api.off(short)


@pytest.mark.hil_id("HIL-DIAG-05")
def test_query_to_absent_address_reports_no_answer(api, sniffer, paced,
                                                  test_artifacts):
    known = set(api.addrs())
    absent = next((a for a in range(63, -1, -1) if a not in known), None)
    if absent is None:
        pytest.skip("every short address is occupied (full emulator fleet) — "
                    "no address can be queried as absent")
    with sniffer.window() as win:
        paced(1.0)
        status, body = api.raw_request(
            "POST", "dali/command",
            {"wire_address": (absent << 1) | 1, "command": 0x90,
             "repeat_count": 1})
        win.expect_frame("QUERY STATUS (short %d)" % absent)
    test_artifacts.attach_json("no_answer", {"status": status, "body": body})
    assert status == 200, (status, body)
    assert body.get("backward_frame") in (None, 0), body


RESTORED_102_QUERIES = (
    (0xA6, "QUERY MANUFACTURER SPECIFIC MODE"),
    (0xAA, "QUERY CONTROL GEAR FAILURE"),
)


@pytest.mark.hil_id("HIL-DIAG-06")
def test_the_restored_102_queries_are_accepted_and_reach_the_wire(
        api, sniffer, paced, lamps, state_snapshot, test_artifacts):
    short = lamps.by_label[lamps.labels()[0]]
    observed = {}
    for opcode, name in RESTORED_102_QUERIES:
        with sniffer.window() as win:
            paced(1.0)
            status, body = api.raw_request(
                "POST", "dali/command",
                {"wire_address": (short << 1) | 1, "command": opcode,
                 "repeat_count": 1})
            observed["%#04x" % opcode] = {"status": status, "body": body}
            assert status == 200, (
                "%s (%#04x) must reach the validated path; 400 here is the "
                "ISSUE-58 gap reopening: %r" % (name, opcode, (status, body))
            )
            win.expect_frame("short %d" % short)
    test_artifacts.attach_json("restored_102_queries", observed)
