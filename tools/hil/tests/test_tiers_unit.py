import ast
import importlib.util
from pathlib import Path

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


AUTOUSE = ["production_state", "bench_baseline", "dut_continuity", "optical_session",
           "_fast_fade_prep", "unit_test_writes_stay_off_the_session_log", "request",
           "pytestconfig"]


def test_a_session_of_unit_tests_is_hardware_free():
    closures = [AUTOUSE + ["tmp_path", "monkeypatch"], AUTOUSE + ["capsys"]]
    assert tiers.session_hardware_free(closures)


def test_one_bench_fixture_makes_the_whole_session_a_bench_session():
    closures = [AUTOUSE + ["tmp_path"], AUTOUSE + ["api", "hil_config"]]
    assert not tiers.session_hardware_free(closures)


def test_the_bench_configuration_alone_is_a_bench_fixture():
    assert not tiers.session_hardware_free([AUTOUSE + ["hil_config", "test_artifacts"]])


def test_an_empty_session_is_hardware_free():
    assert tiers.session_hardware_free([])


def test_a_unit_module_that_requests_a_bench_fixture_is_named():
    found = tiers.unit_violation("tests/test_x_unit.py::test_y", "tests/test_x_unit.py",
                                 ["tmp_path", "api"])
    assert "tests/test_x_unit.py::test_y" in found and "api" in found
    assert tiers.unit_violation("tests/test_x.py::test_y", "tests/test_x.py", ["api"]) is None
    assert tiers.unit_violation("tests/test_x_unit.py::test_y", "tests/test_x_unit.py",
                                ["tmp_path"]) is None


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


TESTS = Path(__file__).resolve().parent
PYTEST_OWN_FIXTURES = frozenset({
    "tmp_path", "tmp_path_factory", "tmpdir", "monkeypatch", "capsys", "capsysbinary",
    "capfd", "capfdbinary", "caplog", "recwarn", "request", "pytestconfig", "pytester",
})
PARAMETRIZE = "parametrize"


def _arguments(test):
    args = test.args
    return {arg.arg for arg in args.posonlyargs + args.args + args.kwonlyargs}


def _parametrized(test):
    names = set()
    for decorator in test.decorator_list:
        if isinstance(decorator, ast.Call) and getattr(decorator.func, "attr", "") == PARAMETRIZE \
                and decorator.args and isinstance(decorator.args[0], ast.Constant):
            names |= {name.strip() for name in decorator.args[0].value.split(",")}
    return names


def _needs_only_pytest(module):
    tests = [node for node in ast.walk(ast.parse(module.read_text()))
             if isinstance(node, ast.FunctionDef) and node.name.startswith("test_")]
    return bool(tests) and all(_arguments(test) - _parametrized(test) <= PYTEST_OWN_FIXTURES
                               for test in tests)


def test_a_module_whose_tests_need_only_pytest_is_a_unit_module_the_unit_run_takes():
    missed = [module.name for module in sorted(TESTS.glob("test_*.py"))
              if not tiers.is_unit(module) and _needs_only_pytest(module)]
    assert missed == []
