import errno
import socket
import subprocess
import time
from pathlib import Path

import pytest
import serial

import hil_optics
from hil.camera import backend as camera_backend

pytest_plugins = ["pytester"]

CONFTEST = Path(__file__).resolve().parent / "conftest.py"
CLOSED = "http://127.0.0.1:9"
NO_RETRY_BUDGET_S = 10.0
UNIT_PROBE = "def test_nothing():\n    assert True\n"
BENCH_PROBE = "def test_reaches(api):\n    assert api\n"
OPTICAL_PROBE = "def test_sees(calibration):\n    assert calibration\n"
HALTING_PROBE = ("from hil import remote_serial\n\n\n"
                 "def test_halts(tmp_path):\n    remote_serial.control(None, 'run')\n")
SILENT_SKIP_PROBE = "import pytest\n\n\ndef test_skips():\n    pytest.skip('no camera here')\n"
DECLARED_SKIP_PROBE = ("import pytest\n\n\n@pytest.mark.skip(reason='declared')\n"
                       "def test_skips():\n    pass\n")
REFUSED_PROBE = ("from hil.lamp_guard import LampNotAllowed\n\n\n"
                 "def test_refused():\n    raise LampNotAllowed('SA1 is outside the allowlist')\n")
MODULE_SKIP_PROBE = "import pytest\n\npytest.skip('gone', allow_module_level=True)\n"
DESTRUCTIVE_PAIR = ("import pytest\n\n\n@pytest.mark.destructive\ndef test_one():\n    pass\n\n\n"
                    "@pytest.mark.destructive\ndef test_two():\n    pass\n\n\n"
                    "def test_plain():\n    pass\n")


@pytest.fixture
def exits(monkeypatch, tmp_path):
    seen = []

    def connect(sock, address):
        seen.append(("connect", address))
        raise ConnectionRefusedError(errno.ECONNREFUSED, "refused by the unit test")

    def spawn(*args, **kwargs):
        seen.append(("spawn", args[0] if args else kwargs.get("args")))
        raise OSError(errno.EPERM, "spawning refused by the unit test")

    def open_serial(*args, **kwargs):
        seen.append(("serial", args))
        raise serial.SerialException("serial refused by the unit test")

    monkeypatch.setattr(socket.socket, "connect", connect)
    monkeypatch.setattr(subprocess, "Popen", spawn)
    monkeypatch.setattr(serial, "serial_for_url", open_serial)
    monkeypatch.setattr(serial.Serial, "open", open_serial)
    for name, value in {"HIL_BASE": CLOSED, "HIL_PEER_BASE": CLOSED,
                        "HIL_SERIAL_REMOTE": "nobody@127.0.0.1:/dev/hil-unit-test",
                        "HIL_WB_SSH": "nobody@127.0.0.1", "HIL_NO_CAMERA": "1",
                        "HIL_RUN_DIR": str(tmp_path / "run")}.items():
        monkeypatch.setenv(name, value)
    return seen


def _session(pytester, **modules):
    pytester.makeconftest(CONFTEST.read_text())
    pytester.makepyfile(**modules)
    return pytester.runpytest_inprocess("-p", "no:cacheprovider")


def test_a_unit_only_session_touches_no_network_no_process_and_no_serial(pytester, exits):
    started = time.monotonic()
    result = _session(pytester, test_probe_unit=UNIT_PROBE)
    result.assert_outcomes(passed=1)
    assert exits == [], "a hardware-free session reached out: %r" % exits
    assert time.monotonic() - started < NO_RETRY_BUDGET_S, \
        "a hardware-free session waited on retries"
    result.stdout.fnmatch_lines(["*hardware-free session*"])


def test_a_unit_session_its_collection_lint_refuses_reaches_nothing(pytester, exits):
    result = _session(pytester, test_probe_unit=HALTING_PROBE)
    assert result.ret == pytest.ExitCode.USAGE_ERROR
    assert exits == [], "a refused hardware-free session reached out: %r" % exits


def test_a_session_that_never_finishes_collecting_reaches_nothing(pytester, exits):
    pytester.makeconftest(CONFTEST.read_text())
    result = pytester.runpytest_inprocess("-p", "no:cacheprovider", "missing_unit.py")
    assert result.ret == pytest.ExitCode.USAGE_ERROR
    assert exits == [], "an unclassified session reached out: %r" % exits


