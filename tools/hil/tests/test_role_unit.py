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
    (flash.VIA_WB, "none", 1),
    (flash.VIA_WB, "valid", 0),
    (flash.VIA_WB, "pending_verify", 1),
    (flash.VIA_WB, "new", 1),
])
def test_the_banner_state_is_a_hard_gate(tmp_path, monkeypatch, capsys, via, state, expected):
    log = _banner_log(tmp_path, "# ready build=abcd1234 slot=ota_1 state=%s" % state)
    monkeypatch.setattr(flash.serialmon, "log_path", lambda cfg: log)
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda root: "abcd1234ffff")
    assert flash.verify_banner(HilConfig(), 0, via, tmp_path) == expected
    if state == "new":
        assert "role controller --via wb" in capsys.readouterr().err


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
    monkeypatch.setattr(role, "foreign_flashers", lambda cfg: [])
    monkeypatch.setattr(role, "_get", _health({DUT: {"role": "active"}, PEER: {"role": "active"}}))
    problems = role.preconditions(dut, peer)
    assert any("not a standby" in p for p in problems)


def test_role_refuses_while_a_session_ledger_is_open(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    (tmp_path / role.LEDGER).write_text("{}")
    monkeypatch.setattr(role, "foreign_flashers", lambda cfg: [])
    monkeypatch.setattr(role, "_get", _health({DUT: {"role": "active"}, PEER: {"role": "standby"}}))
    assert any("teardown" in p for p in role.preconditions(dut, peer))
    peer_state = peer.state_dir
    peer_state.mkdir(parents=True, exist_ok=True)
    (peer_state / role.ROLE_PIN).write_text(json.dumps({"role": role.ROLE_GEAR_SIM, "via": "ota"}))
    assert role.to_controller(dut, peer) == 2


def _pin(peer, **fields):
    peer.state_dir.mkdir(parents=True, exist_ok=True)
    (peer.state_dir / role.ROLE_PIN).write_text(json.dumps(fields))


def _silent_peer(monkeypatch, compared, banner=("abcd", "ota_1", "pending_verify")):
    monkeypatch.setattr(role, "controller_health", lambda cfg: {
        DUT: {"role": "active", "uptime_seconds": 500}}.get(cfg.base))
    monkeypatch.setattr(role, "emulator_banner", lambda cfg: banner)
    monkeypatch.setattr(role, "compare_registries",
                        lambda dut, peer, log=print: compared.append(peer) or 0)


def test_an_ota_role_returns_by_a_reset(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    _pin(peer, role=role.ROLE_GEAR_SIM, via="ota")
    sent, compared = [], []
    _silent_peer(monkeypatch, compared)
    monkeypatch.setattr(role.remote_serial, "control", lambda cfg, cmd: sent.append(cmd) or "ok")
    monkeypatch.setattr(role, "_await_controller", lambda cfg: True)
    assert role.to_controller(dut, peer) == 0
    assert sent == ["run"] and compared == [peer]
    assert role.current(peer)["role"] == role.ROLE_CONTROLLER


def test_a_wired_role_returns_by_writing_the_controller(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    _pin(peer, role=role.ROLE_GEAR_SIM, via="wb")
    runs, compared = [], []
    _silent_peer(monkeypatch, compared, banner=("abcd", "ota_0", "valid"))
    monkeypatch.setattr(role.flash, "run", lambda cfg, **kw: runs.append(kw) or 0)
    assert role.to_controller(dut, peer) == 0
    assert runs == [{"image": flash.CONTROLLER, "via": flash.VIA_WB, "allow_stale_ui": True}]


def test_a_reset_that_keeps_the_emulator_names_the_wired_return(tmp_path, monkeypatch, capsys):
    dut, peer = _cfgs(tmp_path)
    _pin(peer, role=role.ROLE_GEAR_SIM, via="ota")
    _silent_peer(monkeypatch, [])
    monkeypatch.setattr(role.remote_serial, "control", lambda cfg, cmd: "ok")
    monkeypatch.setattr(role, "_await_controller", lambda cfg: False)
    assert role.to_controller(dut, peer) == 1
    assert "--via wb" in capsys.readouterr().err
    assert role.current(peer)["role"] == role.ROLE_GEAR_SIM


def test_a_silent_peer_needs_an_explicit_way_back(tmp_path, monkeypatch, capsys):
    dut, peer = _cfgs(tmp_path)
    _pin(peer, role=role.ROLE_GEAR_SIM, via="ota")
    _silent_peer(monkeypatch, [], banner=None)
    monkeypatch.setattr(role.serialmon, "alive", lambda cfg: False)
    monkeypatch.setattr(role.remote_serial, "control", lambda cfg, cmd: pytest.fail(cmd))
    assert role.to_controller(dut, peer) == 1
    err = capsys.readouterr().err
    assert "--via ota|wb" in err and "monitor is down" in err


def _witness(monkeypatch, healths):
    replies = list(healths)
    monkeypatch.setattr(role, "controller_health",
                        lambda cfg: replies.pop(0) if len(replies) > 1 else replies[0])
    monkeypatch.setattr(role.remote_serial, "control", lambda cfg, cmd: "ok")
    monkeypatch.setattr(role.time, "sleep", lambda s: None)


UP = {"role": "active", "uptime_seconds": 500}


@pytest.mark.parametrize("healths", [
    [UP, None],
    [UP, None, {"role": "active", "uptime_seconds": 4}],
])
def test_a_reset_that_takes_the_dut_down_is_reported(tmp_path, monkeypatch, healths):
    dut, peer = _cfgs(tmp_path)
    _witness(monkeypatch, healths)
    with pytest.raises(role.RoleError, match="names the DUT"):
        role.reset_witnessed(dut, peer, window_s=0.2)


def test_a_slow_dut_that_keeps_its_uptime_is_not_accused(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    _witness(monkeypatch, [UP, None, None, {"role": "active", "uptime_seconds": 503}])
    role.reset_witnessed(dut, peer, window_s=5)


def test_a_reset_without_a_witness_is_refused(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    monkeypatch.setattr(role, "controller_health", lambda cfg: None)
    monkeypatch.setattr(role.remote_serial, "control", lambda cfg, cmd: pytest.fail(cmd))
    with pytest.raises(role.RoleError, match="does not answer"):
        role.reset_witnessed(dut, peer)


def test_the_return_refuses_a_peer_bridge_that_is_the_duts(tmp_path, monkeypatch, capsys):
    dut = HilConfig(base=DUT, state_dir=tmp_path, runs_dir=tmp_path / "runs", peer_base=PEER,
                    serial_remote="root@wb:/dev/ttyA", peer_serial_remote="root@wb:/dev/ttyA")
    assert role.to_controller(dut, dut.peer()) == 2
    assert "is the DUT's" in capsys.readouterr().err


def test_a_peer_already_back_as_a_controller_is_only_recorded_and_compared(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    _pin(peer, role=role.ROLE_GEAR_SIM, via="ota")
    compared = []
    monkeypatch.setattr(role, "controller_health", lambda cfg: {"role": "standby"})
    monkeypatch.setattr(role, "compare_registries",
                        lambda dut, peer, log=print: compared.append(peer) or 0)
    monkeypatch.setattr(role.remote_serial, "control", lambda cfg, cmd: pytest.fail(cmd))
    assert role.to_controller(dut, peer) == 0
    assert compared == [peer] and role.current(peer)["role"] == role.ROLE_CONTROLLER


@pytest.mark.parametrize("state,via", [
    ("pending_verify", flash.VIA_OTA), ("valid", flash.VIA_WB), ("new", flash.VIA_WB)])
def test_the_fresh_ready_line_decides_how_the_role_returns(state, via):
    assert role.return_via(("abcd", "ota_1", state)) == via


def test_the_banner_is_asked_for_through_the_peers_own_bridge(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    log = tmp_path / "peer.log"
    log.write_text("# ready build=old slot=ota_1 state=pending_verify\n")
    sent = []

    def control(cfg, cmd):
        sent.append((cfg.base, cmd))
        with open(log, "a") as fh:
            fh.write("# ready build=abcd slot=ota_0 state=valid\n")
        return "ok"
    monkeypatch.setattr(role.serialmon, "log_size", lambda cfg: log.stat().st_size)
    monkeypatch.setattr(role.flash.serialmon, "log_path", lambda cfg: log)
    monkeypatch.setattr(role.remote_serial, "control", control)
    assert role.emulator_banner(peer) == ("abcd", "ota_0", "valid")
    assert sent == [(PEER, "write ready")]


def test_the_role_is_recorded_before_the_banner_is_checked(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    monkeypatch.setattr(role, "preconditions", lambda dut, peer: [])

    def run(cfg, image, via, on_delivered):
        on_delivered()
        return 1
    monkeypatch.setattr(role.flash, "run", run)
    assert role.to_gear_sim(dut, peer, via=flash.VIA_WB) == 1
    held = role.current(peer)
    assert held["role"] == role.ROLE_GEAR_SIM and held["confirmed"] is False


def test_a_stale_gear_sim_record_does_not_skip_the_switch(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    _pin(peer, role=role.ROLE_GEAR_SIM, via="ota", confirmed=True)
    runs = []
    monkeypatch.setattr(role, "controller_health", lambda cfg: {"role": "standby"})
    monkeypatch.setattr(role, "emulator_banner", lambda cfg: pytest.fail("asked"))
    monkeypatch.setattr(role, "preconditions", lambda dut, peer: [])
    monkeypatch.setattr(role.flash, "run", lambda cfg, **kw: runs.append(kw["image"]) or 0)
    assert role.to_gear_sim(dut, peer) == 0
    assert runs == [flash.GEAR_SIM] and role.current(peer)["confirmed"] is True


def test_a_standby_that_keeps_stale_entries_is_reset_once(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    differences, sent = [["virtual lamps: ..."], []], []
    monkeypatch.setattr(role, "_converged", lambda dut, peer: True)
    monkeypatch.setattr(role, "registry_difference", lambda dut, peer: differences.pop(0))
    monkeypatch.setattr(role.remote_serial, "control", lambda cfg, cmd: sent.append(cmd) or "ok")
    monkeypatch.setattr(role, "controller_health",
                        lambda cfg: {"role": "active", "uptime_seconds": 500})
    monkeypatch.setattr(role, "_await_controller", lambda cfg: True)
    assert role.compare_registries(dut, peer, log=lambda line: None) == 0
    assert sent == ["run"]


def test_a_standby_that_never_converges_fails_the_return(tmp_path, monkeypatch, capsys):
    dut, peer = _cfgs(tmp_path)
    monkeypatch.setattr(role, "_converged", lambda dut, peer: False)
    assert role.compare_registries(dut, peer) == 1
    assert "did not match" in capsys.readouterr().err


def test_the_registries_are_compared_by_devices_and_bindings(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    views = {
        DUT: {"physical_devices": [{"short_address": 1}],
              "virtual_lamps": [{"virtual_lamp_id": 1, "binding": {"physical_short_address": 1}}]},
        PEER: {"physical_devices": [{"short_address": 1}, {"short_address": 16}],
               "virtual_lamps": [{"virtual_lamp_id": 1, "binding": {"physical_short_address": 1}}]},
    }
    monkeypatch.setattr(role, "_get", lambda base, path: views[base])
    assert role.registry_difference(dut, peer) == [
        "physical devices: the DUT has [1], the standby [1, 16]"]


def test_the_emulator_goes_only_onto_a_standby_peer(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    with pytest.raises(flash.BoardError):
        flash.require_standby_peer(dut)
    monkeypatch.setattr(flash, "health", lambda base: {"role": "active"})
    with pytest.raises(flash.BoardError):
        flash.require_standby_peer(peer)
    monkeypatch.setattr(flash, "health", lambda base: {"role": "standby"})
    flash.require_standby_peer(peer)


def test_a_board_proof_needs_a_witness(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)
    monkeypatch.setattr(flash, "health", lambda base: None)
    with pytest.raises(flash.BoardError):
        flash.board_proof(peer)


def test_the_ready_line_is_asked_for_until_a_late_monitor_records_it(tmp_path, monkeypatch):
    log = tmp_path / "serial.log"
    log.write_text("")
    cfg = HilConfig(serial_remote="root@wb:/dev/ttyX")
    monkeypatch.setattr(flash.serialmon, "log_path", lambda c: log)
    monkeypatch.setattr(flash, "BANNER_BOOT_S", 0.1)
    monkeypatch.setattr(flash, "READY_ASK_INTERVAL_S", 0.2)
    monkeypatch.setattr(flash, "POLL_S", 0.02)
    asks = []

    def control(c, cmd):
        asks.append(cmd)
        if len(asks) == 3:
            log.write_text("# ready build=abcd slot=ota_0 state=valid\n")
        return "ok wrote"
    monkeypatch.setattr(flash.remote_serial, "control", control)
    assert flash.wait_banner(cfg, 0, timeout_s=3) == ("abcd", "ota_0", "valid")
    assert asks == ["write ready"] * 3


def test_a_missed_boot_line_is_asked_for_again(tmp_path, monkeypatch):
    log = tmp_path / "serial.log"
    log.write_text("")
    cfg = HilConfig(serial_remote="root@wb:/dev/ttyX")
    monkeypatch.setattr(flash.serialmon, "log_path", lambda c: log)
    monkeypatch.setattr(flash, "BANNER_BOOT_S", 0.3)
    monkeypatch.setattr(flash, "POLL_S", 0.05)

    def control(c, cmd):
        assert cmd == "write ready"
        log.write_text("# ready build=abcd slot=ota_1 state=none\n")
        return "ok wrote"
    monkeypatch.setattr(flash.remote_serial, "control", control)
    assert flash.wait_banner(cfg, 0, timeout_s=2) == ("abcd", "ota_1", "none")


def test_a_slice_the_dut_lacks_does_not_block_convergence():
    dut = {"groups_a0": (40, 7), "rules_b1": (None, 0)}
    assert role.slices_match(dut, {"groups_a0": (40, 7), "rules_b1": (12, 99)})
    assert not role.slices_match(dut, {"groups_a0": (40, 8)})
    assert not role.slices_match({}, {})


def test_a_wb_that_does_not_answer_counts_as_a_flasher(tmp_path, monkeypatch):
    dut, peer = _cfgs(tmp_path)

    def run(cmd, **kwargs):
        if cmd[0] == "ssh":
            raise role.subprocess.TimeoutExpired(cmd, 20)
        return role.subprocess.CompletedProcess(cmd, 1, stdout="", stderr="")
    monkeypatch.setattr(role.subprocess, "run", run)
    assert role.foreign_flashers(peer) == ["%s did not say which flashers run" % peer.wb_ssh]
