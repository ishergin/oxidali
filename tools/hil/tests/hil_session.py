import os
import time

import pytest

from hil import api as api_mod
from hil import config as config_mod
from hil import prod_state
from hil import remote_serial as remote_serial_mod
from hil import serialmon as serialmon_mod
from hil import tiers
from hil import validity
from hil.results import failure_line
from hil.seriallog import SerialLog
from hil_harness import peer_health

def pytest_addoption(parser):
    parser.addoption(
        "--no-camera", action="store_true", default=False,
        help="explicitly allow an optical-less run: camera/calibration "
             "problems become skips instead of failures")
    parser.addoption(
        "--skip-calibration", action="store_true", default=False,
        help="reuse the calibration already on disk instead of running one "
             "first; the saved profile must still be schema v3")
    parser.addoption(
        "--fast-fade", action="store_true", default=False,
        help="bench prep: set the fade time of every lamp in HIL_LAMP_SHORTS to 0 "
             "(instant) after production_state's snapshot, so level/optical "
             "assertions settle inside their windows instead of racing a "
             "multi-second ramp; production_state restores it at the end of the "
             "session, so it refuses to run with HIL_STATE_GUARD=0.")


def pytest_runtest_setup(item):
    api_mod.Client.context = item.nodeid


def _collection_lint(items):
    violations = [tiers.reboot_violation(
        item.nodeid, item.fixturenames, {m.name for m in item.iter_markers()},
        tiers.reach_sources(item.function) if hasattr(item, "function") else [])
        for item in items]
    violations += [tiers.unit_violation(item.nodeid, item.path, item.fixturenames)
                   for item in items]
    violations = [v for v in violations if v]
    if violations:
        raise pytest.UsageError("\n".join(violations))


NO_SERIAL = "no board attached here and no WB bridge answering"


def _serial_absence(cfg):
    dut_serial_port, _ = serialmon_mod.effective_port(cfg)
    if "://" not in dut_serial_port:
        return dut_serial_port, None if os.path.exists(dut_serial_port) else NO_SERIAL
    try:
        remote_serial_mod.control(cfg, "ping")
        return dut_serial_port, None
    except remote_serial_mod.BridgePortGone as exc:
        return dut_serial_port, str(exc)
    except Exception:
        return dut_serial_port, NO_SERIAL


def _take_isr_baseline(config, cfg):
    try:
        stats = api_mod.Client(cfg).stats()
        config._hil_isr_baseline = (
            validity.isr_timing_counters(stats),
            (stats.get("controller") or {}).get("uptime_ms"),
            time.monotonic(),
        )
    except Exception:
        config._hil_isr_baseline = ({}, None, None)


def _take_session_baselines(config, cfg):
    _take_isr_baseline(config, cfg)
    config._hil_peer_start = peer_health(cfg.peer())[1] if cfg.has_peer else None


def _tier(item):
    if item.get_closest_marker("destructive"):
        return 2 if "commissioning" in item.nodeid else 1
    return 0


def _mark_skips(items, serial_absent, dut_serial_port):
    allow_destructive = os.environ.get("HIL_ALLOW_DESTRUCTIVE") == "1"
    for item in items:
        if serial_absent and item.get_closest_marker("serial"):
            item.add_marker(pytest.mark.skip(
                reason="serial port %s unreachable — %s" % (dut_serial_port,
                                                           serial_absent)))
        if not allow_destructive and item.get_closest_marker("destructive"):
            item.add_marker(pytest.mark.skip(
                reason="destructive tier needs HIL_ALLOW_DESTRUCTIVE=1 "
                       "(run one test at a time; see tools/hil/README.md)"))


def pytest_collection_modifyitems(config, items):
    config._hil_hardware_free = tiers.session_hardware_free(
        item.fixturenames for item in items)
    _collection_lint(items)
    dut_serial_port, serial_absent = None, None
    if not config._hil_hardware_free:
        cfg = config_mod.load()
        dut_serial_port, serial_absent = _serial_absence(cfg)
        _take_session_baselines(config, cfg)
    _mark_skips(items, serial_absent, dut_serial_port)
    items.sort(key=_tier)


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    outcome = yield
    report = outcome.get_result()
    if report.when != "call" and not (
            report.when == "setup" and (report.skipped or report.failed)):
        return
    results = item.config._hil_step_results
    name = item.name.replace("test_", "", 1)
    if report.passed:
        results.append(("PASS", name, ""))
    elif report.skipped:
        reason = ""
        if isinstance(report.longrepr, tuple):
            reason = report.longrepr[2]
        results.append(("SKIP", name, str(reason)))
    elif report.failed:
        results.append(("FAIL", name, failure_line(report.longrepr)))


def _refuse_unguarded_fast_fade(config):
    if config.getoption("--fast-fade") and not prod_state.guard_enabled():
        raise pytest.UsageError(
            "--fast-fade writes fade time 0 that only production_state restores; "
            "with HIL_STATE_GUARD=0 nothing would bring it back")


def pytest_configure(config):
    _refuse_unguarded_fast_fade(config)
    config.addinivalue_line("markers", "needs_capability(name): auto-skip when absent")
    config._hil_step_results = []
    config._hil_validity = {"baseline": [], "reboots": [], "contended": []}
    config._hil_uptime = None
    config._hil_counted = []
    config._hil_isr_baseline = ({}, None, None)
    try:
        path = SerialLog(config_mod.load()).log_path
        config._hil_serial_offset = path.stat().st_size if path.exists() else 0
    except Exception:
        config._hil_serial_offset = None
