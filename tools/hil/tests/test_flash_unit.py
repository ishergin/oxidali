import importlib.util
import json
import subprocess

import pytest

from hil import flash, serialmon
from hil.config import HilConfig

C6 = "/dev/cu.usbmodemC6SIM1"
P4 = "/dev/cu.usbmodemP4DUT1"


def _cfg(tmp_path, port, pinned):
    return HilConfig(state_dir=tmp_path, serial_port=port,
                     serial_port_pinned=pinned, serial_remote="")


def test_an_env_pin_is_used_and_recorded(tmp_path, capsys):
    port = flash.serial_port_for_flash(_cfg(tmp_path, P4, pinned=True))
    assert port == P4
    assert serialmon.recorded_pin(_cfg(tmp_path, C6, pinned=False)) == P4
    assert "pinned via HIL_SERIAL_PORT" in capsys.readouterr().out


def test_a_recorded_pin_reaches_espflash_without_the_env(tmp_path, capsys):
    serialmon.record_pin(_cfg(tmp_path, P4, pinned=False), P4)
    port = flash.serial_port_for_flash(_cfg(tmp_path, C6, pinned=False))
    assert port == P4
    assert "serial_port.pin" in capsys.readouterr().out


def test_two_boards_and_no_pin_refuse_to_guess(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(flash.serialport, "candidates", lambda: [C6, P4])
    assert flash.serial_port_for_flash(_cfg(tmp_path, C6, pinned=False)) is None
    err = capsys.readouterr().err
    assert "HIL_SERIAL_PORT" in err
    assert C6 in err and P4 in err


def test_one_board_and_no_pin_proceeds_auto(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(flash.serialport, "candidates", lambda: [P4])
    port = flash.serial_port_for_flash(_cfg(tmp_path, P4, pinned=False))
    assert port == P4
    assert serialmon.recorded_pin(_cfg(tmp_path, P4, pinned=False)) is None
    assert "auto-discovered" in capsys.readouterr().out


def test_the_bridge_is_the_port_when_the_dut_is_remote(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(flash.remote_serial, "ensure", lambda cfg: None)
    cfg = HilConfig(state_dir=tmp_path, serial_port=C6, serial_port_pinned=False,
                    serial_remote="root@wb:/dev/serial/by-id/usb-1a86_x-if00")
    port = flash.serial_port_for_flash(cfg)
    assert port == flash.remote_serial.data_url(cfg)
    assert "WB bridge" in capsys.readouterr().out


HEAD_SHA = "51ab13c8f0e1d2c3b4a5968778695a4b3c2d1e0f"


def _manifest(tmp_path, body='{"firmware": {"sha256": "abc"}}'):
    p = tmp_path / "manifest.json"
    p.write_text(body)
    return p


def test_the_reported_version_is_recorded_and_accepted_when_it_names_head(tmp_path, monkeypatch):
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda _root: HEAD_SHA)
    m = _manifest(tmp_path)
    rc = flash.verify_running_version({"version": "0.1.868+51ab13c8"}, tmp_path, m)
    assert rc == 0
    assert json.loads(m.read_text())["firmware"]["reported_version"] == "0.1.868+51ab13c8"
    assert json.loads(m.read_text())["firmware"]["sha256"] == "abc"


def test_a_dirty_build_is_still_that_commit(tmp_path, monkeypatch):
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda _root: HEAD_SHA)
    rc = flash.verify_running_version(
        {"version": "0.1.868+51ab13c8.dirty.0913T0142"}, tmp_path, _manifest(tmp_path))
    assert rc == 0


def test_a_version_naming_another_commit_fails_the_flash(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda _root: HEAD_SHA)
    rc = flash.verify_running_version({"version": "0.1.860+deadbeef"}, tmp_path, _manifest(tmp_path))
    assert rc == 1
    assert json.loads((tmp_path / "manifest.json").read_text())[
        "firmware"]["reported_version"] == "0.1.860+deadbeef"
    assert "not the commit just built" in capsys.readouterr().err


def test_a_build_with_no_git_is_not_verified_rather_than_refused(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda _root: HEAD_SHA)
    rc = flash.verify_running_version({"version": "0.1.0+unknown"}, tmp_path, _manifest(tmp_path))
    assert rc == 0
    assert "not verified" in capsys.readouterr().out


@pytest.mark.parametrize("health", [
    {"version": "0.1.0"},
    {"version": ""},
    {},
    {"version": None},
])
def test_a_version_naming_no_commit_is_refused_not_excused(tmp_path, monkeypatch, capsys, health):
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda _root: HEAD_SHA)
    rc = flash.verify_running_version(health, tmp_path, _manifest(tmp_path))
    assert rc == 1
    assert "names no commit at all" in capsys.readouterr().err


