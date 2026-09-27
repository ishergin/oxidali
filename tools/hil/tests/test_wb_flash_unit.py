import subprocess

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
    def __init__(self, rcs):
        self.rcs, self.commands = list(rcs), []

    def __call__(self, tgt, command, timeout=60, **kwargs):
        self.commands.append(command)
        if command.startswith("cat "):
            return subprocess.CompletedProcess(command, 0, stdout=self.rcs.pop(0), stderr="")
        return subprocess.CompletedProcess(command, 0, stdout="esptool said no\n", stderr="")


def test_a_detached_write_is_polled_until_its_exit_code(monkeypatch):
    remote = _Remote(["", "", "0\n"])
    monkeypatch.setattr(wb_flash, "_ssh", remote)
    monkeypatch.setattr(wb_flash.time, "sleep", lambda s: None)
    assert wb_flash.run_detached(_Target(), "python3 -m esptool x") == 0
    assert "nohup setsid sh -c" in remote.commands[0]
    assert remote.commands[0].count(wb_flash.WRITE_RC) == 2


def test_a_failed_detached_write_shows_its_log(monkeypatch):
    remote, lines = _Remote(["2\n"]), []
    monkeypatch.setattr(wb_flash, "_ssh", remote)
    monkeypatch.setattr(wb_flash.time, "sleep", lambda s: None)
    assert wb_flash.run_detached(_Target(), "python3 -m esptool x", log=lines.append) == 2
    assert lines == ["esptool said no\n"]


def test_a_write_that_never_finishes_is_an_error(monkeypatch):
    monkeypatch.setattr(wb_flash, "_ssh", _Remote([""] * 10))
    monkeypatch.setattr(wb_flash.time, "sleep", lambda s: None)
    with pytest.raises(wb_flash.WbFlashError, match="did not finish"):
        wb_flash.run_detached(_Target(), "x", timeout_s=0.0)
