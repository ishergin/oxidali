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


SHOW_HEAD = "# idx addr random   type lvl groups on  drop\n"
LATE_ROW_S = 0.3


def _row(index, short):
    return "# %3d %4d a1b2c3  DT6 254 0x0010 yes 0\n" % (index, short)


def _replying(tmp_path, monkeypatch, now, later=()):
    cfg, log = _peer(tmp_path, monkeypatch)

    def write(lines):
        with open(log, "a") as fh:
            fh.writelines(STAMP + line for line in lines)

    def control(peer, command):
        write(now)
        if later:
            threading.Timer(LATE_ROW_S, write, args=(later,)).start()
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    monkeypatch.setattr(gearsim, "REPLY_QUIET_S", 0.05)
    monkeypatch.setattr(gearsim, "POLL_S", 0.01)
    monkeypatch.setattr(gearsim, "REPLY_TIMEOUT_S", 1.0)
    return gearsim.GearSim(cfg)


def test_a_show_row_the_log_writes_after_a_pause_is_still_read(tmp_path, monkeypatch):
    sim = _replying(tmp_path, monkeypatch, [SHOW_HEAD, _row(0, 16), _row(1, 17)],
                   later=[_row(2, 18), "# 3 gear listed\n"])
    assert [row["short"] for row in sim.show()] == [16, 17, 18]


def test_a_show_reply_without_its_footer_is_refused(tmp_path, monkeypatch):
    sim = _replying(tmp_path, monkeypatch, [SHOW_HEAD, _row(0, 16), _row(1, 17)])
    with pytest.raises(gearsim.GearSimUnavailable, match="gear listed"):
        sim.show()


def test_a_show_that_lost_a_row_is_refused(tmp_path, monkeypatch):
    sim = _replying(tmp_path, monkeypatch,
                    [SHOW_HEAD, _row(0, 16), _row(2, 18), "# 3 gear listed\n"])
    with pytest.raises(gearsim.GearSimUnavailable, match="listed 3 gear and 2 rows"):
        sim.show()


def test_a_stats_reply_cut_before_its_send_twice_line_is_refused(tmp_path, monkeypatch):
    sim = _replying(tmp_path, monkeypatch, ["# fleet 12 gear, 0 enabled, reserved 0xffff\n",
                                            "# heard=4 forward16=4 answered=1 collisions=0\n"],
                    later=["# send_twice_pairs=0 send_twice_interloper=0\n"])
    assert sim.stats()["send_twice_interloper"] == 0
    sim = _replying(tmp_path, monkeypatch, ["# fleet 12 gear, 0 enabled, reserved 0xffff\n"])
    with pytest.raises(gearsim.GearSimUnavailable, match="send_twice_interloper"):
        sim.stats()


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


def _dtrs(mirek):
    return [(0xA3, mirek & 0xFF), (0xC3, mirek >> 8)]


def _write(mirek, *between):
    return _dtrs(mirek) + [ENABLE_DT8, STAGE, ENABLE_DT8, COLOUR_STATUS] + list(between) + [
        ENABLE_DT8, ACTIVATE]


def test_every_colour_the_fix_stages_is_paired_with_its_activate():
    tally = gearsim.colour_writes(_frames(*(_write(370) + _write(286))), 20)
    assert tally.activated() == [370, 286]
    assert (tally.gates, tally.faults) == ([], [])


def test_a_status_gate_between_staging_and_activation_is_direct_evidence():
    tally = gearsim.colour_writes(_frames(*_write(370, STATUS)), 20)
    assert tally.activated() == [370] and tally.faults == []
    assert len(tally.gates) == 1 and "QUERY STATUS" in tally.gates[0]


def test_a_status_query_to_another_gear_is_not_a_gate():
    tally = gearsim.colour_writes(_frames(*_write(370, (0x03, 0x90))), 20)
    assert tally.gates == []


