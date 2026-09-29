import json
import threading

import pytest

from hil import gearsim, role
from hil.config import HilConfig
from hil.seriallog import LogWindow

STAMP = "2026-09-26T10:00:00.000Z "


def test_a_level_change_is_parsed_with_its_gear():
    change = gearsim.parse_change(STAMP + "C 123456 A16 level 0 -> 180")
    assert change == gearsim.Change(123456, "A16", "level", "0", "180")
    assert change.short == 16


@pytest.mark.parametrize("line,field,old,new", [
    ("C 1 A20 scene[3] mirek unset -> 250", "scene[3] mirek", "unset", "250"),
    ("C 1 A20 rgbwaf 0,0,0,0,0,0 -> 254,0,0,0,0,0", "rgbwaf", "0,0,0,0,0,0", "254,0,0,0,0,0"),
    ("C 1 U03 short_address - -> 40", "short_address", "-", "40"),
    ("C 1 A21 groups 0x0000 -> 0x0010", "groups", "0x0000", "0x0010"),
])
def test_every_change_line_shape_parses(line, field, old, new):
    change = gearsim.parse_change(STAMP + line)
    assert (change.field, change.old, change.new) == (field, old, new)


def test_an_unaddressed_gear_has_no_short():
    assert gearsim.parse_change("C 1 U03 level 0 -> 5").short is None


@pytest.mark.parametrize("line", ["# ready build=x slot=ota_1 state=none",
                                  "F 1 fe 90 Broadcast QUERY STATUS", "C 1 A16 level",
                                  "C x A16 level 0 -> 1", ""])
def test_other_lines_are_not_changes(line):
    assert gearsim.parse_change(STAMP + line) is None


def _peer(tmp_path, monkeypatch, role_name="gear-sim"):
    cfg = HilConfig(base="http://10.0.0.1", state_dir=tmp_path, runs_dir=tmp_path / "r",
                    peer_base="http://10.0.0.2", serial_remote="").peer()
    cfg.state_dir.mkdir(parents=True, exist_ok=True)
    (cfg.state_dir / role.ROLE_PIN).write_text(json.dumps({"role": role_name, "via": "ota"}))
    log = tmp_path / "serial.log"
    log.write_text("")
    monkeypatch.setattr(gearsim.serialmon, "alive", lambda c: True)
    monkeypatch.setattr(gearsim.serialmon, "log_path", lambda c: log)
    monkeypatch.setattr(gearsim.role, "controller_health", lambda c: None)
    return cfg, log


def test_the_console_needs_the_emulator_role(tmp_path, monkeypatch):
    cfg, _ = _peer(tmp_path, monkeypatch, role_name="controller")
    with pytest.raises(gearsim.GearSimUnavailable):
        gearsim.GearSim(cfg)


def test_a_command_goes_through_the_bridge_and_its_reply_through_the_log(tmp_path, monkeypatch):
    cfg, log = _peer(tmp_path, monkeypatch)
    sent = []

    def control(peer, command):
        sent.append(command)
        with open(log, "a") as fh:
            fh.write(STAMP + "C 5 A16 level 0 -> 1\n")
            fh.write(STAMP + "# reserved 0x000000000000ffff (16 addresses)\n")
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    monkeypatch.setattr(gearsim, "REPLY_QUIET_S", 0.0)
    sim = gearsim.GearSim(cfg)
    assert sim.reserve(range(16)) == "reserved 0x000000000000ffff (16 addresses)"
    assert sent == ["write reserve 0-15"]


def test_a_reserve_the_emulator_echoes_differently_is_refused(tmp_path, monkeypatch):
    cfg, log = _peer(tmp_path, monkeypatch)

    def control(peer, command):
        with open(log, "a") as fh:
            fh.write(STAMP + "# reserved 0x000000000000000f (4 addresses)\n")
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    monkeypatch.setattr(gearsim, "REPLY_QUIET_S", 0.0)
    with pytest.raises(gearsim.GearSimUnavailable):
        gearsim.GearSim(cfg).reserve(range(16))


def test_a_peer_that_answers_as_a_controller_has_no_emulator_console(tmp_path, monkeypatch):
    cfg, _ = _peer(tmp_path, monkeypatch)
    monkeypatch.setattr(gearsim.role, "controller_health", lambda c: {"role": "standby"})
    with pytest.raises(gearsim.GearSimUnavailable):
        gearsim.GearSim(cfg)


def test_a_refusal_is_raised_not_swallowed(tmp_path, monkeypatch):
    cfg, log = _peer(tmp_path, monkeypatch)

    def control(peer, command):
        with open(log, "a") as fh:
            fh.write(STAMP + "# refused: `reserve` first\n")
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    monkeypatch.setattr(gearsim, "REPLY_QUIET_S", 0.0)
    with pytest.raises(gearsim.GearSimUnavailable, match="refused"):
        gearsim.GearSim(cfg).enable("all")


