import importlib.util

from hil import tiers

HALTS = '''
def test_halts(hil_config):
    remote_serial.control(hil_config, "bootloader")
'''
MENTIONS = '''
def test_mentions(api):
    note = "remote_serial.control(hil_config, 'run') is what dut_reboot does"
'''
MODULE = '''
from hil import remote_serial


def _halt(cfg):
    remote_serial.control(cfg, "bootloader")


def test_through_a_helper(hil_config):
    _halt(hil_config)


def test_innocent(api):
    len([])
'''


def test_the_reboot_fixture_without_the_marker_is_named():
    found = tiers.reboot_violation("tests/test_poller.py::test_x",
                                   ["api", "dut_reboot"], {"hil_id"}, [""])
    assert "tests/test_poller.py::test_x" in found
    assert "dut_reboot" in found


def test_the_marker_clears_the_violation():
    assert tiers.reboot_violation("t", ["dut_reboot"], {"destructive"}, [HALTS]) is None


def test_a_bridge_control_call_without_the_marker_is_named():
    found = tiers.reboot_violation("t", ["hil_config"], set(), [HALTS])
    assert "remote_serial.control()" in found


def test_a_mention_in_a_string_is_not_a_call():
    assert tiers.reboot_violation("t", ["api"], set(), [MENTIONS]) is None


def test_a_helper_in_the_same_module_is_followed(tmp_path):
    path = tmp_path / "halting_module.py"
    path.write_text(MODULE)
    spec = importlib.util.spec_from_file_location("halting_module", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    assert tiers.reboot_violation(
        "t", ["hil_config"], set(), tiers.reach_sources(module.test_through_a_helper))
    assert tiers.reboot_violation(
        "t", ["api"], set(), tiers.reach_sources(module.test_innocent)) is None