def test_an_unamendable_manifest_does_not_fail_a_good_flash(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(flash.benchenv, "head_commit", lambda _root: HEAD_SHA)
    rc = flash.verify_running_version(
        {"version": "0.1.868+51ab13c8"}, tmp_path, _manifest(tmp_path, body="not json"))
    assert rc == 0
    assert "could not record reported version" in capsys.readouterr().out



def _image(tmp_path, *versions):
    blob = b"\x00\x01rodata" + b"".join(b"\x00" + v.encode() for v in versions) + b"\xff" * 64
    path = tmp_path / "dali2rust-merged.bin"
    path.write_bytes(blob)
    return path


def test_the_version_is_read_out_of_the_image(tmp_path):
    assert flash.image_version(_image(tmp_path, "0.1.990+8e2e8a2f")) == "0.1.990+8e2e8a2f"
    assert flash.image_version(
        _image(tmp_path, "0.1.868+51ab13c8.dirty.0913T0142")
    ) == "0.1.868+51ab13c8.dirty.0913T0142"
    assert flash.image_version(_image(tmp_path, "0.1.0+unknown")) == "0.1.0+unknown"


def test_an_unreadable_or_ambiguous_image_answers_none(tmp_path):
    assert flash.image_version(tmp_path / "absent.bin") is None
    assert flash.image_version(_image(tmp_path, "no version here")) is None
    assert flash.image_version(
        _image(tmp_path, "0.1.990+8e2e8a2f", "0.1.991+deadbeef")
    ) is None


def test_a_matching_image_passes_even_when_head_has_moved_on(tmp_path, capsys):
    image = _image(tmp_path, "0.1.990+8e2e8a2f")

    rc = flash.verify_running_version(
        {"version": "0.1.990+8e2e8a2f"}, tmp_path, tmp_path / "manifest.json",
        image=image)

    assert rc == 0
    assert "matches the image just written" in capsys.readouterr().out


def test_a_stale_image_on_the_die_still_fails(tmp_path, capsys):
    image = _image(tmp_path, "0.1.990+8e2e8a2f")

    rc = flash.verify_running_version(
        {"version": "0.1.947+bfdba300"}, tmp_path, tmp_path / "manifest.json",
        image=image)

    assert rc == 1
    assert "0.1.990+8e2e8a2f" in capsys.readouterr().err


class _Ran:
    def __init__(self, returncode):
        self.returncode = returncode


def _mirror(monkeypatch, returncode, seen):
    def fake_run(cmd, cwd=None, **kwargs):
        seen.append(cmd)
        return _Ran(returncode)
    monkeypatch.setattr(flash.subprocess, "run", fake_run)


def test_a_stale_ui_bundle_refuses_the_build(tmp_path, monkeypatch, capsys):
    seen, built = [], []
    _mirror(monkeypatch, 1, seen)
    monkeypatch.setattr(flash, "build", lambda *a, **k: built.append(1) or 0)
    assert flash.run(_cfg(tmp_path, P4, pinned=True), build_only=True) == 1
    assert seen == [["bash", flash.UI_MIRROR_SCRIPT]]
    assert built == []
    assert "build_web_ui.sh" in capsys.readouterr().err


def test_allow_stale_ui_builds_with_the_previous_bundle(tmp_path, monkeypatch):
    seen, built = [], []
    _mirror(monkeypatch, 1, seen)
    monkeypatch.setattr(flash, "build", lambda *a, **k: built.append(1) or 0)
    cfg = _cfg(tmp_path, P4, pinned=True)
    assert flash.run(cfg, build_only=True, allow_stale_ui=True) == 0
    assert built == [1]


ISR_SPEC = {"isr_iram_check": True, "firmware_bin": "target/riscv32imafc-esp-espidf/debug/x"}
GATE_RED = 1
TOOL_VERSION = "esp-14.2.0_20251107"


def test_the_gate_tells_missing_tools_apart_from_an_interrupt_that_reaches_flash(
        tmp_path, monkeypatch, capsys):
    cfg = _cfg(tmp_path, P4, pinned=True)
    _mirror(monkeypatch, flash.ISR_IRAM_TOOLS_MISSING, [])
    assert flash.check_isr_iram(cfg, ISR_SPEC) == (flash.ISR_IRAM_UNCHECKED, 1)
    unchecked = capsys.readouterr().err
    assert "could not check" in unchecked and "SKIP or UNCHECKED line" in unchecked
    assert "reaches flash" not in unchecked
    assert flash.check_isr_iram(cfg, ISR_SPEC, allow_red=True) == (flash.ISR_IRAM_UNCHECKED, 0)
    capsys.readouterr()
    _mirror(monkeypatch, flash.ISR_IRAM_NOTHING_CHECKED, [])
    assert flash.check_isr_iram(cfg, ISR_SPEC) == (flash.ISR_IRAM_UNCHECKED, 1)
    assert "reaches flash" not in capsys.readouterr().err
    _mirror(monkeypatch, GATE_RED, [])
    assert flash.check_isr_iram(cfg, ISR_SPEC) == (flash.ISR_IRAM_FAILED, 1)
    assert "reaches flash" in capsys.readouterr().err


def _isr_gate():
    spec = importlib.util.spec_from_file_location(
        "verify_dali_isr_iram", flash.REPO_ROOT / flash.ISR_IRAM_SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _install_tool(tools_root, name):
    path = (tools_root / "riscv32-esp-elf" / TOOL_VERSION / "riscv32-esp-elf" / "bin"
            / ("riscv32-esp-elf-" + name))
    path.parent.mkdir(parents=True)
    path.write_text("")
    return path


def test_the_gate_takes_the_toolchain_the_firmware_build_installed_first(tmp_path, monkeypatch,
                                                                          capsys):
    gate = _isr_gate()
    monkeypatch.setattr(gate, "ROOT", tmp_path / "repo")
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    built = _install_tool(tmp_path / "repo" / ".embuild" / "espressif" / "tools", "objdump")
    shared = _install_tool(tmp_path / "home" / ".espressif" / "tools", "objdump")
    assert gate._tool("objdump", True) == str(built)
    built.unlink()
    assert gate._tool("objdump", True) == str(shared)
    shared.unlink()
    with pytest.raises(SystemExit) as missing:
        gate._tool("objdump", True)
    assert missing.value.code == flash.ISR_IRAM_TOOLS_MISSING
    assert ".embuild" in capsys.readouterr().out


SECTIONS = ("  [ 1] .iram0.text PROGBITS 4ff00000 001000 008000 00 AX 0 0 4\n"
            "  [ 2] .flash.text PROGBITS 40000000 009000 010000 00 AX 0 0 4\n")


def _linked(tools):
    def run(argv, **kwargs):
        return subprocess.CompletedProcess(argv, 0, tools.get(argv[1], ""), "")
    return run


def test_a_gate_that_finds_nothing_to_check_says_unchecked_not_red(tmp_path, monkeypatch, capsys):
    gate = _isr_gate()
    elf = tmp_path / "dali2rust"
    elf.write_bytes(b"")
    monkeypatch.setattr(gate, "_tool", lambda name, require: name)
    monkeypatch.setattr(gate.subprocess, "run", _linked({"-S": SECTIONS}))
    assert gate.main(["gate", "--require-tools", str(elf)]) == flash.ISR_IRAM_NOTHING_CHECKED
    assert "nothing was actually checked" in capsys.readouterr().out
    monkeypatch.setattr(gate.subprocess, "run", _linked({"-S": ""}))
    with pytest.raises(SystemExit) as stopped:
        gate.main(["gate", "--require-tools", str(elf)])
    assert stopped.value.code == flash.ISR_IRAM_NOTHING_CHECKED
    assert (gate.EXIT_MISSING, gate.EXIT_UNCHECKED, gate.EXIT_RED) == (
        flash.ISR_IRAM_TOOLS_MISSING, flash.ISR_IRAM_NOTHING_CHECKED, GATE_RED)


ENTRY_TABLE = ("4ff00010 g F .iram0.text 00000040 dali_phy_raw_isr\n"
               "4ff00050 g F .iram0.text 00000040 dali_phy_alarm_isr\n")
ENTRIES_IN_IRAM = ("4ff00010 <dali2rust_dali_phy::dali_phy_raw_isr>:\n"
                   "4ff00050 <dali2rust_dali_phy::dali_phy_alarm_isr>:\n")


def test_an_entry_point_linked_outside_iram_is_red_not_unchecked(tmp_path, monkeypatch, capsys):
    gate = _isr_gate()
    elf = tmp_path / "dali2rust"
    elf.write_bytes(b"")
    monkeypatch.setattr(gate, "_tool", lambda name, require: name)
    monkeypatch.setattr(gate.subprocess, "run", _linked({"-S": SECTIONS, "-t": ENTRY_TABLE}))
    assert gate.main(["gate", "--require-tools", str(elf)]) == GATE_RED
    assert "linked outside IRAM" in capsys.readouterr().out
    monkeypatch.setattr(gate.subprocess, "run", _linked(
        {"-S": SECTIONS, "-t": ENTRY_TABLE, "-d": ENTRIES_IN_IRAM}))
    assert gate.main(["gate", "--require-tools", str(elf)]) == 0