def test_the_show_table_is_parsed(tmp_path, monkeypatch):
    cfg, log = _peer(tmp_path, monkeypatch)

    def control(peer, command):
        with open(log, "a") as fh:
            fh.write(STAMP + "# idx addr random   type lvl groups on  drop\n")
            fh.write(STAMP + "#   0   16 a1b2c3  DT6 254 0x0010 yes 0\n")
            fh.write(STAMP + "#   1    - 00beef  DT8   0 0x0000 no 0\n")
            fh.write(STAMP + "# 2 gear listed\n")
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    monkeypatch.setattr(gearsim, "REPLY_QUIET_S", 0.0)
    rows = gearsim.GearSim(cfg).show()
    assert rows == [{"short": 16, "random": 0xA1B2C3, "dt8": False, "level": 254,
                     "groups": 0x10, "enabled": True}]


def test_the_oracle_finds_the_change_it_waits_for(tmp_path, monkeypatch):
    cfg, log = _peer(tmp_path, monkeypatch)
    monkeypatch.setattr(gearsim, "UNTOUCHED_QUIET_S", 0.05)
    oracle = gearsim.GearOracle(gearsim.GearSim(cfg))
    with LogWindow(log) as window:
        with open(log, "a") as fh:
            fh.write(STAMP + "C 7 A17 level 0 -> 200\n")
            fh.write(STAMP + "C 8 A16 level 0 -> 180\n")
        assert oracle.expect(window, 16, "level", 180, timeout_s=0.5).at_us == 8
        with pytest.raises(AssertionError, match="SA18"):
            oracle.expect(window, 18, "level", 1, timeout_s=0.2)
        with pytest.raises(AssertionError, match="must stay still"):
            oracle.untouched(window, [17])


def _reserve_reply(tmp_path, monkeypatch, reply):
    cfg, log = _peer(tmp_path, monkeypatch)

    def control(peer, command):
        with open(log, "a") as fh:
            fh.write(STAMP + "# %s\n" % reply)
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    monkeypatch.setattr(gearsim, "REPLY_QUIET_S", 0.0)
    return gearsim.GearSim(cfg)


def test_a_second_session_on_one_boot_keeps_an_equal_reserve(tmp_path, monkeypatch):
    sim = _reserve_reply(tmp_path, monkeypatch, "refused: reserved already 0x000000000000ffff "
                         "for this boot; reboot to change it")
    assert sim.reserve(range(16)).startswith("refused: reserved already")


def test_a_second_session_with_another_reserve_is_refused(tmp_path, monkeypatch):
    sim = _reserve_reply(tmp_path, monkeypatch, "refused: reserved already 0x000000000000000f "
                         "for this boot; reboot to change it")
    with pytest.raises(gearsim.GearSimUnavailable, match="one per boot"):
        sim.reserve(range(16))


def test_untouched_waits_for_the_emulator_to_fall_quiet(tmp_path, monkeypatch):
    cfg, log = _peer(tmp_path, monkeypatch)
    monkeypatch.setattr(gearsim, "UNTOUCHED_QUIET_S", 0.3)
    monkeypatch.setattr(gearsim, "POLL_S", 0.02)
    oracle = gearsim.GearOracle(gearsim.GearSim(cfg))

    def late_line():
        with open(log, "a") as fh:
            fh.write(STAMP + "C 9 A17 level 0 -> 5\n")
    with LogWindow(log) as window:
        timer = threading.Timer(0.1, late_line)
        timer.start()
        try:
            with pytest.raises(AssertionError, match="must stay still"):
                oracle.untouched(window, [17])
        finally:
            timer.join()


def test_a_heard_frame_carries_its_bytes():
    heard = gearsim.parse_heard(STAMP + "F 4321 29 e2 Short(20) ACTIVATE")
    assert heard == gearsim.Heard(4321, 0x29, 0xE2)
    lines = [STAMP + "F 1 fe 90 Broadcast QUERY STATUS", STAMP + "C 2 A16 level 0 -> 5",
             STAMP + "B 3 84 n=1", STAMP + "# log level frame", "F x 29 e2", "F 5 zz 90"]
    assert gearsim.heard_frames(lines) == [gearsim.Heard(1, 0xFE, 0x90)]


def test_addresses_follow_the_frame_layout():
    assert gearsim.command_address(20) == 0x29
    assert gearsim.group_dapc_address(4) == 0x88


def _frames(*pairs):
    return [gearsim.Heard(at, address, data) for at, (address, data) in enumerate(pairs)]


ENABLE_DT8 = (0xC1, 0x08)
STAGE = (0x29, 0xE7)
ACTIVATE = (0x29, 0xE2)
STATUS = (0x29, 0x90)
COLOUR_STATUS = (0x29, 0xF8)
DTRS = ((0xA3, 0x4D), (0xC3, 0x01))


