import threading
import time

import pytest

pytestmark = pytest.mark.slow

QUERY_STATUS = 0x90

FOREIGN_TARGET_SHORT = 20

FOREIGN_BURST_FRAMES = 30

MIN_COLOUR_WRITES = 3


def _delta(before, after, key):
    return after.get(key, 0) - before.get(key, 0)


def _pairs(gear_sim):
    return gear_sim.stats().get("send_twice_pairs", 0)


def _assert_pairs_held(gear_sim, before, after, before_pairs):
    executed = _pairs(gear_sim) - before_pairs
    assert _delta(before, after, "send_twice_interloper") == 0, (
        "a configuration command was split from its twin; %d pair(s) completed "
        "in the same window. Read this against the DUT's retry counters before "
        "calling it a framing defect: a retried unit legitimately re-sends "
        "ENABLE, CMD, CMD from the top" % executed
    )
    print(
        "send-twice pairs the emulator executed: %d%s"
        % (executed, "" if executed else " (emulated fleet disabled — no evidence)")
    )


def _first_short(api):
    addrs = api.addrs()
    if not addrs:
        pytest.skip("no addressed gear on the segment")
    return addrs[0]


class _ForeignPressure:
    def __init__(self, foreign, frames):
        self._foreign = foreign
        self._frames = frames
        self._thread = None
        self.error = None

    def __enter__(self):
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return self

    def _run(self):
        try:
            self._foreign.send_frames(self._frames)
        except Exception as exc:
            self.error = exc

    def in_flight(self):
        return self._thread is not None and self._thread.is_alive()

    def __exit__(self, *exc_info):
        self._thread.join(timeout=120)
        return False

    def assert_delivered(self):
        if self.error is not None:
            raise AssertionError(
                "the foreign master did not transmit, so this run applied no "
                "pressure and its zero means nothing: %s" % self.error
            )


def test_a_quiet_bus_produces_no_protocol_violations(api, gear_sim):
    short = _first_short(api)
    before = gear_sim.violations()
    before_pairs = _pairs(gear_sim)

    api.attr_read_checked(short)

    after = gear_sim.violations()
    assert _delta(before, after, "enable_consumed") == 0, (
        "no foreign master was transmitting, so nothing can have split a sequence"
    )
    _assert_pairs_held(gear_sim, before, after, before_pairs)


def test_a_split_sequence_is_visible_from_the_receiving_end(api, gear_sim, foreign):
    short = _first_short(api)
    burst = [
        {"addr": FOREIGN_TARGET_SHORT << 1, "data": 10 + (i % 2) * 190}
        for i in range(FOREIGN_BURST_FRAMES)
    ]
    before = gear_sim.violations()
    before_pairs = _pairs(gear_sim)

    pressure = _ForeignPressure(foreign, burst)
    writes = 0
    with pressure:
        while pressure.in_flight() or writes < MIN_COLOUR_WRITES:
            api.ts(
                short,
                {
                    "power": "on",
                    "level": 180,
                    "color_mode": "cct",
                    "color_temperature_kelvin": 3000,
                },
            )
            writes += 1
    pressure.assert_delivered()
    print("colour writes issued under foreign pressure: %d" % writes)

    after = gear_sim.violations()
    assert _delta(before, after, "enable_consumed") == 0, (
        "a DT8 colour write must survive a bus another master is using: the "
        "prelude and its extended frame are one transaction"
    )
    _assert_pairs_held(gear_sim, before, after, before_pairs)


def test_no_forward_frame_is_sent_below_the_priority_one_floor(api, gear_sim):
    short = _first_short(api)
    before = gear_sim.violations()
    before_fwd = gear_sim.stats().get("forward16", 0)

    for _ in range(20):
        api.cmd(short, QUERY_STATUS)

    bands = gear_sim.settle_bands()
    if not bands:
        pytest.skip("the emulator heard no forward frames — nothing to judge")

    after = gear_sim.violations()
    drove = gear_sim.stats().get("forward16", 0) - before_fwd
    assert _delta(before, after, "below_p1_floor") == 0, (
        "a forward frame arrived below the 13,5 ms priority-1 floor while this "
        "test drove %d frames; bands seen: %r" % (drove, bands)
    )


def test_the_priority_ladder_is_visible_on_the_wire(api, gear_sim):
    short = _first_short(api)
    api.attr_read_checked(short)
    api.ts(short, {"power": "on", "level": 120})

    bands = gear_sim.settle_bands()
    if not bands:
        pytest.skip("the emulator heard no forward frames — nothing to report")

    wire = api.diagnostics().get("dali_wire", {})
    print("observed settling bands (C6): %r" % bands)
    print("intended priorities (DUT): %r" % wire.get("frames_sent_by_priority"))
    assert sum(bands.values()) > 0


def test_the_wire_counters_report_transactions_and_stay_within_their_gates(api):
    short = _first_short(api)
    before = api.diagnostics().get("dali_wire", {})

    api.attr_read_checked(short)

    after = api.diagnostics().get("dali_wire", {})
    assert _delta(before, after, "transactions_started") > 0, (
        "an attribute read is a series of transactions; none was recorded"
    )
    assert _delta(before, after, "transaction_leaks") == 0, (
        "a transaction outlived its command — priority 1 was armed outside one "
        f"(since boot: {after.get('transaction_leaks', 0)})"
    )
    assert _delta(before, after, "transaction_budget_exceeded") == 0, (
        "a unit that is not declared indivisible ran past the §9.2 400 ms "
        "guidance; its transaction boundary is in the wrong place "
        f"(since boot: {after.get('transaction_budget_exceeded', 0)})"
    )