def test_a_staged_colour_never_activated_is_a_fault():
    tally = gearsim.colour_writes(_frames(*_dtrs(370), ENABLE_DT8, STAGE, STATUS), 20)
    assert tally.activated() == []
    assert any("never activated" in fault for fault in tally.faults)


def test_a_colour_replaced_before_its_activate_cannot_hide_behind_an_extra_one():
    frames = _write(370)[:-2] + _write(286) + [ENABLE_DT8, ACTIVATE]
    tally = gearsim.colour_writes(_frames(*frames), 20)
    assert tally.activated() == [286]
    assert any("370 mirek staged on SA20 was replaced by 286" in f for f in tally.faults)


def test_an_activate_the_enable_did_not_open_is_a_fault():
    frames = _dtrs(370) + [ENABLE_DT8, STAGE, (0x29, 0x98), ACTIVATE]
    tally = gearsim.colour_writes(_frames(*frames), 20)
    assert any("did not follow ENABLE DEVICE TYPE 8" in fault for fault in tally.faults)


def test_a_retried_unit_counts_its_colour_once():
    frames = _dtrs(370) + [ENABLE_DT8, STAGE] + _write(370) + _write(370)
    tally = gearsim.colour_writes(_frames(*frames), 20)
    assert tally.activated() == [370] and tally.faults == []


GROUP_MASK_FRAME = (0x88, 0xFF)


@pytest.mark.parametrize("pairs,masks,moves", [
    ([GROUP_MASK_FRAME], 1, 0),
    ([GROUP_MASK_FRAME, (0x89, 0xA0), (0x24, 0x05), (0x21, 0x90)], 1, 0),
    ([(0x88, 150)], 0, 1),
    ([GROUP_MASK_FRAME, GROUP_MASK_FRAME], 2, 0),
    ([GROUP_MASK_FRAME, (32, 150)], 1, 1),
    ([GROUP_MASK_FRAME, (0x21, 0x05)], 1, 1),
    ([GROUP_MASK_FRAME, (0xFE, 100)], 1, 1),
    ([GROUP_MASK_FRAME, (0x89, 0x10)], 1, 1),
    ([(32, 150), GROUP_MASK_FRAME, (32, 0xFF)], 1, 1),
    ([(0x88, 100), GROUP_MASK_FRAME], 1, 1),
])
def test_a_stop_is_counted_in_masks_and_level_frames_anywhere_in_its_window(pairs, masks, moves):
    frames = _frames(*pairs)
    assert gearsim.mask_frames(frames, 4) == masks
    assert len(gearsim.level_moves(frames, 4, [16, 17])) == moves


def _console(tmp_path, monkeypatch, dropped=(0, 2)):
    cfg, log = _peer(tmp_path, monkeypatch)
    sent, stats = [], list(dropped)

    def control(peer, command):
        sent.append(command)
        reply = {"write log": "usage: log off|change|frame|trace (now: change)",
                 "write stats": "decode_failed=0 other_width=0 ring_dropped=0 log_dropped=%d\n"
                 "send_twice_pairs=0 send_twice_interloper=0"
                 % (stats.pop(0) if command == "write stats" and stats else 0),
                 "write log frame": "log level frame",
                 "write log change": "log level change"}[command]
        with open(log, "a") as fh:
            fh.writelines(STAMP + "# %s\n" % line for line in reply.split("\n"))
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
    assert window.heard_settled(0.05, 1.0, 0.01) == [gearsim.Heard(7, 0x88, 0xFF)]
    assert window.losses() == {"log_dropped": 2}


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


def test_a_frame_the_dut_sent_counts_as_heard_only_in_order():
    heard = _frames((0xA3, 0x4D), (0x03, 0x90), (0xC1, 0x08), (0x29, 0xE2))
    assert gearsim.unheard([(0xA3, 0x4D), (0xC1, 0x08), (0x29, 0xE2)], heard) == []
    assert gearsim.unheard([(0xC1, 0x08), (0xA3, 0x4D), (0x29, 0xE2)], heard) == [(0xA3, 0x4D)]
    assert gearsim.unheard([(0x88, 0xFF)], heard) == [(0x88, 0xFF)]


