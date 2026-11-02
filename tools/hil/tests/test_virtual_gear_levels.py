import pytest

from hil import gearsim
from hil.lamp_guard import RULE_SEPARATOR, http_rule
from hil.wait import wait_until

pytestmark = pytest.mark.virtual_gear

LAMP_TARGET = "lamp"
RULE_PREFIX = "hil-vg-09-"
START_LEVEL = 120
NARROW_MIN = 40
NARROW_MAX = 200
UP, DOWN, RECALL_MAX, RECALL_MIN = 0x01, 0x02, 0x05, 0x06
STEPS = (("dim_up", UP, True), ("dim_down", DOWN, False))
RECALLS = (("recall_max", RECALL_MAX, NARROW_MAX), ("recall_min", RECALL_MIN, NARROW_MIN))
EXTENDED_FADE_MS = 300_000
EXTENDED_FADE_MINUTE_MS = 60_000
EXTENDED_FADE_MINUTE_MULTIPLIER = 4
EXTENDED_FADE_MULTIPLIER_SHIFT = 4
EXTENDED_FADE_BASE_OFFSET = 1
HEARD_TIMEOUT_S = 10.0
LEVEL_WAIT_S = 10.0
POLL_S = 0.3


def _extended_fade_byte():
    base = EXTENDED_FADE_MS // EXTENDED_FADE_MINUTE_MS - EXTENDED_FADE_BASE_OFFSET
    return "0x%02x" % ((EXTENDED_FADE_MINUTE_MULTIPLIER << EXTENDED_FADE_MULTIPLIER_SHIFT)
                       | base)


def _rule_name(verb):
    return RULE_PREFIX + verb.replace("_", "-")


def _narrow_the_range(api, virtual_bench, attr_guard, short):
    attr_guard(short, "max_level", "min_level", verify=True, required=True)
    oracle = virtual_bench.oracle
    with oracle.window() as window:
        written = api.wait_op(api.write_attrs(short, {"min_level": NARROW_MIN,
                                                      "max_level": NARROW_MAX}))
        assert written.get("status") == "succeeded", written
        oracle.expect(window, short, "level_range", "%d..%d" % (NARROW_MIN, NARROW_MAX))
    api.vlamps.ts(virtual_bench.vl(short), {"power": "on", "level": START_LEVEL})
    assert wait_until(lambda: api.actual_levels([short]) == {short: START_LEVEL},
                      LEVEL_WAIT_S, interval_s=POLL_S), api.actual_levels([short])


def _heard(window, short, opcode):
    frame = (gearsim.command_address(short), opcode)
    return wait_until(lambda: frame in [(f.address, f.data) for f in window.heard()],
                      HEARD_TIMEOUT_S, interval_s=POLL_S)


def _dapcs(window, short):
    dapc_address = gearsim.command_address(short) & ~gearsim.SELECTOR
    return [f for f in window.heard() if f.address == dapc_address]


def _level_moved(oracle, window, short, rises):
    def moved():
        for change in oracle.changes(window):
            if change.short == short and change.field == "level":
                return (int(change.new) > int(change.old)) == rises
        return False
    return wait_until(moved, LEVEL_WAIT_S, interval_s=POLL_S)


def _recall_lands(oracle, window, short, opcode, bound):
    oracle.expect(window, short, "level", bound)
    assert _heard(window, short, opcode), (
        "SA%d reached %d but the gear never heard 0x%02x: %r"
        % (short, bound, opcode, window.heard()))
    assert not _dapcs(window, short), "a DAPC reached SA%d: %r" % (short, _dapcs(window, short))


@pytest.mark.hil_id("HIL-VG-08")
def test_a_recall_of_the_level_range_sends_its_own_command_and_lands_on_the_bound(
        api, virtual_bench, attr_guard):
    short, others = virtual_bench.park[0], virtual_bench.park[1:]
    oracle = virtual_bench.oracle
    _narrow_the_range(api, virtual_bench, attr_guard, short)
    for recall, opcode, bound in (("max_level", RECALL_MAX, NARROW_MAX),
                                  ("min_level", RECALL_MIN, NARROW_MIN)):
        with oracle.hearing() as window:
            api.ts(short, {"recall": recall})
            _recall_lands(oracle, window, short, opcode, bound)
            oracle.untouched(window, others)


def _steps_move_the_level(oracle, run, short):
    for verb, opcode, rises in STEPS:
        with oracle.hearing() as window:
            run(verb)
            assert _heard(window, short, opcode), (verb, window.heard())
            assert _level_moved(oracle, window, short, rises), (
                "%s did not move SA%d %s: %s" % (verb, short, "up" if rises else "down",
                                                 oracle.changes(window)))
            assert not _dapcs(window, short), (verb, _dapcs(window, short))


def _up_leaves_a_dark_lamp_dark(api, oracle, run, short, lamp):
    with oracle.window() as window:
        api.vlamps.ts(lamp, {"power": "off"})
        oracle.expect(window, short, "level", 0)
    with oracle.hearing() as window:
        run("dim_up")
        assert _heard(window, short, UP), window.heard()
        oracle.untouched(window, [short])


@pytest.mark.hil_id("HIL-VG-09")
def test_rule_recalls_and_steps_reach_the_lamp_and_up_never_lights_a_dark_one(
        api, virtual_bench, attr_guard, rules_guard, op_check):
    short = virtual_bench.park[0]
    lamp = virtual_bench.vl(short)
    oracle = virtual_bench.oracle
    _narrow_the_range(api, virtual_bench, attr_guard, short)
    verbs = [verb for verb, _, _ in STEPS + RECALLS]
    rules_guard(RULE_SEPARATOR.join(
        http_rule(_rule_name(verb), LAMP_TARGET, lamp, "%s()" % verb) for verb in verbs))

    def run(verb):
        op_check(api.wait_op(api.rules_run(_rule_name(verb))))

    _steps_move_the_level(oracle, run, short)
    for verb, opcode, bound in RECALLS:
        with oracle.hearing() as window:
            run(verb)
            _recall_lands(oracle, window, short, opcode, bound)
    _up_leaves_a_dark_lamp_dark(api, oracle, run, short, lamp)


@pytest.mark.hil_id("HIL-VG-10")
def test_an_extended_fade_beyond_sixteen_bits_is_stored_in_its_iec_encoding(
        api, virtual_bench, attr_guard):
    short = virtual_bench.park[0]
    oracle = virtual_bench.oracle
    attr_guard(short, "extended_fade_time_ms", verify=True, required=True)
    with oracle.window() as window:
        written = api.wait_op(api.write_attrs(short, {"extended_fade_time_ms": EXTENDED_FADE_MS}))
        assert written.get("status") == "succeeded", written
        oracle.expect(window, short, "extended_fade_time", _extended_fade_byte())
    api.attr_read_checked(short, groups="extended")
    held = (((api.attributes(short, ["extended"]).get("attributes") or {})
             .get("extended") or {}).get("fade_time_ms") or {}).get("value")
    assert held == EXTENDED_FADE_MS, "SA%d reads back %r ms" % (short, held)
