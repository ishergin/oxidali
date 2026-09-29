import subprocess
import sys

import pytest

from hil import remote_serial, wb_flash
from hil.config import HilConfig

NEW_BRIDGE = "ok port=/dev/ttyX baud=115200 client=none uptime=5 verbs=release,reacquire,write"
OLD_BRIDGE = "ok port=/dev/ttyX baud=115200 client=none uptime=5"


def _cfg():
    return HilConfig(serial_remote="root@wb:/dev/ttyX")


def _control(monkeypatch, replies=None, failing=()):
    sent = []

    def control(cfg, cmd):
        sent.append(cmd)
        if cmd in failing:
            raise remote_serial.RemoteError("bridge refused %r" % cmd)
        return (replies or {}).get(cmd, "ok")
    monkeypatch.setattr(wb_flash.remote_serial, "control", control)
    return sent


def test_the_tools_are_found_as_packages_when_a_script_shadows_them(tmp_path, monkeypatch):
    (tmp_path / "esptool.py").write_text("")
    monkeypatch.syspath_prepend(str(tmp_path))
    monkeypatch.delitem(sys.modules, "esptool", raising=False)
    site = wb_flash._site_packages()
    for package in wb_flash.TOOL_PACKAGES:
        assert (site / package / "__init__.py").is_file(), (site, package)


def test_a_bridge_that_cannot_release_is_refused_before_any_reset(monkeypatch):
    sent = _control(monkeypatch, {"status": OLD_BRIDGE})
    with pytest.raises(wb_flash.WbFlashError, match="--restart"):
        wb_flash.require_release(_cfg())
    assert sent == ["status"]


def test_a_released_port_is_named_before_any_reset(monkeypatch):
    _control(monkeypatch, {"status": "ok port=/dev/ttyX released"})
    with pytest.raises(wb_flash.WbFlashError, match="reacquire"):
        wb_flash.require_release(_cfg())


def test_a_current_bridge_passes(monkeypatch):
    _control(monkeypatch, {"status": NEW_BRIDGE})
    wb_flash.require_release(_cfg())


@pytest.mark.parametrize("failing,proof_fails,expected", [
    ((), False, ["bootloader", "release"]),
    (("release",), False, ["bootloader", "release", "run"]),
    ((), True, ["bootloader", "run"]),
])
def test_any_failure_before_the_write_puts_the_board_back_into_its_application(
        monkeypatch, failing, proof_fails, expected):
    sent = _control(monkeypatch, failing=failing)

    def proof():
        if proof_fails:
            raise RuntimeError("wrong board")
    if failing or proof_fails:
        with pytest.raises(Exception):
            wb_flash.enter_loader(_cfg(), proof)
    else:
        wb_flash.enter_loader(_cfg(), proof)
    assert sent == expected


class _Target:
    ssh = "root@wb"


class _Remote:
    def __init__(self, rcs, start_rc=0):
        self.rcs, self.commands, self.start_rc = list(rcs), [], start_rc

    def __call__(self, tgt, command, timeout=60, **kwargs):
        self.commands.append(command)
        if command.startswith("cat "):
            return subprocess.CompletedProcess(command, 0, stdout=self.rcs.pop(0), stderr="")
        if command.startswith("nohup "):
            return subprocess.CompletedProcess(command, self.start_rc, stdout="", stderr="")
        return subprocess.CompletedProcess(command, 0, stdout="esptool said no\n", stderr="")


def _detached(monkeypatch, remote):
    monkeypatch.setattr(wb_flash, "_ssh", remote)
    monkeypatch.setattr(wb_flash.time, "sleep", lambda s: None)
    return remote


def test_a_detached_write_is_polled_until_its_exit_code(monkeypatch):
    remote = _detached(monkeypatch, _Remote(["", "", "0\n"]))
    assert wb_flash.run_detached(_Target(), "python3 -m esptool x") == 0
    start, *polls, cleanup = remote.commands
    rc_file = start.split("echo $? > ")[1].split("'")[0]
    assert start.startswith("nohup setsid sh -c") and rc_file.endswith(".rc")
    assert all(rc_file in poll for poll in polls) and rc_file in cleanup


def test_every_write_gets_its_own_exit_code_file(monkeypatch):
    remote = _detached(monkeypatch, _Remote(["0\n", "0\n"]))
    wb_flash.run_detached(_Target(), "x")
    wb_flash.run_detached(_Target(), "x")
    starts = [c for c in remote.commands if c.startswith("nohup")]
    assert starts[0] != starts[1]


def test_a_start_that_fails_is_not_read_as_a_finished_write(monkeypatch):
    remote = _detached(monkeypatch, _Remote(["0\n"], start_rc=127))
    with pytest.raises(wb_flash.WbFlashError, match="did not start"):
        wb_flash.run_detached(_Target(), "x")
    assert not any(c.startswith("cat ") for c in remote.commands)


def test_a_dropped_start_ssh_is_polled_like_a_hung_one(monkeypatch):
    remote = _detached(monkeypatch, _Remote(["", "0\n"], start_rc=wb_flash.SSH_UNREACHABLE))
    assert wb_flash.run_detached(_Target(), "x", log=lambda line: None) == 0
    assert sum(c.startswith("cat ") for c in remote.commands) == 2


class _Port:
    def __init__(self, busy_polls):
        self.busy_polls, self.commands = busy_polls, []

    def __call__(self, tgt, command, timeout=60, **kwargs):
        self.commands.append(command)
        busy = self.busy_polls > 0
        self.busy_polls -= 1
        return subprocess.CompletedProcess(command, 0 if busy else 1, stdout="", stderr="")


class _PortTarget(_Target):
    device = "/dev/ttyX"


def test_the_port_is_reacquired_only_once_esptool_lets_go(monkeypatch):
    sent = _control(monkeypatch)
    monkeypatch.setattr(wb_flash, "_ssh", _Port(busy_polls=2))
    monkeypatch.setattr(wb_flash.time, "sleep", lambda s: None)
    wb_flash.reacquire_when_free(_cfg(), _PortTarget())
    assert sent == ["reacquire"]


def test_a_port_still_held_is_left_released(monkeypatch):
    sent = _control(monkeypatch)
    monkeypatch.setattr(wb_flash, "_ssh", _Port(busy_polls=1000))
    monkeypatch.setattr(wb_flash.time, "sleep", lambda s: None)
    with pytest.raises(wb_flash.WbFlashError, match="stays released"):
        wb_flash.reacquire_when_free(_cfg(), _PortTarget(), timeout_s=0.0)
    assert sent == []


def test_a_failed_detached_write_shows_its_log(monkeypatch):
    lines = []
    _detached(monkeypatch, _Remote(["2\n"]))
    assert wb_flash.run_detached(_Target(), "python3 -m esptool x", log=lines.append) == 2
    assert lines == ["esptool said no\n"]


def test_a_write_that_never_finishes_is_an_error(monkeypatch):
    _detached(monkeypatch, _Remote([""] * 10))
    with pytest.raises(wb_flash.WbFlashError, match="did not finish"):
        wb_flash.run_detached(_Target(), "x", timeout_s=0.0)
