import importlib.util
from pathlib import Path

import pytest

from hil.config import HilConfig

HIL_ROOT = Path(__file__).resolve().parent.parent
BOOT = "I (1) dali2rust: boot\n"
HCL_STALE = "W (2) redundancy: worker has not turned for 61 s: hcl-scheduler"
QUALIFYING = [{"d_ticks": 1, "d_timeouts": 2}]
TIMING = ("2026-09-26T08:00:00.000Z I (3) x: DALI sniff timing: stage max=7 ticks "
          "over budget=2 of 40 poll gap max=900 us")
FLUSH = "2026-09-26T08:00:01.000Z W (4) x: slow flush_dirty_slices took 310 ms"


def _script(name):
    spec = importlib.util.spec_from_file_location(name, HIL_ROOT / ("%s.py" % name))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ISSUE86 = _script("issue86_acceptance")
ISSUE89 = _script("issue89_flush_stall_ab")


def _cfg(tmp_path, lamp_shorts="0,2,3"):
    return HilConfig(state_dir=tmp_path, serial_remote="", base="http://127.0.0.1:9",
                     lamp_shorts=lamp_shorts)


def test_issue86_reads_the_log_its_own_monitor_writes(tmp_path):
    log = tmp_path / "elsewhere" / "serial.log"
    log.parent.mkdir()
    log.write_text(BOOT)
    (tmp_path / "serial_monitor.pid.log").write_text(str(log))
    assert ISSUE86.serial_start(_cfg(tmp_path)) == (log, len(BOOT))


def test_issue86_passes_nothing_on_a_window_the_log_never_grew_into(tmp_path):
    log = tmp_path / "serial.log"
    assert ISSUE86.serial_window(log, 0) is None
    log.write_text(BOOT)
    start = log.stat().st_size
    assert ISSUE86.serial_window(log, start) is None
    assert not ISSUE86.verdict(QUALIFYING, False, None, 0)


def test_issue86_judges_a_window_the_log_did_grow_into(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text(BOOT)
    start = log.stat().st_size
    with log.open("a") as fh:
        fh.write(BOOT)
    assert ISSUE86.serial_window(log, start) == []
    assert ISSUE86.verdict(QUALIFYING, False, [], 0)
    with log.open("a") as fh:
        fh.write(HCL_STALE + "\n")
    stale = ISSUE86.serial_window(log, start)
    assert stale == [HCL_STALE]
    assert not ISSUE86.verdict(QUALIFYING, False, stale, 0)


def test_issue89_reads_its_windows_by_offset_whatever_the_stamps(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text(BOOT)
    start = ISSUE89.log_size(log)
    assert ISSUE89.log_lines(log, start) is None
    with log.open("a") as fh:
        fh.write("%s\n%s\nnoise\n" % (TIMING, FLUSH))
    assert ISSUE89.window(ISSUE89.log_lines(log, start)) == ([7], 2, 40, [900], [310])


class _Registry:
    def __init__(self, record):
        self.record, self.patches = dict(record), []

    def state(self, short):
        return dict(self.record)

    def device_patch(self, short, body):
        self.patches.append(body)
        self.record.update(body)

    def devices(self):
        return {"physical_devices": [{"short_address": s} for s in (1, 2, 4)]}


def test_issue89_provokes_with_notes_it_restores_and_never_writes_a_name():
    registry = _Registry({"name": "Kitchen", "notes": "the owner's note"})
    toggle = ISSUE89.NotesToggle(registry, 2)
    for _ in range(3):
        toggle()
    assert registry.record["notes"] == ISSUE89.MARKER
    toggle.restore()
    assert registry.record == {"name": "Kitchen", "notes": "the owner's note"}
    assert all(set(body) == {"notes"} for body in registry.patches)


def test_issue89_writes_only_gear_the_bench_may_write(tmp_path):
    assert ISSUE89.probe_target(_Registry({}), _cfg(tmp_path)) == 2
    with pytest.raises(SystemExit, match="HIL_LAMP_SHORTS"):
        ISSUE89.probe_target(_Registry({}), _cfg(tmp_path, lamp_shorts="7"))


def test_issue89_names_no_bench_address_and_no_checkout_path():
    source = (HIL_ROOT / "issue89_flush_stall_ab.py").read_text()
    assert "192.168." not in source and "/Users/" not in source