def _write(*between):
    return list(DTRS) + [ENABLE_DT8, STAGE, ENABLE_DT8, COLOUR_STATUS] + list(between) + [
        ENABLE_DT8, ACTIVATE]


def test_a_colour_write_the_fix_sends_is_staged_and_activated():
    tally = gearsim.colour_writes(_frames(*(_write() + _write())), 20)
    assert (tally.stagings, tally.activations, tally.faults) == (2, 2, [])


def test_a_status_gate_between_staging_and_activation_is_a_fault():
    tally = gearsim.colour_writes(_frames(*_write(STATUS)), 20)
    assert tally.activations == 1
    assert any("QUERY STATUS" in fault for fault in tally.faults)


def test_a_status_query_to_another_gear_is_not_a_gate():
    tally = gearsim.colour_writes(_frames(*_write((0x03, 0x90))), 20)
    assert tally.faults == []


def test_a_staged_colour_never_activated_is_a_fault():
    tally = gearsim.colour_writes(_frames(*list(DTRS), ENABLE_DT8, STAGE, STATUS), 20)
    assert tally.activations == 0
    assert any("never activated" in fault for fault in tally.faults)


def test_an_activate_the_enable_did_not_open_is_a_fault():
    tally = gearsim.colour_writes(_frames(ENABLE_DT8, STAGE, (0x29, 0x98), ACTIVATE), 20)
    assert any("did not follow ENABLE DEVICE TYPE 8" in fault for fault in tally.faults)


def test_a_restaged_colour_activated_once_is_one_unit():
    frames = _frames(ENABLE_DT8, STAGE, ENABLE_DT8, STAGE, ENABLE_DT8, ACTIVATE)
    tally = gearsim.colour_writes(frames, 20)
    assert (tally.stagings, tally.activations, tally.faults) == (2, 1, [])


GROUP_MASK_FRAME = (0x88, 0xFF)


@pytest.mark.parametrize("pairs,faults", [
    ([GROUP_MASK_FRAME], 0),
    ([GROUP_MASK_FRAME, (0x89, 0xA0), (0x24, 0x05), (0x21, 0x90)], 0),
    ([(0x88, 150)], 2),
    ([GROUP_MASK_FRAME, GROUP_MASK_FRAME], 1),
    ([GROUP_MASK_FRAME, (32, 150)], 1),
    ([GROUP_MASK_FRAME, (0x21, 0x05)], 1),
    ([GROUP_MASK_FRAME, (0xFE, 100)], 1),
    ([GROUP_MASK_FRAME, (0x89, 0x10)], 1),
    ([GROUP_MASK_FRAME, (32, 0xFF)], 0),
])
def test_a_stop_is_one_group_mask_and_no_level_after_it(pairs, faults):
    assert len(gearsim.stop_fade_faults(_frames(*pairs), 4, [16, 17])) == faults


def _console(tmp_path, monkeypatch, dropped=(0, 2)):
    cfg, log = _peer(tmp_path, monkeypatch)
    sent, stats = [], list(dropped)

    def control(peer, command):
        sent.append(command)
        reply = {"write log": "usage: log off|change|frame|trace (now: change)",
                 "write stats": "decode_failed=0 other_width=0 ring_dropped=0 log_dropped=%d"
                 % (stats.pop(0) if command == "write stats" and stats else 0),
                 "write log frame": "log level frame",
                 "write log change": "log level change"}[command]
        with open(log, "a") as fh:
            fh.write(STAMP + "# %s\n" % reply)
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    monkeypatch.setattr(gearsim, "REPLY_QUIET_S", 0.0)
    return gearsim.GearSim(cfg), log, sent


def test_hearing_logs_frames_for_its_window_and_puts_the_level_back(tmp_path, monkeypatch):
    sim, log, sent = _console(tmp_path, monkeypatch)
    assert sim.log_level_now() == "change"
    sent.clear()
    with gearsim.GearOracle(sim).hearing() as window:
        with open(log, "a") as fh:
            fh.write(STAMP + "F 7 88 ff Group(4) DAPC\n")
    assert sent == ["write log", "write stats", "write log frame", "write log change"]
    assert window.heard() == [gearsim.Heard(7, 0x88, 0xFF)]
    assert window.blind() == {"log_dropped": 2}


def test_a_level_the_emulator_does_not_name_is_refused(tmp_path, monkeypatch):
    sim, log, _ = _console(tmp_path, monkeypatch)

    def control(peer, command):
        with open(log, "a") as fh:
            fh.write(STAMP + "# unknown command 'log' — try 'help'\n")
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    with pytest.raises(gearsim.GearSimUnavailable, match="log level"):
        sim.log_level_now()


def test_moved_names_only_the_counters_that_moved():
    assert gearsim.moved({"a": 1, "b": 2}, {"a": 1, "b": 5, "c": 1}, ("a", "b", "c")) == {
        "b": 3, "c": 1}
