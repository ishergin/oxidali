import json

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
    assert sim.reserve("0-15") == "reserved 0x000000000000ffff (16 addresses)"
    assert sent == ["write reserve 0-15"]


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
