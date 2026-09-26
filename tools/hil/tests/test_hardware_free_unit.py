import errno
import socket
import subprocess
import time
from pathlib import Path

import pytest
import serial

pytest_plugins = ["pytester"]

CONFTEST = Path(__file__).resolve().parent / "conftest.py"
CLOSED = "http://127.0.0.1:9"
NO_RETRY_BUDGET_S = 10.0
UNIT_PROBE = "def test_nothing():\n    assert True\n"
BENCH_PROBE = "def test_reaches(api):\n    assert api\n"
HALTING_PROBE = ("from hil import remote_serial\n\n\n"
                 "def test_halts(tmp_path):\n    remote_serial.control(None, 'run')\n")


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