def _answering(tmp_path, monkeypatch, answer):
    cfg, log = _peer(tmp_path, monkeypatch)
    sent = []

    def control(peer, command):
        sent.append(command)
        with open(log, "a") as fh:
            fh.writelines(STAMP + "# %s\n" % line for line in answer(command.split()[1:]))
        return "ok"
    monkeypatch.setattr(gearsim.remote_serial, "control", control)
    monkeypatch.setattr(gearsim, "REPLY_QUIET_S", 0.0)
    return gearsim.GearSim(cfg), sent


def _counting_peek(words):
    short, bank, offset, count = int(words[1]), words[2], int(words[3], 16), int(words[4])
    cells = "".join(" %02X" % ((offset + i) & 0xFF) for i in range(count))
    return ["A%02d bank %s 0x%02X:%s" % (short, bank, offset, cells)]


@pytest.mark.parametrize("line,parsed", [
    ("A16 bank 1 0x3C: 44 41 --", (16, 1, 0x3C, [0x44, 0x41, None])),
    (STAMP + "A07 bank 0 0x00: 1A", (7, 0, 0x00, [0x1A])),
    ("A16 bank 1 0x3c: 44", None),
    ("A16 bank 1 0x3C:", None),
    ("A16 bank1_lock=0x55 on 1 gear", None),
    ("A16 peek found 0 gear", None),
])
def test_a_peek_line_parses_into_its_bytes_and_holes(line, parsed):
    assert gearsim.parse_peek(line) == parsed


def test_a_long_peek_is_read_in_pieces_the_console_takes(tmp_path, monkeypatch):
    sim, sent = _answering(tmp_path, monkeypatch, _counting_peek)
    assert sim.peek(16, 1, 0x3C, 60) == list(range(0x3C, 0x3C + 60))
    assert sent == ["write peek 16 1 0x3C 32", "write peek 16 1 0x5C 28"]


@pytest.mark.parametrize("reply", [
    ["A16 bank 1 0x3D: 00"], ["A16 bank 1 0x3C: 00 01"], ["A17 bank 1 0x3C: 00"],
    ["A16 peek found 0 gear"], [],
])
def test_a_peek_that_shows_another_span_is_refused(tmp_path, monkeypatch, reply):
    sim, _ = _answering(tmp_path, monkeypatch, lambda words: reply)
    with pytest.raises(gearsim.GearSimUnavailable, match="did not show"):
        sim.peek(16, 1, 0x3C, 1)


def test_a_lock_is_confirmed_by_its_echo_on_one_gear(tmp_path, monkeypatch):
    sim, sent = _answering(tmp_path, monkeypatch,
                           lambda words: ["A16 bank1_lock=0xAA on 1 gear"])
    assert sim.bank1_lock(16, 0xAA) == "A16 bank1_lock=0xAA on 1 gear"
    assert sent == ["write lock 16 0xAA"]
    with pytest.raises(gearsim.GearSimUnavailable):
        sim.bank1_lock(16, 0x55)


def test_a_luminaire_on_no_gear_is_refused(tmp_path, monkeypatch):
    sim, sent = _answering(tmp_path, monkeypatch,
                           lambda words: ["luminaire %s on 0 gear at short %s" % (words[1],
                                                                                 words[0])])
    with pytest.raises(gearsim.GearSimUnavailable):
        sim.luminaire(16, "3")
    assert sent == ["write luminaire 16 3"]


def test_an_image_without_the_command_names_the_flash_it_needs(tmp_path, monkeypatch):
    sim, _ = _answering(tmp_path, monkeypatch,
                        lambda words: ["unknown command 'lock' — try 'help'",
                                       "unknown command 'peek' — try 'help'"])
    with pytest.raises(gearsim.GearSimUnavailable, match="flash the peer"):
        sim.bank1_lock(16, 0xAA)
    with pytest.raises(gearsim.GearSimUnavailable, match="flash the peer"):
        sim.peek(16, 1, 0x02, 1)