def test_a_session_with_one_bench_test_still_reaches_the_controller(pytester, exits,
                                                                     monkeypatch):
    monkeypatch.setattr(time, "sleep", lambda *_: None)
    result = _session(pytester, test_probe_unit=UNIT_PROBE, test_probe=BENCH_PROBE)
    result.assert_outcomes(passed=1, skipped=1)
    assert ("connect", ("127.0.0.1", 9)) in exits


class _Lens:
    def close(self):
        pass


def test_a_controller_that_is_down_skips_no_unit_test_beside_an_optical_test(pytester, exits,
                                                                            monkeypatch):
    monkeypatch.setattr(time, "sleep", lambda *_: None)
    monkeypatch.delenv("HIL_NO_CAMERA")
    monkeypatch.delenv("HIL_SKIP_CALIBRATION", raising=False)
    monkeypatch.setattr(camera_backend, "probe_and_select", lambda cfg: _Lens())
    result = _session(pytester, test_probe_unit=UNIT_PROBE, test_optics=OPTICAL_PROBE)
    result.assert_outcomes(passed=1, skipped=1)
    result.stdout.fnmatch_lines(["*STEP sees SKIP*DUT unreachable at %s*" % CLOSED])
    assert ("connect", ("127.0.0.1", 9)) in exits


def test_a_unit_test_fails_on_a_guard_refusal_and_on_a_skip_no_marker_declares(pytester,
                                                                               exits):
    result = _session(pytester, test_silent_unit=SILENT_SKIP_PROBE,
                      test_declared_unit=DECLARED_SKIP_PROBE, test_refused_unit=REFUSED_PROBE)
    result.assert_outcomes(failed=2, skipped=1)
    result.stdout.fnmatch_lines(["*no skip or skipif marker declares*no camera here*"])
    result.stdout.fnmatch_lines(["*LampNotAllowed: SA1 is outside the allowlist*"])
    assert exits == []


def test_a_unit_module_that_skips_as_it_loads_is_a_collection_error(pytester, exits):
    result = _session(pytester, test_gone_unit=MODULE_SKIP_PROBE)
    result.assert_outcomes(errors=1)
    assert exits == []


def test_the_destructive_tier_refuses_a_session_that_selects_two_of_its_tests(pytester, exits,
                                                                           monkeypatch):
    monkeypatch.setenv("HIL_ALLOW_DESTRUCTIVE", "1")
    pytester.makeconftest(CONFTEST.read_text())
    pytester.makepyfile(test_tier_unit=DESTRUCTIVE_PAIR)
    refused = pytester.runpytest_inprocess("-p", "no:cacheprovider")
    assert refused.ret == pytest.ExitCode.USAGE_ERROR
    refused.stderr.fnmatch_lines(["*one destructive test per session*STRATEGY.md §4.5*"
                                  "test_tier_unit.py::test_one*test_tier_unit.py::test_two*"])
    assert "PASSED" not in refused.stdout.str() and "STEP " not in refused.stdout.str()
    pytester.runpytest_inprocess("-p", "no:cacheprovider", "-m", "not destructive"
                                 ).assert_outcomes(passed=1, deselected=2)
    pytester.runpytest_inprocess("-p", "no:cacheprovider", "-k", "test_one"
                                 ).assert_outcomes(passed=1, deselected=2)
    assert exits == []


class _Waking:
    def __init__(self, failures):
        self.failures, self.asked = failures, 0

    def health(self):
        self.asked += 1
        if self.asked <= self.failures:
            raise ConnectionError("still booting")
        return {"status": "ok"}


def test_the_optics_probe_gives_a_booting_controller_a_few_seconds(monkeypatch):
    waits = []
    monkeypatch.setattr(hil_optics.time, "sleep", waits.append)
    waking = _Waking(failures=hil_optics.HEALTH_PROBES - 1)
    assert hil_optics.unreachable(waking) is None and waking.asked == hil_optics.HEALTH_PROBES
    down = _Waking(failures=hil_optics.HEALTH_PROBES)
    assert isinstance(hil_optics.unreachable(down), ConnectionError)
    assert waits == [hil_optics.HEALTH_PROBE_GAP_S] * 2 * (hil_optics.HEALTH_PROBES - 1)
