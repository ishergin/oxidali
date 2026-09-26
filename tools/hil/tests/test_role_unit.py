import json

import pytest

from hil import flash, role, wb_flash
from hil.config import HilConfig

DUT = "http://10.0.0.1"
PEER = "http://10.0.0.2"


def _cfgs(tmp_path):
    dut = HilConfig(base=DUT, state_dir=tmp_path, runs_dir=tmp_path / "runs",
                    peer_base=PEER, serial_remote="")
    return dut, dut.peer()


def _health(roles):
    def get(base, path):
        return roles.get(base)
    return get


def test_the_gear_emulator_image_builds_in_its_own_workspace():
    spec = flash.board_spec(HilConfig(board="esp32p4"), flash.GEAR_SIM)
    assert spec["workspace"] == "tools/dali-gear-sim"
    assert spec["firmware_bin"] == \
        "tools/dali-gear-sim/target/riscv32imafc-esp-espidf/debug/dali-gear-sim"
    assert spec["bootloader"] == "target/riscv32imafc-esp-espidf/debug/bootloader.bin"
    assert spec["cargo_args"] == ["build"] and not spec["bench"]
    assert spec["board_env"]["MCU"] == "esp32p4"


def test_the_controller_image_keeps_its_paths():
    spec = flash.board_spec(HilConfig(board="esp32p4"))
    assert spec["firmware_bin"] == "target/riscv32imafc-esp-espidf/debug/dali2rust"
    assert spec["merged_bin"] == "target/riscv32imafc-esp-espidf/debug/dali2rust-merged.bin"
    assert spec["cargo_args"] == ["fw"] and spec["bench"]


def test_an_unknown_image_is_refused():
    with pytest.raises(flash.BoardError):
        flash.board_spec(HilConfig(board="esp32p4"), "firmware")


def _banner_log(tmp_path, line):
    log = tmp_path / "serial.log"
    log.write_text("2026-09-26T10:00:00.000Z # dali-gear-sim build=abcd1234 (esp32p4)\n"
                   "2026-09-26T10:00:01.000Z %s\n" % line)
    return log


def test_the_ready_line_is_read_from_the_monitor_log(tmp_path, monkeypatch):
    log = _banner_log(tmp_path, "# ready build=abcd1234 slot=ota_1 state=pending_verify")
    monkeypatch.setattr(flash.serialmon, "log_path", lambda cfg: log)
    assert flash.ready_line(HilConfig(), 0) == ("abcd1234", "ota_1", "pending_verify")


@pytest.mark.parametrize("via,state,expected", [
    (flash.VIA_OTA, "pending_verify", 0),
    (flash.VIA_OTA, "new", 1),
    (flash.VIA_OTA, "valid", 1),
    (flash.VIA_WB, "none", 0),
    (flash.VIA_WB, "pending_verify", 1),
])
def test_the_banner_state_is_a_hard_gate(tmp_path, monkeypatch, capsys, via, state, expected):
    log = _banner_log(tmp_path, "# ready build=abcd1234 slot=ota_1 state=%s" % state)
    monkeypatch.setattr(flash.serialmon, "log_path", lambda cfg: log)
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda root: "abcd1234ffff")
    assert flash.verify_banner(HilConfig(), 0, via, tmp_path) == expected
    if state == "new":
        assert "recover by wire" in capsys.readouterr().err


def test_a_banner_from_another_commit_fails(tmp_path, monkeypatch, capsys):
    log = _banner_log(tmp_path, "# ready build=1111aaaa slot=ota_1 state=pending_verify")
    monkeypatch.setattr(flash.serialmon, "log_path", lambda cfg: log)
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda root: "abcd1234ffff")
    assert flash.verify_banner(HilConfig(), 0, flash.VIA_OTA, tmp_path) == 1
    assert "not the commit just built" in capsys.readouterr().err


class _Resp:
    def __init__(self, body, ok=True):
        self.body, self.ok = body, ok

    def json(self):
        return self.body


def test_an_update_that_reaches_ready_to_reboot_succeeds(monkeypatch):
    states = iter(["downloading", "finishing", "ready_to_reboot"])
    monkeypatch.setattr(flash.requests, "get",
                        lambda url, timeout: _Resp({"update": {"state": next(states)}}))
    monkeypatch.setattr(flash.time, "sleep", lambda s: None)
    assert flash.await_update(PEER, timeout_s=5) == 0


