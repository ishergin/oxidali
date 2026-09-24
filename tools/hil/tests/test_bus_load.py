import time

import pytest

QUERY_STATUS = 0x90
BURST = 30


def _query_status_frame(short: int) -> int:
    return (((short << 1) | 1) << 8) | QUERY_STATUS
PHY_TICK_US = 104
TICKS_PER_SECOND = 1_000_000 // PHY_TICK_US
FORWARD16_TICKS = 152
BACKWARD8_TICKS = 88
FLOOR = 0.6
LOAD_FLOOR_PERMILLE = 100
PEAK_WATCH_S = 2.0
SETTLE_S = 6.0
U32 = 1 << 32


def _wire(api):
    return api.diagnostics()["dali_wire"]


def _delta(now, before):
    return (now - before) % U32


def _assert_consistent(wire):
    assert 0 <= wire["load_own_permille"] <= wire["load_permille"] <= 1000, (
        "a gauge is per mille and our share cannot exceed the whole: %r" % wire)


@pytest.mark.hil_id("HIL-WIRE-01")
def test_the_interrupt_is_booking_wire_time(api):
    a = _wire(api)
    assert a["wire_ticks_total"] > 0, "not measured on this transport: %r" % a
    time.sleep(1.0)
    b = _wire(api)
    d_total = _delta(b["wire_ticks_total"], a["wire_ticks_total"])
    assert 0.8 * TICKS_PER_SECOND < d_total < 2.0 * TICKS_PER_SECOND, (
        "the PHY should book ~%d ticks per second, saw %d" % (TICKS_PER_SECOND, d_total))
    d_active = _delta(b["wire_ticks_active"], a["wire_ticks_active"])
    d_tx = _delta(b["wire_ticks_tx"], a["wire_ticks_tx"])
    assert d_tx <= d_active <= d_total, (
        "tx ⊆ active ⊆ total over one window: tx=%d active=%d total=%d"
        % (d_tx, d_active, d_total))
    _assert_consistent(b)


@pytest.mark.hil_id("HIL-WIRE-02")
def test_a_burst_of_queries_is_booked_as_our_load_and_recedes(api):
    present = api.present_addrs()
    if not present:
        pytest.skip("no present gear to query")
    frame = _query_status_frame(present[0])
    before = _wire(api)
    answered = 0
    for _ in range(BURST):
        if api.raw(frame, expects_backward=True).get("success"):
            answered += 1
    after = _wire(api)

    d_tx = _delta(after["wire_ticks_tx"], before["wire_ticks_tx"])
    d_active = _delta(after["wire_ticks_active"], before["wire_ticks_active"])
    assert d_tx >= BURST * FORWARD16_TICKS * FLOOR, (
        "%d forward frames should book ≥ %d tx ticks, saw %d"
        % (BURST, BURST * FORWARD16_TICKS * FLOOR, d_tx))
    assert d_active - d_tx >= answered * BACKWARD8_TICKS * FLOOR, (
        "%d answers should book ≥ %d ticks beyond our own, saw %d"
        % (answered, answered * BACKWARD8_TICKS * FLOOR, d_active - d_tx))

    peak = after["load_permille"]
    deadline = time.monotonic() + PEAK_WATCH_S
    while time.monotonic() < deadline:
        wire = _wire(api)
        _assert_consistent(wire)
        peak = max(peak, wire["load_permille"])
        time.sleep(0.2)
    assert peak >= LOAD_FLOOR_PERMILLE, (
        "a burst of %d queries should lift the gauge past %d‰, peak was %d‰"
        % (BURST, LOAD_FLOOR_PERMILLE, peak))

    time.sleep(SETTLE_S)
    quiet = _wire(api)
    _assert_consistent(quiet)
    assert quiet["load_permille"] < peak, (
        "the gauge must recede once the burst is over: peak %d‰, now %d‰"
        % (peak, quiet["load_permille"]))
