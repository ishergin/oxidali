import pytest

import hil_fake_clock
import test_bus_load as bus_load
from hil import wait

PEAK = 141
QUIET = 60
FOREIGN = 166
OWN_CAP = 40
GAUGE_WINDOW_S = 1.0
WAIT_STARTS_S = bus_load.PEAK_WATCH_S
HALF_WAIT_S = WAIT_STARTS_S + bus_load.RECEDE_WITHIN_S / 2
PAST_DEADLINE_S = WAIT_STARTS_S + bus_load.RECEDE_WITHIN_S + 2 * bus_load.WATCH_POLL_S


class _Wire:
    def __init__(self, clock, load_at, own_at=None):
        self.clock, self.load_at = clock, load_at
        self.own_at = own_at or (lambda t, load: min(load, OWN_CAP))
        self.tx = 0
        self.active = 0

    def present_addrs(self):
        return [1]

    def raw(self, frame, expects_backward):
        self.tx += bus_load.FORWARD16_TICKS
        self.active += bus_load.FORWARD16_TICKS + bus_load.BACKWARD8_TICKS
        return {"success": True}

    def diagnostics(self):
        t = self.clock.now
        load = self.load_at(t)
        return {"dali_wire": {
            "wire_ticks_total": int(t * bus_load.TICKS_PER_SECOND),
            "wire_ticks_active": self.active,
            "wire_ticks_tx": self.tx,
            "load_permille": load,
            "load_own_permille": self.own_at(t, load),
        }}


@pytest.fixture
def clock(monkeypatch):
    return hil_fake_clock.install(monkeypatch, bus_load, wait)


def _burst_then(load_after_watch):
    def load_at(t):
        return PEAK if t < WAIT_STARTS_S else load_after_watch(t)
    return load_at


def _run(wire):
    bus_load.test_a_burst_of_queries_is_booked_as_our_load_and_recedes(wire)


def test_a_gauge_that_recedes_passes_though_a_foreign_burst_follows(clock):
    _run(_Wire(clock, _burst_then(lambda t: QUIET if t < HALF_WAIT_S else FOREIGN)))


def test_a_gauge_that_recedes_late_in_the_wait_for_one_window_passes(clock):
    def load_at(t):
        if t < HALF_WAIT_S:
            return PEAK
        return QUIET if t < HALF_WAIT_S + GAUGE_WINDOW_S else FOREIGN
    _run(_Wire(clock, _burst_then(load_at)))


def test_a_gauge_that_recedes_only_after_the_deadline_fails(clock):
    load_at = _burst_then(lambda t: PEAK if t < PAST_DEADLINE_S else QUIET)
    with pytest.raises(AssertionError, match="must recede"):
        _run(_Wire(clock, load_at))


def test_an_inconsistent_sample_while_waiting_fails(clock):
    def own_at(t, load):
        return load + 1 if t >= WAIT_STARTS_S else min(load, OWN_CAP)
    with pytest.raises(AssertionError, match="per mille"):
        _run(_Wire(clock, _burst_then(lambda t: QUIET), own_at))
