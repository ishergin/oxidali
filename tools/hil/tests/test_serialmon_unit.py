import datetime
import time

import pytest

from hil import serialmon
from hil.config import HilConfig

C6 = "/dev/cu.usbmodemC6SIM1"
P4 = "/dev/cu.usbmodemP4DUT1"


def _cfg(tmp_path, port, pinned):
    return HilConfig(state_dir=tmp_path, serial_port=port,
                     serial_port_pinned=pinned, serial_remote="")


@pytest.fixture()
def spawned(monkeypatch):
    calls = []

    class _Proc:
        pid = 4242

    monkeypatch.setattr(serialmon, "pid_alive", lambda pidfile: False)
    monkeypatch.setattr(serialmon.subprocess, "Popen",
                        lambda argv, **kw: calls.append(argv) or _Proc())
    return calls


@pytest.fixture()
def attached(monkeypatch):
    real = serialmon.os.path.exists

    def _attach(*ports):
        monkeypatch.setattr(serialmon.os.path, "exists",
                            lambda p: str(p) in ports or real(p))
    return _attach


def _mode_and_port(argv):
    return argv[-1], argv[3]


def test_a_pinned_start_records_the_pin(tmp_path, spawned, attached):
    attached(C6, P4)
    assert serialmon.start(_cfg(tmp_path, P4, pinned=True)) == 0
    assert _mode_and_port(spawned[0]) == ("pinned", P4)
    assert serialmon.recorded_pin(_cfg(tmp_path, C6, pinned=False)) == P4


def test_a_restart_from_a_pinless_process_inherits_the_pin(
        tmp_path, spawned, attached):
    attached(C6, P4)
    serialmon.start(_cfg(tmp_path, P4, pinned=True))
    assert serialmon.start(_cfg(tmp_path, C6, pinned=False)) == 0
    assert _mode_and_port(spawned[1]) == ("pinned", P4)


def test_stop_keeps_the_pin_for_the_next_start(tmp_path, spawned, attached,
                                               monkeypatch, capsys):
    attached(C6, P4)
    cfg = _cfg(tmp_path, P4, pinned=True)
    serialmon.start(cfg)
    monkeypatch.setattr(serialmon, "pid_alive", lambda pidfile: True)
    monkeypatch.setattr(serialmon.os, "getpgid", lambda pid: pid)
    killed = []
    monkeypatch.setattr(serialmon.os, "killpg",
                        lambda pgid, sig: killed.append((pgid, sig)))
    assert serialmon.stop(cfg) == 0
    assert killed, "stop() no longer signals the daemon"
    assert not (tmp_path / "serial_monitor.pid").exists()
    assert serialmon.recorded_pin(cfg) == P4
    capsys.readouterr()


def test_a_new_explicit_pin_replaces_the_recorded_one(tmp_path, spawned, attached):
    attached(C6, P4)
    serialmon.start(_cfg(tmp_path, C6, pinned=True))
    serialmon.start(_cfg(tmp_path, P4, pinned=True))
    assert _mode_and_port(spawned[1]) == ("pinned", P4)
    assert serialmon.recorded_pin(_cfg(tmp_path, C6, pinned=False)) == P4


def test_no_pin_anywhere_stays_auto(tmp_path, spawned, attached):
    attached(P4)
    assert serialmon.start(_cfg(tmp_path, P4, pinned=False)) == 0
    assert _mode_and_port(spawned[0]) == ("auto", P4)
    assert serialmon.recorded_pin(_cfg(tmp_path, P4, pinned=False)) is None


def test_a_recorded_pin_naming_a_gone_port_fails_loudly(
        tmp_path, spawned, attached, capsys):
    serialmon.record_pin(_cfg(tmp_path, P4, pinned=False), P4)
    attached(C6)
    assert serialmon.start(_cfg(tmp_path, C6, pinned=False)) == 1
    assert spawned == []
    err = capsys.readouterr().err
    assert "serial_port.pin" in err
    assert "HIL_SERIAL_PORT" in err


MOSCOW = "MSK-3"
RUN_DIR_STAMP = "%Y%m%d-%H%M%S"


@pytest.fixture()
def local_time_is_not_utc(monkeypatch):
    monkeypatch.setenv("TZ", MOSCOW)
    time.tzset()
    yield
    monkeypatch.undo()
    time.tzset()


def test_serial_lines_are_stamped_in_utc_like_the_run_directories(local_time_is_not_utc):
    assert serialmon.stamp(0.5) == "1970-01-01T00:00:00.500Z"
    now = time.time()
    stamped = datetime.datetime.strptime(serialmon.stamp(now), "%Y-%m-%dT%H:%M:%S.%fZ")
    run_dir = datetime.datetime.strptime(time.strftime(RUN_DIR_STAMP, time.gmtime(now)),
                                         RUN_DIR_STAMP)
    assert stamped.replace(microsecond=0) == run_dir
    assert time.localtime(now).tm_hour != time.gmtime(now).tm_hour, "the premise: TZ moved"