def test_a_board_that_goes_quiet_after_finishing_has_rebooted(monkeypatch):
    replies = iter([_Resp({"update": {"state": "finishing"}})])

    def get(url, timeout):
        try:
            return next(replies)
        except StopIteration:
            raise flash.requests.ConnectionError("rebooting")
    monkeypatch.setattr(flash.requests, "get", get)
    monkeypatch.setattr(flash.time, "sleep", lambda s: None)
    assert flash.await_update(PEER, timeout_s=5) == 0


def test_a_failed_update_is_reported(monkeypatch, capsys):
    monkeypatch.setattr(flash.requests, "get", lambda url, timeout: _Resp(
        {"update": {"state": "failed", "error": "fetch_failed"}}))
    assert flash.await_update(PEER, timeout_s=5) == 1
    assert "fetch_failed" in capsys.readouterr().err


def test_the_wb_write_command_keeps_the_board_in_its_bootloader():
    cmd = wb_flash.write_command("/dev/serial/by-id/usb-1a86_X-if00", "/mnt/x/i.bin")
    assert "--before no_reset --after no_reset" in cmd
    assert "write_flash 0x0 /mnt/x/i.bin" in cmd
    assert cmd.startswith("PYTHONPATH=%s python3 -m esptool --chip esp32p4" % wb_flash.REMOTE_TOOLS)


def test_the_tools_digest_follows_the_sources(tmp_path):
    for package in wb_flash.TOOL_PACKAGES:
        (tmp_path / package).mkdir()
        (tmp_path / package / "__init__.py").write_text("x = 1\n")
    first = wb_flash.tools_digest(tmp_path)
    (tmp_path / "esptool" / "__init__.py").write_text("x = 2\n")
    assert wb_flash.tools_digest(tmp_path) != first


def test_a_board_whose_http_stays_up_in_its_bootloader_is_the_wrong_board(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    monkeypatch.setattr(flash, "health", lambda base: {"uptime_seconds": 100})
    monkeypatch.setattr(flash, "_goes_quiet", lambda base, timeout_s=0: False)
    with pytest.raises(flash.BoardError):
        flash.board_proof(peer)()


def test_the_other_board_restarting_fails_the_proof(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    uptimes = {PEER: [{"uptime_seconds": 50}], DUT: [{"uptime_seconds": 500}, {"uptime_seconds": 3}]}
    monkeypatch.setattr(flash, "health", lambda base: uptimes[base].pop(0) if uptimes[base] else None)
    monkeypatch.setattr(flash, "_goes_quiet", lambda base, timeout_s=0: True)
    check = flash.board_proof(peer)
    with pytest.raises(flash.BoardError):
        check()


def test_role_refuses_a_peer_that_is_not_a_standby(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    monkeypatch.setattr(role, "_get", _health({DUT: {"role": "active"}, PEER: {"role": "active"}}))
    problems = role.preconditions(dut, peer)
    assert any("not a standby" in p for p in problems)


def test_role_refuses_while_a_session_ledger_is_open(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    (tmp_path / role.LEDGER).write_text("{}")
    monkeypatch.setattr(role, "_get", _health({DUT: {"role": "active"}, PEER: {"role": "standby"}}))
    assert any("teardown" in p for p in role.preconditions(dut, peer))
    peer_state = peer.state_dir
    peer_state.mkdir(parents=True, exist_ok=True)
    (peer_state / role.ROLE_PIN).write_text(json.dumps({"role": role.ROLE_GEAR_SIM, "via": "ota"}))
    assert role.to_controller(dut, peer) == 2


def test_an_ota_role_returns_by_a_console_reboot(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    peer.state_dir.mkdir(parents=True, exist_ok=True)
    (peer.state_dir / role.ROLE_PIN).write_text(json.dumps({"role": role.ROLE_GEAR_SIM, "via": "ota"}))
    sent = []
    monkeypatch.setattr(role.remote_serial, "control", lambda cfg, cmd: sent.append(cmd) or "ok")
    monkeypatch.setattr(role, "_await_controller", lambda cfg: True)
    assert role.to_controller(dut, peer) == 0
    assert sent == ["write reboot"]
    assert role.current(peer)["role"] == role.ROLE_CONTROLLER


def test_a_wired_role_returns_by_writing_the_controller(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    peer.state_dir.mkdir(parents=True, exist_ok=True)
    (peer.state_dir / role.ROLE_PIN).write_text(json.dumps({"role": role.ROLE_GEAR_SIM, "via": "wb"}))
    runs = []
    monkeypatch.setattr(role.flash, "run", lambda cfg, **kw: runs.append(kw) or 0)
    assert role.to_controller(dut, peer) == 0
    assert runs == [{"image": flash.CONTROLLER, "via": flash.VIA_WB}]
