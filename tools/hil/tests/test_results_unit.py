import pytest

from hil.foreign import ForeignMasterSilent
from hil.results import FAILURE_LINE_CHARS, failure_line

GATEWAY = "<< 0x0190 gateway error: frame not sent, bus busy for 812 ms"
SILENT = ("wb-mqtt-dali accepted 1 frame(s) over WS and they did not all reach the "
          "wire within 3.0s, over 3 attempts; the WB's own monitor said: %s. The bench "
          "foreign master is jammed, not the firmware — `systemctl restart wb-mqtt-dali` "
          "on root@wb (STRATEGY.md §2)." % GATEWAY)
LOCALS = {"frames": [{"addr": 0x01, "data": 0x90}] * 40, "api": "x" * 300}


def _send(frames, api):
    raise ForeignMasterSilent(SILENT)


def _long_repr():
    try:
        _send(**LOCALS)
    except ForeignMasterSilent:
        return pytest.ExceptionInfo.from_current().getrepr(funcargs=True)


def test_the_line_is_the_crash_not_the_tests_locals():
    longrepr = _long_repr()
    assert "ForeignMasterSilent" not in str(longrepr)[:120], "the premise: the head is locals"
    line = failure_line(longrepr)
    assert line.startswith("hil.foreign.ForeignMasterSilent: wb-mqtt-dali accepted")
    assert GATEWAY in line and line.endswith("(STRATEGY.md §2).")


def test_a_repr_without_a_crash_gives_its_last_line():
    text = "the DUT REBOOTED during this test\n\nEverything measured here ran fresh.\n"
    assert failure_line(text) == "Everything measured here ran fresh."
    assert failure_line("") == ""


def test_a_multi_line_crash_message_is_one_line_and_bounded():
    class Crash:
        message = "Failed: first line\n  second line\n" + "x" * (2 * FAILURE_LINE_CHARS)

    class Repr:
        reprcrash = Crash()

    line = failure_line(Repr())
    assert "\n" not in line and line.startswith("Failed: first line second line x")
    assert len(line) == FAILURE_LINE_CHARS
