import collections
import datetime
import os
import subprocess
import time

import pytest

from hil.gearsim import GearSim, GearSimUnavailable

from hil import api as api_mod
from hil import pair
from hil import config as config_mod
from hil import remote_serial as remote_serial_mod
from hil import serialmon as serialmon_mod
from hil import validity
from hil.artifacts import Artifacts
from hil.seriallog import SerialLog
from hil.sniffer import SnifferTap, ssh_argv
from hil.wait import wait_until

ANCHOR_TZ = "UTC0"

BENCH_BASELINE_TZ = os.environ.get("HIL_BENCH_TZ", "MSK-3")

LEAKED_NAME_PREFIX = "hil-"

UPTIME_SLACK_S = 30.0

PANEL_SHORT = 0
NVM_SETTLING_WAIT_S = 90


def _await_nvm_settling(api, short):
    deadline = time.monotonic() + NVM_SETTLING_WAIT_S
    while time.monotonic() < deadline:
        device = api.input_device(short)
        settling_until = device.get("nvm_settling_until_ms")
        now = device.get("now_ms")
        if not settling_until or not now or settling_until <= now:
            return
        time.sleep(2.0)
    pytest.skip(
        "panel %d is still in its NVM settling window after %d s — an earlier "
        "write owns it, and nothing this test asks would be answered"
        % (short, NVM_SETTLING_WAIT_S))


@pytest.fixture()
def panel(api):
    devices = api.input_devices().get("input_devices") or []
    if not any(d["short_address"] == PANEL_SHORT for d in devices):
        pytest.skip("no control device at short %d — the 103 panel is off the "
                    "segment" % PANEL_SHORT)
    _await_nvm_settling(api, PANEL_SHORT)
    return PANEL_SHORT

RULES_SOURCE_LIMIT_BYTES = 12_240


@pytest.fixture(scope="session")
def hil_config():
    return config_mod.load()


@pytest.fixture(scope="session")
def run_dir(hil_config):
    return hil_config.new_run_dir()


@pytest.fixture()
def test_artifacts(run_dir, request):
    return Artifacts(run_dir, request.node.nodeid)


@pytest.fixture(scope="session")
def api(request, hil_config):
    client = _track(request.config, api_mod.Client(hil_config))
    try:
        client.health()
    except Exception as exc:
        pytest.skip("DUT unreachable at %s: %s" % (hil_config.base, exc))
    return client


@pytest.fixture(scope="session")
def peer_config(hil_config):
    if not hil_config.has_peer:
        pytest.skip("no second controller named (HIL_PEER_BASE / `hil --peer flash`)")
    return hil_config.peer()


@pytest.fixture(scope="session")
def peer_api(request, peer_config):
    client = _track(request.config, api_mod.Client(peer_config))
    try:
        client.health()
    except Exception as exc:
        pytest.skip("peer unreachable at %s: %s" % (peer_config.base, exc))
    return client


@pytest.fixture(scope="session")
def peer_api_if_any(request, hil_config):
    if not hil_config.has_peer:
        return None
    return _track(request.config, api_mod.Client(hil_config.peer()))


@pytest.fixture(scope="session")
def device_inventory(api):
    return api.devices()["physical_devices"]


@pytest.fixture(scope="session")
def capabilities(api, device_inventory):
    class Capabilities:
        def __init__(self):
            self._primed = set()

        def _flags(self, short):
            for d in api.devices()["physical_devices"]:
                if d["short_address"] == short:
                    return d.get("capabilities", {})
            return {}

        def ensure(self, short, cap):
            if self._flags(short).get(cap):
                return True
            if short not in self._primed:
                self._primed.add(short)
                try:
                    api.wait_op(api.attr_read(short))
                except api_mod.ApiError:
                    return False
                if self._flags(short).get(cap):
                    return True
            return False

        def any_lamp_with(self, cap):
            for short in api.optical_addrs():
                if self.ensure(short, cap):
                    return short
            return None

    return Capabilities()


@pytest.fixture()
def state_snapshot(api, hil_config):
    snap = api.snapshot_states()
    yield snap
    allowed = frozenset() if hil_config.lamps_read_only else hil_config.lamp_short_set()
    driven = [entry for entry in snap if entry["short_address"] in allowed]
    left = [entry for entry in snap if entry["short_address"] not in allowed]
    for line in api.state_divergences(left):
        print("state_snapshot: NOT restored (HIL_LAMPS_READ_ONLY or outside "
              "HIL_LAMP_SHORTS): %s" % line)
    if driven:
        api.restore_states(driven)


@pytest.fixture(scope="session")
def sniffer(request, hil_config, run_dir):
    validity_state = _validity(request.config)
    probe = subprocess.run(ssh_argv(hil_config, "true"),
                           capture_output=True, timeout=15)
    if probe.returncode != 0:
        validity_state["wb"] = "UNREACHABLE over ssh (%s) — every sniffer " \
            "test skipped, so no wire evidence was collected" % hil_config.wb_ssh
        pytest.skip("WB sniffer host %s unreachable over ssh" % hil_config.wb_ssh)
    tap = _track(request.config,
                 SnifferTap(hil_config, log_path=run_dir / "sniffer.log"))
    if not tap.wait_ready():
        tap.close()
        validity_state["wb"] = "reachable but the tap produced no data — no " \
            "wire evidence was collected"
        pytest.skip("sniffer tap produced no data (mosquitto_sub/topics?)")
    validity_state["wb"] = "reachable, tap live (%s)" % hil_config.wb_ssh
    start = tap.log_path.stat().st_size if tap.log_path.exists() else 0
    yield tap
    from hil.sniffer import Window
    census = Window(tap)
    census._offset = start
    try:
        sporadic, monitor = len(census.frames()), len(census.monitor_lines())
    except Exception:
        sporadic, monitor = -1, -1
    validity_state["wb"] = "reachable, tap live (%s); delivered %d bus frames, " \
        "%d monitor lines%s" % (
            hil_config.wb_ssh, sporadic, monitor,
            "  <-- JAMMED? see ISSUE-48, `systemctl restart wb-mqtt-dali`"
            if sporadic == 0 or monitor == 0 else "")
    tap.close()


@pytest.fixture(scope="session")
def gear_sim(request, hil_config):
    validity_state = _validity(request.config)
    try:
        sim = GearSim(hil_config.gear_sim_port)
    except GearSimUnavailable as exc:
        validity_state["gear_sim"] = "unavailable (%s) — no receiving-end " \
            "evidence about transaction integrity was collected" % exc
        pytest.skip("gear emulator unavailable: %s" % exc)
    validity_state["gear_sim"] = "console open (%s)" % hil_config.gear_sim_port
    yield sim
    sim.close()


@pytest.fixture()
def serial_log(hil_config):
    log = SerialLog(hil_config)
    if not log.monitor_alive:
        pytest.skip("serial monitor not running (start: hil monitor start) — "
                    "note: it must stay persistent, port open/close reboots the DUT")
    return log



def _validity(config):
    return config._hil_validity


def _track(config, obj):
    config._hil_counted.append(obj)
    return obj


def _standalone_client(config):
    client = getattr(config, "_hil_standalone_client", None)
    if client is None:
        client = _track(config, api_mod.Client(config_mod.load()))
        config._hil_standalone_client = client
    return client


@pytest.fixture(scope="session", autouse=True)
def production_state(pytestconfig, request):
    from hil import prod_state
    if pytestconfig.getoption("--collect-only") or \
            os.environ.get("HIL_STATE_GUARD", "1") in ("0", "false", "no") or \
            not any("api" in item.fixturenames for item in request.session.items):
        yield None
        return
    cfg = config_mod.load()
    try:
        api = api_mod.Client(cfg)
        api.health()
    except Exception as exc:
        print("\nproduction_state: could not reach the DUT (%s) — not guarded" % exc)
        yield None
        return
    state = _validity(pytestconfig)
    snap = prod_state.capture(api)
    last = prod_state.last_path(cfg)
    pending = prod_state.unrestored(last)
    if not pending:
        prod_state.save(dict(snap, session_open=True), last)
    prod_state.save(snap, request.getfixturevalue("run_dir") / "production_state_before.json")
    state["production_state"] = "captured %d devices at %s" % (
        len(snap["devices"]), snap["taken_at"])
    if pending:
        state["production_state_pending"] = str(last)
    try:
        yield snap
    finally:
        residual = prod_state.restore(api, snap, drive_lamps=not cfg.lamps_read_only,
                                      lamp_shorts=cfg.lamp_short_set())
        if not residual and not pending:
            prod_state.mark_restored(last, snap)
        state["production_state_residual"] = residual
        state["production_state"] += "; restored%s" % (
            " completely" if not residual else " with %d residual line(s)" % len(residual))
        absorbed = {k: v for k, v in api.retries.items() if v}
        if absorbed:
            state["production_state"] += "; the guard's own retries: %s" % ", ".join(
                "%s=%d" % kv for kv in sorted(absorbed.items()))


@pytest.fixture(scope="session", autouse=True)
def bench_baseline(pytestconfig, production_state):
    findings = []
    if pytestconfig.getoption("--collect-only"):
        yield
        return
    try:
        api = _standalone_client(pytestconfig)
        api.health()
    except Exception as exc:
        print("\nbench_baseline: could not reach the DUT (%s) — not enforcing" % exc)
        yield
        return

    findings.extend(_neutralize_poller(api))
    schedule_findings, suspended = _neutralize_schedules(api)
    findings.extend(schedule_findings)
    findings.extend(_neutralize_timezone(api))
    leaked = _leaked_names(api)
    _validity(pytestconfig)["observed"] = _observed_baseline(api)

    _validity(pytestconfig)["baseline"] = list(findings)
    for line in findings:
        print("\nbench_baseline: %s" % line)
    try:
        if leaked:
            _validity(pytestconfig)["baseline"].extend(leaked)
            pytest.fail(
                "the bench carries state leaked by a previous run:\n  %s\n"
                "These are persisted in flash and survive both a reboot and a "
                "reflash, so every measurement below them is of a bench nobody "
                "reset. Clear them, then re-run (ISSUE-31 п.4)."
                % "\n  ".join(leaked), pytrace=False)
        yield
    finally:
        _restore_schedules(api, suspended)


def _neutralize_poller(api):
    if not api.poller.get().get("enabled"):
        return []
    api.poller.patch({"enabled": False})
    if api.poller.get().get("enabled"):
        pytest.fail("bench_baseline could not disable the poller — every test "
                    "below would measure a bus under continuous read traffic",
                    pytrace=False)
    return ["found the poller ENABLED — disabled it (leftover from an earlier "
            "run; see ISSUE-30)"]


def _neutralize_schedules(api):
    enabled = [s["schedule_id"] for s in api.hcl.list() if s.get("enabled")]
    if not enabled:
        return [], []
    for schedule_id in enabled:
        api.hcl.patch(schedule_id, {"enabled": False})
    still_on = [s["schedule_id"] for s in api.hcl.list() if s.get("enabled")]
    if still_on:
        pytest.fail("bench_baseline could not disable schedules %s — the rig's "
                    "own schedules would drive the fixtures under test"
                    % still_on, pytrace=False)
    return (["found %d HCL schedule(s) ENABLED (%s) — suspended for the "
             "session, restored at the end"
             % (len(enabled), ", ".join(enabled))], enabled)


def _restore_schedules(api, suspended):
    for schedule_id in suspended:
        try:
            api.hcl.patch(schedule_id, {"enabled": True})
        except Exception as exc:
            print("\nbench_baseline: COULD NOT RE-ENABLE schedule %s (%s) — "
                  "re-enable it by hand" % (schedule_id, exc))


def _neutralize_timezone(api):
    found = api.time_get().get("timezone")
    if found == BENCH_BASELINE_TZ:
        return []
    api.time_set(timezone=BENCH_BASELINE_TZ)
    now = api.time_get().get("timezone")
    if now != BENCH_BASELINE_TZ:
        pytest.fail("bench_baseline could not set the zone to %s (still %r)"
                    % (BENCH_BASELINE_TZ, now), pytrace=False)
    leak = " — this is ANCHOR_TZ, i.e. a clock_guard that never reached its " \
           "finally" if found == ANCHOR_TZ else ""
    return ["found timezone %r, expected %r%s — restored"
            % (found, BENCH_BASELINE_TZ, leak)]


def _observed_baseline(api):
    schedules = api.hcl.list()
    return "poller %s, %d/%d HCL schedules enabled, tz %s" % (
        "on" if api.poller.get().get("enabled") else "off",
        sum(1 for s in schedules if s.get("enabled")), len(schedules),
        api.time_get().get("timezone"))


def _leaked_names(api):
    return ["short address %s is still named %r"
            % (d.get("short_address"), d.get("name"))
            for d in api.devices().get("physical_devices", [])
            if str(d.get("name") or "").startswith(LEAKED_NAME_PREFIX)]


@pytest.fixture()
def op_check(request):
    def check(view, retry=None):
        if view.get("status") == "succeeded":
            return view
        if not api_mod.op_contended(view):
            pytest.fail("operation failed: %r" % (view,), pytrace=False)
        _validity(request.config)["contended"].append(request.node.name)
        if retry is not None:
            view = retry()
            if view.get("status") == "succeeded":
                return view
            if not api_mod.op_contended(view):
                pytest.fail("operation failed on retry: %r" % (view,),
                            pytrace=False)
            _validity(request.config)["contended"].append(request.node.name)
        pytest.fail(
            "the operation never got the DALI bus (%s) — this is bench load, "
            "not a product failure: something else was driving the wire "
            "(the WB foreign master, or a leaked poller). Re-run alone to "
            "confirm; if it is reproducible, it is ours (ISSUE-31 п.5).\n%r"
            % (api_mod.BUS_CONTENDED, view), pytrace=False)
    return check


@pytest.fixture(autouse=True)
def dut_continuity(request):
    yield
    config = request.config
    if request.config.getoption("--collect-only"):
        return
    state = getattr(config, "_hil_uptime", None)
    if "dut_reboot" in request.fixturenames:
        config._hil_uptime = None
        return
    try:
        now, uptime = time.time(), float(_standalone_client(config)
                                         .health()["uptime_seconds"])
    except Exception as exc:
        config._hil_uptime = None
        if state is None:
            return
        breach = ("%s: the DUT stopped answering (%s) — continuity across it "
                  "is unverified" % (request.node.name, exc))
        _validity(config).setdefault("reboots", []).append(breach)
        pytest.fail(
            "the DUT became UNREACHABLE during this test — %s.\n"
            "A reboot presents exactly this way, and the probe's few seconds "
            "of retries cannot outwait a boot. Everything measured after this "
            "point may be against a freshly booted device: check whether "
            "anything touched %s mid-run (ISSUE-31 п.3)."
            % (breach, serialmon_mod.effective_port(config_mod.load())[0]),
            pytrace=False)
    config._hil_uptime = (now, uptime)
    shortfall = validity.uptime_broke(state, now, uptime, UPTIME_SLACK_S)
    if shortfall is None:
        return
    breach = ("%s: uptime went %.0fs -> %.0fs across a %.0fs test (short by %.0fs)"
              % (request.node.name, state[1], uptime, now - state[0], shortfall))
    _validity(config).setdefault("reboots", []).append(breach)
    pytest.fail(
        "the DUT REBOOTED during this test — %s.\n"
        "Everything measured here, and in every test after it, ran against a "
        "freshly booted device. Opening the serial port asserts reset: check "
        "whether anything touched %s mid-run (ISSUE-31 п.3)."
        % (breach, serialmon_mod.effective_port(config_mod.load())[0]),
        pytrace=False)



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
        help="bench prep: set every gear's fade time to 0 (instant) before the "
             "suite runs, so level/optical assertions settle inside their "
             "windows instead of racing a multi-second ramp. Persistent gear "
             "config — left in place after the run.")


def _optical_unavailable(request, reason):
    if request.config.getoption("--no-camera") \
            or os.environ.get("HIL_NO_CAMERA"):
        pytest.skip(reason)
    pytest.fail(
        "optical channel unavailable: %s\n"
        "(pass --no-camera or set HIL_NO_CAMERA=1 to explicitly accept an "
        "optical-less run)" % reason, pytrace=False)


_OPTICAL_FIXTURES = frozenset(("camera", "calibration", "geometry", "oracle"))


def _run_needs_optics(session):
    return any(_OPTICAL_FIXTURES.intersection(item.fixturenames)
               for item in session.items)


@pytest.fixture(scope="session", autouse=True)
def optical_session(hil_config, request):
    state = {"backend": None, "calibration": None, "error": None}
    opted_out = bool(request.config.getoption("--no-camera")
                     or os.environ.get("HIL_NO_CAMERA"))
    if opted_out or not _run_needs_optics(request.session):
        state["error"] = ("optical channel not requested for this run"
                          if not opted_out else
                          "optical channel opted out (--no-camera)")
        yield state
        return

    from hil.camera.backend import probe_and_select
    from hil.camera.calibrate import Calibrator
    from hil.camera.calibrate import load as load_cal
    try:
        state["backend"] = probe_and_select(hil_config)
        if not _skip_calibration(request):
            Calibrator(hil_config, request.getfixturevalue("api"),
                       state["backend"]).run()
        state["calibration"] = load_cal(hil_config)
    except Exception as exc:
        state["error"] = str(exc)
    yield state
    if state["backend"] is not None:
        state["backend"].close()


@pytest.fixture(scope="session")
def camera(optical_session, request):
    if optical_session["backend"] is None:
        _optical_unavailable(request, optical_session["error"] or "camera unavailable")
    return optical_session["backend"]


def _skip_calibration(request):
    return bool(request.config.getoption("--skip-calibration")
                or os.environ.get("HIL_SKIP_CALIBRATION"))


@pytest.fixture(scope="session")
def calibration(optical_session, camera, request):
    if optical_session["calibration"] is None:
        _optical_unavailable(
            request, optical_session["error"] or "calibration unavailable")
    return optical_session["calibration"]


@pytest.fixture(scope="session")
def geometry(hil_config, calibration):
    from hil.camera.masks import load_geometry
    return load_geometry(hil_config)


@pytest.fixture(scope="session")
def lamp_roster(optical_session, hil_config, request, api):
    if optical_session["calibration"] is not None:
        return optical_session["calibration"]["lamps"]
    if os.environ.get("HIL_LAMP_ROSTER") == "registry":
        bound = {(v.get("binding") or {}).get("physical_short_address"):
                 v["virtual_lamp_id"] for v in api.vlamps.list()["virtual_lamps"]}
        return [
            {"label": bound[d["short_address"]] + 1, "short_address": d["short_address"],
             "identity": {"gtin": d.get("gtin"),
                          "identification_number": d.get("identification_number")}}
            for d in api.devices()["physical_devices"]
            if d["short_address"] in bound and d.get("gtin") is not None
            and d.get("identification_number") is not None
        ]
    from hil.camera.calibrate import load as load_cal
    try:
        return load_cal(hil_config)["lamps"]
    except Exception as exc:
        _optical_unavailable(request, "lamp roster unavailable: %s" % exc)


@pytest.fixture(scope="session")
def lamps(api, lamp_roster, hil_config):
    from hil.identity import refresh_short_addresses
    mapping, missing = refresh_short_addresses(api, lamp_roster)
    allowed = hil_config.lamp_short_set()
    refused = {label: short for label, short in mapping.items() if short not in allowed}
    if refused:
        print("lamps: %s refused by HIL_LAMP_SHORTS=%s (owner's rule), not absent"
              % (sorted(refused.items()), hil_config.lamp_shorts))
    mapping = {label: short for label, short in mapping.items() if short in allowed}
    if not mapping:
        pytest.skip("none of the calibrated lamps are on the bus and permitted")

    class Lamps:
        by_label = mapping
        missing_labels = missing

        def short(self, label):
            if label not in mapping:
                pytest.skip("lamp %s (calibrated) not on the bus" % label)
            return mapping[label]

        def labels(self):
            return sorted(mapping)
    return Lamps()


@pytest.fixture(scope="session")
def _oracle_session(request, camera, calibration, geometry, hil_config):
    from hil.camera.calibrate import at_exposure_floor
    from hil.oracle import CameraOracle
    if at_exposure_floor(calibration):
        _validity(request.config)["camera_exposure"] = (
            "calibration measured at the exposure FLOOR (%s x100us) with the "
            "glow ring still clipping at level 254 — thresholds are usable "
            "(tests run at 120-140) but the rig has no headroom: dim it or move "
            "the camera, then `hil calibrate`"
            % calibration.get("exposure_time_abs"))
    return _track(request.config,
                  CameraOracle(camera, calibration, geometry, hil_config))


def _keep_calibration_fresh(oracle, hil_config, api, request):
    from hil.camera.calibrate import Calibrator, is_stale, age_seconds
    from hil.camera.calibrate import load as load_cal

    if _skip_calibration(request) or not is_stale(oracle.cal):
        return
    age_min = age_seconds(oracle.cal) / 60.0
    print("\ncalibration: profile %.0f min old — re-measuring before this test"
          % age_min)
    Calibrator(hil_config, api, oracle.backend).run()
    oracle.adopt_calibration(load_cal(hil_config))
    _validity(request.config).setdefault("recalibrations", []).append(
        "%s (profile was %.0f min old)" % (request.node.name, age_min))


@pytest.fixture()
def camera_oracle(_oracle_session, test_artifacts, api, request, hil_config):
    from hil.camera.backend import CameraError
    _oracle_session.artifacts = test_artifacts
    _keep_calibration_fresh(_oracle_session, hil_config, api, request)
    try:
        _oracle_session.fresh_baseline(api)
    except CameraError as exc:
        _optical_unavailable(request, "baseline capture: %s" % exc)
    return _oracle_session



@pytest.fixture(scope="session", autouse=True)
def _fast_fade_prep(request):
    if not request.config.getoption("--fast-fade"):
        yield
        return
    client = _track(request.config,
                    api_mod.Client(request.getfixturevalue("hil_config")))
    try:
        client.health()
        addrs = client.addrs()
    except Exception as exc:
        print("hil: --fast-fade skipped bench prep (DUT unavailable: %s)" % exc)
        yield
        return
    done, failed = [], []
    for short in addrs:
        try:
            view = client.wait_op(client.write_attrs(short, {"fade_time_ms": 0}))
            (done if view.get("status") == "succeeded" else failed).append(short)
        except Exception as exc:
            failed.append(short)
            print("hil: --fast-fade write failed for SA%02d: %s" % (short, exc))
    print("hil: --fast-fade set fade_time_ms=0 (instant) on %d/%d gear "
          "(persistent bench config)%s"
          % (len(done), len(addrs),
             "" if not failed else "; failed: %s" % failed))
    yield


@pytest.fixture(scope="session")
def scenes_supported(api):
    try:
        api.scenes.list()
    except api_mod.CapabilityUnsupported:
        pytest.skip("scenes surface absent on this firmware (pre-R6 build)")


@pytest.fixture(scope="session")
def vl_bindings(api, lamps, run_dir, hil_config):
    from hil.results import dumps as _dumps
    registry = os.environ.get("HIL_LAMP_ROSTER") == "registry"
    if registry:
        everyone = {d["short_address"] for d in api.devices_unfiltered()["physical_devices"]}
        if hil_config.lamps_read_only or not everyone <= hil_config.lamp_short_set():
            pytest.skip("vl_bindings drives lamps beyond the roster: on the owner's "
                        "wire only with HIL_LAMPS_READ_ONLY off and every lamp in "
                        "HIL_LAMP_SHORTS (HIL_LAMP_ROSTER=registry)")
    materialized = {v["virtual_lamp_id"]: v
                    for v in api.vlamps.list()["virtual_lamps"]}
    mapping = {}
    for label in lamps.labels():
        lid = label - 1
        short = lamps.by_label[label]
        have = (materialized.get(lid, {}).get("binding")
                or {}).get("physical_short_address")
        if have != short and registry:
            pytest.skip("VL%d is bound to %r, not SA%d — the owner's binding "
                        "is never rewritten (HIL_LAMP_ROSTER=registry)"
                        % (lid, have, short))
        if have != short:
            api.vlamps.bind(lid, short)
        mapping[lid] = short
    (run_dir / "bench_canon.json").write_text(
        _dumps({"vl_bindings": mapping}, indent=1, sort_keys=True))
    return mapping


def _teardown_write(fn):
    from requests import RequestException
    try:
        return fn()
    except RequestException:
        time.sleep(2.0)
        return fn()


@pytest.fixture()
def group_matrix_guard(api):
    before = api.groups.matrix()
    yield before
    rows = [{"virtual_lamp_id": r["virtual_lamp_id"], "desired": r["desired"]}
            for r in before["rows"]]
    if rows:
        _teardown_write(lambda: api.groups.matrix_patch(rows))
    res = _teardown_write(api.groups.apply)
    if "operation_id" in res:
        api.wait_op(res)


def _scene_desired_writeback(desired):
    out = {"included": desired["included"]}
    for key in ("power", "level", "color_mode", "color_temperature_kelvin",
                "xy", "rgb"):
        if desired.get(key) is not None:
            out[key] = desired[key]
    return out


@pytest.fixture()
def scene_matrix_guard(api):
    guarded = []

    def guard(scene_id):
        snap = api.scenes.matrix(scene_id)
        guarded.append((scene_id, snap))
        return snap
    yield guard
    for scene_id, before in reversed(guarded):
        rows = [{"virtual_lamp_id": r["virtual_lamp_id"],
                 "desired": _scene_desired_writeback(r["desired"])}
                for r in before["rows"]]
        if rows:
            _teardown_write(lambda: api.scenes.matrix_patch(scene_id, rows))
        res = _teardown_write(lambda: api.scenes.apply(scene_id))
        if "operation_id" in res:
            api.wait_op(res)


@pytest.fixture()
def binding_guard(api):
    guarded = []

    def guard(lamp_id):
        have = (api.vlamps.get(lamp_id).get("binding")
                or {}).get("physical_short_address")
        guarded.append((lamp_id, have))
        return have
    yield guard
    for lamp_id, want in reversed(guarded):
        have = (api.vlamps.get(lamp_id).get("binding")
                or {}).get("physical_short_address")
        if want is None and have is not None:
            api.vlamps.unbind(lamp_id)
        elif want is not None and want != have:
            api.vlamps.bind(lamp_id, want)


_ATTR_GUARD_SECTIONS = {
    "fade_time_ms": "common_102",
    "min_level": "common_102",
    "max_level": "common_102",
    "dimming_curve": "dt6_led",
}


@pytest.fixture()
def attr_guard(api):
    guarded = []

    def _section(short, name):
        return ((api.attributes(short, [name]).get("attributes") or {})
                .get(name) or {})

    def guard(short, *attrs, default=None, verify=False):
        for name in sorted({_ATTR_GUARD_SECTIONS[a] for a in attrs}):
            api.attr_read_checked(short, groups=name)
        sections = {name: _section(short, name)
                    for name in {_ATTR_GUARD_SECTIONS[a] for a in attrs}}
        values = {attr: (sections[_ATTR_GUARD_SECTIONS[attr]].get(attr)
                         or {}).get("value")
                  for attr in attrs}
        guarded.append((short, attrs, values, default, verify))
        return values

    yield guard
    for short, attrs, values, default, verify in reversed(guarded):
        for attr in attrs:
            want = values[attr] if values[attr] is not None else default
            if want is None:
                continue
            api.wait_op(api.write_attrs(short, {attr: want}))
            if verify:
                holds = (_section(short, _ATTR_GUARD_SECTIONS[attr]).get(attr)
                         or {}).get("value")
                assert holds == want, (
                    "FAILED TO RESTORE %s on gear %d: wanted %r, holds %r — "
                    "the gear is left holding test configuration, and every "
                    "later run measures it; fix before trusting anything else."
                    % (attr, short, want, holds))


@pytest.fixture()
def dut_reboot(api, hil_config, peer_api_if_any):
    from hil import remote_serial, serialmon

    def toggle():
        if remote_serial.enabled(hil_config):
            remote_serial.control(hil_config, "run")
            return
        serialmon.stop(hil_config)
        time.sleep(2.0)
        serialmon.start(hil_config)

    def reboot():
        with api.expect_reboot():
            return _reboot_and_wait(api, toggle)

    def _reboot_and_wait(api, toggle):
        t0 = time.monotonic()
        toggle()
        deadline = time.monotonic() + 120
        retoggled = False
        while time.monotonic() < deadline:
            try:
                health = api.health()
            except Exception:
                health = None
            if health and health.get("status") == "ok":
                if health.get("uptime_seconds", 10**9) < time.monotonic() - t0:
                    pair.hand_back(api, peer_api_if_any)
                    _wait_bus_ready(health)
                    return health
                if not retoggled and time.monotonic() - t0 > 30:
                    retoggled = True
                    toggle()
            time.sleep(1.0)
        raise AssertionError("DUT did not demonstrably reboot within 120s")

    def _wait_bus_ready(health, timeout_s=20.0):
        try:
            short = api.addrs()[0]
        except Exception:
            return

        def _answers():
            try:
                return bool(api.cmd(short, 0x90).get("success"))
            except Exception:
                return False

        wait_until(_answers, timeout_s, interval_s=1.0)
    return reboot


@pytest.fixture()
def rules_guard(api):
    doc = api.rules_get()
    if doc.get("diagnostic"):
        pytest.skip("the stored rules document does not compile (%s) — a test "
                    "rule appended to it would be refused for that reason"
                    % doc["diagnostic"])
    original = doc.get("source") or ""

    def _commit(source, what):
        view = api.wait_op(api.rules_put(source, api.rules_get()["revision"]))
        if view.get("status") != "succeeded":
            pytest.fail("rules %s did not commit: %r" % (what, view),
                        pytrace=False)
        return view

    def add(fragment):
        source = (original + "\n\n" + fragment) if original else fragment
        if len(source.encode()) > RULES_SOURCE_LIMIT_BYTES:
            pytest.skip(
                "the stored document plus a test rule exceeds the %d-byte "
                "source limit; this bench cannot add one without evicting the "
                "owner's" % RULES_SOURCE_LIMIT_BYTES)
        _commit(source, "append")

    try:
        yield add
    finally:
        if api.rules_get().get("source") != original:
            _commit(original, "restore")


@pytest.fixture()
def adapter_enabled_guard(api):
    yield
    api.adapter_patch({"enabled": True})


@pytest.fixture(scope="session")
def foreign(request, hil_config, api):
    from hil.foreign import ForeignMaster
    master = ForeignMaster(hil_config, api=api)
    try:
        master.probe()
    except Exception as exc:
        pytest.skip("WB foreign master (lunatone ws) unavailable: %s" % exc)
    request.config._hil_counted = getattr(request.config, "_hil_counted", [])
    request.config._hil_counted.append(master)
    return master


@pytest.fixture(scope="session")
def fanout_supported(api, foreign, lamps, wait_state):
    diag = api._req("GET", "diagnostics")
    if "sniffer_translator" not in diag:
        pytest.skip("pre-I1 firmware: /api/v1/diagnostics has no "
                    "sniffer_translator block")

    def counters():
        d = api._req("GET", "diagnostics")
        return (d.get("sniffer_translator", {}).get("observed_published", 0),
                d.get("projector", {}).get("runtime_updates_published", 0))

    label = lamps.labels()[0]
    short = lamps.by_label[label]
    api.off(short)
    before = counters()
    foreign.dapc(short, 77)
    last = wait_state(short, lambda s: s.get("level") == 77, timeout_s=5.0)
    after = counters()
    api.off(short)
    if last.get("level") != 77:
        raise AssertionError(
            "I1 is present but a foreign DAPC did not project: short %d level=%r "
            "source=%r; translator observed_published %d->%d, projector "
            "runtime_updates_published %d->%d"
            % (short, last.get("level"), last.get("last_dapc_source"),
               before[0], after[0], before[1], after[1]))


@pytest.fixture(scope="session")
def wait_state(api):
    def wait(short, pred, timeout_s=4.0, every_s=0.4):
        last = {}

        def _sample():
            nonlocal last
            last = api.state(short).get("state") or {}
            return pred(last)

        wait_until(_sample, timeout_s, interval_s=every_s)
        return last
    return wait


@pytest.fixture()
def free_group(api, capabilities):
    matrix = api.groups.matrix()
    used = set()
    for row in matrix["rows"]:
        for gid in range(16):
            if row["desired"][gid] or row["applied"][gid]:
                used.add(gid)
    for d in api.devices()["physical_devices"]:
        capabilities.ensure(d["short_address"], "groups")
    for d in api.devices()["physical_devices"]:
        bitmask = d.get("groups_membership")
        if bitmask:
            for gid in range(16):
                if bitmask & (1 << gid):
                    used.add(gid)
    for gid in range(15, -1, -1):
        if gid not in used:
            return gid
    pytest.skip("no free DALI group available on this rig")


@pytest.fixture()
def ops_quiesce(api):
    def wait(timeout_s=45.0):
        def _idle():
            for key in api.operations():
                if not key.startswith(("grp-apply", "scn-apply")):
                    continue
                status, view = api.raw_request("GET", "operations/%s" % key)
                if status == 200 and view.get("status") in ("accepted", "running"):
                    return False
            return True

        if not wait_until(_idle, timeout_s, interval_s=0.7):
            raise AssertionError(
                "apply operations still running after %.0fs" % timeout_s)
    wait()
    return wait


@pytest.fixture()
def poller_guard(api):
    before = api.poller.get()

    def set_(**fields):
        return api.poller.patch(fields)

    try:
        yield set_
    finally:
        api.poller.patch({k: before[k] for k in api_mod._PollerSettings.RESTORABLE
                          if k in before})


def _diag_counters(api, block):
    U32 = 1 << 32

    class Counters:
        def all(self):
            return api.diagnostics()[block]

        def get(self, name):
            return int(self.all().get(name, 0))

        @staticmethod
        def delta(before, after):
            return after - before if after >= before else after + U32 - before

        def wait(self, name, target, timeout_s=30.0, every_s=0.5):
            return bool(wait_until(lambda: self.get(name) >= target,
                                   timeout_s, interval_s=every_s))

        def quiet(self, name, seconds):
            start = self.get(name)
            return not wait_until(lambda: self.get(name) != start,
                                  seconds, interval_s=0.5)

    return Counters()


@pytest.fixture()
def poller_counters(api):
    return _diag_counters(api, "poller")


@pytest.fixture()
def mqtt_counters(api):
    return _diag_counters(api, "mqtt")


def namespace_controller_id():
    return "hiltest"


@pytest.fixture()
def ha_guard(api, hil_config):
    from hil import mqtt_tap

    before = api.ha.get()
    if before.get("controller_id") == namespace_controller_id():
        pytest.fail(
            "the bridge is already in the HIL test namespace (controller_id=%r,"
            " discovery_prefix=%r) — a previous run died before restoring it,"
            " and snapshotting this would make it permanent.\n"
            "Recover the live values first (they are readable from the broker's"
            " retained discovery configs: `mosquitto_sub -t 'homeassistant/#' -v`"
            " gives the discovery prefix, the state topic and the controller id)"
            " and PATCH them back to /api/v1/settings/home-assistant."
            % (before.get("controller_id"), before.get("discovery_prefix")),
            pytrace=False)
    namespace = {
        "enabled": True,
        "broker_host": hil_config.mqtt_broker_host,
        "broker_port": hil_config.mqtt_broker_port,
        "discovery_prefix": "hiltest",
        "state_topic_prefix": "hiltest-dali",
        "controller_id": namespace_controller_id(),
        "publish_qos": 1,
        "retain_state": True,
        "retain_discovery": True,
    }

    class Guard:
        retained_filters = ("hiltest/#", "hiltest-dali/#")

        def enter(self, **overrides):
            body = dict(namespace)
            body.update(overrides)
            return api.ha.patch(body)

        @staticmethod
        def topic(suffix):
            return "hiltest-dali/hiltest/%s" % suffix

        @staticmethod
        def config_topic(component, object_id):
            return "hiltest/%s/hiltest/%s/config" % (component, object_id)

    try:
        yield Guard()
    finally:
        for filt in Guard.retained_filters:
            try:
                for topic in mqtt_tap.collect_retained(hil_config, filt):
                    mqtt_tap.clear_retained(hil_config, topic)
            except Exception:
                pass
        api.ha.patch({k: before[k] for k in api_mod._HomeAssistantSettings.RESTORABLE})


@pytest.fixture()
def clock_guard(api):
    before = api.time_get()
    api.time_set(timezone=ANCHOR_TZ)

    def at(hour, minute, day_offset=0):
        day = (datetime.datetime.now(datetime.timezone.utc)
               + datetime.timedelta(days=day_offset)).date()
        target = datetime.datetime(day.year, day.month, day.day, hour, minute,
                                   tzinfo=datetime.timezone.utc)
        unix_ms = int(target.timestamp() * 1000)
        api.time_set(unix_ms=unix_ms)
        return unix_ms

    try:
        yield at
    finally:
        api.time_set(unix_ms=int(time.time() * 1000))
        if before.get("timezone"):
            api.time_set(timezone=before["timezone"])


@pytest.fixture()
def hcl_guard(api):
    created = []
    suspended = []
    schedules = api.hcl.list()
    for s in schedules:
        if not s.get("enabled"):
            continue
        suspended.append(s["schedule_id"])
        try:
            api.hcl.patch(s["schedule_id"], {"enabled": False})
        except Exception as exc:
            for done in suspended:
                try:
                    api.hcl.patch(done, {"enabled": True})
                except Exception:
                    pass
            pytest.fail(
                f"hcl_guard could not disable pre-existing schedule "
                f"{s['schedule_id']}: {exc}. The rig's own schedules would "
                f"drive the fixtures under test."
            )

    def track(schedule_id):
        created.append(schedule_id)
        return schedule_id

    try:
        yield track
    finally:
        for schedule_id in created:
            try:
                api.hcl.delete(schedule_id)
            except Exception:
                pass
        for schedule_id in suspended:
            try:
                api.hcl.patch(schedule_id, {"enabled": True})
            except Exception:
                pass



def pytest_runtest_setup(item):
    api_mod.Client.context = item.nodeid


def pytest_collection_modifyitems(config, items):
    cfg = config_mod.load()
    dut_serial_port, _ = serialmon_mod.effective_port(cfg)
    if "://" in dut_serial_port:
        try:
            remote_serial_mod.control(cfg, "ping")
            serial_absent = False
        except Exception:
            serial_absent = True
    else:
        serial_absent = not os.path.exists(dut_serial_port)

    allow_destructive = os.environ.get("HIL_ALLOW_DESTRUCTIVE") == "1"

    for item in items:
        if serial_absent and item.get_closest_marker("serial"):
            item.add_marker(pytest.mark.skip(
                reason="serial port %s unreachable — no board attached here and "
                       "no WB bridge answering" % dut_serial_port))
        if not allow_destructive and item.get_closest_marker("destructive"):
            item.add_marker(pytest.mark.skip(
                reason="destructive tier needs HIL_ALLOW_DESTRUCTIVE=1 "
                       "(run one test at a time; see tools/hil/README.md)"))
    def _tier(item):
        if item.get_closest_marker("destructive"):
            return 2 if "commissioning" in item.nodeid else 1
        return 0
    items.sort(key=_tier)


@pytest.fixture()
def needs_capability(request, capabilities):
    marker = request.node.get_closest_marker("needs_capability")
    if marker:
        cap = marker.args[0]
        if capabilities.any_lamp_with(cap) is None:
            pytest.skip("capability %r unsupported by this firmware/lamps" % cap)
    return capabilities



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
        results.append(("FAIL", name, str(report.longrepr)[:120]))


def pytest_terminal_summary(terminalreporter, exitstatus, config):
    results = getattr(config, "_hil_step_results", [])
    tw = terminalreporter
    if results:
        tw.section("HIL steps")
        for status, name, reason in results:
            line = "STEP %s %s" % (name, status)
            if reason:
                line += " — %s" % reason.replace("Skipped: ", "")
            tw.write_line(line)
    lines = getattr(config, "_hil_validity_report", None)
    if lines:
        tw.section("HIL validity")
        for line in lines:
            tw.write_line(line)


def _validity_state(config):
    from hil.camera import server as server_mod

    state = dict(getattr(config, "_hil_validity", {}) or {})
    cfg = config_mod.load()
    try:
        strays = server_mod.stray_pids()
        state["frame_server"] = server_mod.describe(cfg) + (
            "" if len(strays) <= 1 else
            "  *** %d PROCESSES (%s) — they share one mailbox and corrupt each "
            "other's frames ***" % (len(strays),
                                    ", ".join(str(p) for p in strays)))
    except Exception:
        state["frame_server"] = "unknown"
    api_retries, optical, fallbacks = collections.Counter(), collections.Counter(), 0
    retry_events = []
    for obj in getattr(config, "_hil_counted", []):
        if isinstance(obj, api_mod.Client):
            api_retries.update(obj.retries)
            retry_events.extend(getattr(obj, "retry_events", ()))
        elif hasattr(obj, "retry_causes"):
            optical.update(obj.retry_causes)
        elif hasattr(obj, "witness_fallbacks"):
            fallbacks += obj.witness_fallbacks
        elif hasattr(obj, "retries"):
            api_retries.update(obj.retries)
    state["stack_budget"] = validity.load_stack_budget()
    census_lines = _stack_census_lines(config)
    state["stack_min_free"] = validity.stack_min_free(census_lines)
    state["stack_observation_gaps"] = validity.stack_observation_gaps(census_lines)
    state["observed_task_states"] = validity.observed_task_states(census_lines)
    state["boot_heap_budget"] = validity.load_boot_heap_budget()
    state["boot_heap"] = _boot_heap_ladder(config)
    state["runtime_heap_budget"] = validity.load_runtime_heap_budget()
    state["runtime_heap"] = _runtime_heap(cfg)
    state["stack_identity"] = dict(validity.run_identity(cfg.runs_dir),
                                   devices=_registry_device_count(cfg))
    state["gear_segment"] = _gear_segment(cfg)
    state["peer"] = _peer_identity(cfg)
    state["bus_drops"] = _bus_drops(cfg, getattr(config, "_hil_isr_baseline", {}))
    state["bus_subscriber_losses"] = _bus_subscriber_losses(cfg)
    state["retry_events"] = retry_events
    state["ledger"] = validity.collect({
        "api": api_retries,
        "optical": optical,
        "witness_fallbacks": fallbacks,
        "bus_contended": len(state.get("contended") or []),
    })
    return state


def _peer_identity(cfg):
    if not cfg.has_peer:
        return "none (single controller)"
    peer = cfg.peer()
    try:
        version = api_mod.Client(peer).health().get("version")
    except Exception:
        version = "unreachable"
    identity = validity.run_identity(peer.runs_dir)
    return "%s version=%s (last flashed commit %s)" % (
        peer.base, version, (identity.get("commit") or "?")[:12])


def _gear_segment(cfg):
    wanted = cfg.gear_short_set()
    if wanted is None:
        return "whole registry (HIL_GEAR_SHORTS unset)"
    try:
        held = sorted(d["short_address"] for d in
                      api_mod.Client(cfg).devices_unfiltered()["physical_devices"])
    except Exception:
        held = None
    dropped = "unknown" if held is None else (
        ",".join(str(a) for a in held if a not in wanted) or "none")
    return ("RESTRICTED to %s by HIL_GEAR_SHORTS — not exercised: %s. "
            "A tier run this way is not an acceptance run."
            % (",".join(str(a) for a in sorted(wanted)), dropped))


def _registry_device_count(cfg):
    try:
        return len(api_mod.Client(cfg).devices_unfiltered()["physical_devices"])
    except Exception:
        return None


def _runtime_heap(cfg):
    try:
        return validity.runtime_heap_figures(api_mod.Client(cfg).stats())
    except Exception:
        return {}


def _bus_drops(cfg, isr_baseline=None):
    try:
        client = api_mod.Client(cfg)
        drops = validity.bus_drop_counters(client.diagnostics())
        stats = client.stats()
        timing = validity.isr_timing_counters(stats)
        baseline, baseline_uptime, baseline_host = isr_baseline or ({}, None, None)
        uptime = (stats.get("controller") or {}).get("uptime_ms")
        same_boot = validity.same_boot_interval(
            baseline_host, baseline_uptime, time.monotonic(), uptime,
            slack_s=UPTIME_SLACK_S)
        deltas = validity.counter_deltas(timing, baseline, same_boot=same_boot)
        drops.update(deltas)
        if same_boot and baseline_uptime is not None and uptime is not None:
            drops.update(validity.session_rates(deltas, int(uptime) - int(baseline_uptime)))
        drops.update(validity.absolute_counters(stats))
        return drops
    except Exception:
        return {}


def _bus_subscriber_losses(cfg):
    try:
        return validity.event_subscriber_losses(api_mod.Client(cfg).diagnostics())
    except Exception:
        return []


def _stack_census_lines(config):
    offset = getattr(config, "_hil_serial_offset", None)
    if offset is None:
        return []
    try:
        path = SerialLog(config_mod.load()).log_path
        if not path.exists():
            return []
        with open(path, errors="replace") as fh:
            fh.seek(offset)
            return fh.read().splitlines()
    except Exception:
        return []


def _boot_heap_ladder(config):
    offset = getattr(config, "_hil_serial_offset", None)
    if offset is None:
        return {}
    try:
        path = SerialLog(config_mod.load()).log_path
        if not path.exists():
            return {}
        with open(path, errors="replace") as fh:
            fh.seek(offset)
            return validity.boot_heap_ladder(fh.read().splitlines())
    except Exception:
        return {}


def _write_summary(config, lines):
    try:
        path = config_mod.load().run_dir() / "summary.md"
        body = ["# HIL run summary", "", "## Steps", ""]
        body += ["- STEP %s %s%s" % (name, status,
                                     (" — %s" % reason.replace("Skipped: ", ""))
                                     if reason else "")
                 for status, name, reason in getattr(config, "_hil_step_results", [])]
        body += ["", "## Validity", ""] + ["    " + line for line in lines] + [""]
        path.write_text("\n".join(body))
    except Exception as exc:
        print("could not write summary.md: %s" % exc)


def pytest_sessionfinish(session, exitstatus):
    config = session.config
    if config.getoption("--collect-only"):
        return
    state = _validity_state(config)
    budget = validity.load_budget()
    lines = validity.format_report(state, budget)
    over = validity.breaches(state.get("ledger") or {}, budget)
    if over:
        lines.append("")
        lines += ["OVER BUDGET: %s absorbed %d, budget %d" % row for row in over]
        lines.append("A budget is raised with a dated measurement in "
                     "tools/hil/retry_budget.txt, never to make a run green.")
        session.exitstatus = 1
    shed = validity.bus_drop_breaches(state, budget)
    if shed:
        lines.append("")
        lines += ["BUS DROPS OVER BUDGET: %s dropped %d, budget %d" % row for row in shed]
        lines.append("A frame the bus shed is a fact nobody will re-send. If it "
                     "carried an operation's terminal outcome the operation ends "
                     "by TTL reporting a timeout for work that finished "
                     "(ISSUE-50, ADR-021) — find the producer, do not raise "
                     "tools/hil/retry_budget.txt.")
        session.exitstatus = 1
    deep = validity.stack_breaches(state.get("stack_min_free") or {},
                                   state.get("stack_budget") or {})
    if deep:
        lines.append("")
        lines += ["STACK OVER BUDGET: %s used %d B, budget %d B" % row for row in deep]
        lines.append("A task deeper than its budget is one section away from a "
                     "Stack protection fault (ISSUE-49). Shorten the path the "
                     "probe names, or move the growth off the stack — raising "
                     "tools/hil/stack_budget.txt is not the fix.")
        session.exitstatus = 1
    dead = validity.stack_budget_dead_lines(state.get("stack_min_free") or {},
                                            state.get("stack_budget") or {})
    if dead:
        lines.append("")
        lines.append("STACK BUDGET LINE MATCHES NO TASK: %s" % ", ".join(dead))
        lines.append("The census prints the name the task registry holds, and a "
                     "budget key spelled any other way measures nothing. Spell the "
                     "key as `task stack hwm` prints it; a task the firmware spawns "
                     "only on demand belongs in ON_DEMAND_TASKS (hil/validity.py).")
        session.exitstatus = 1
    gaps = state.get("stack_observation_gaps") or []
    if gaps:
        lines.append("")
        lines.append("STACK OBSERVATION STALE/MISSING: %s" % ", ".join(gaps))
        lines.append("A callback-owned task must publish a fresh watermark; a "
                     "cached kernel handle is never dereferenced after exit.")
        session.exitstatus = 1
    pending = state.get("production_state_pending")
    if pending:
        lines.append("")
        lines.append("PRODUCTION STATE: an earlier session never restored %s — it "
                     "was kept, and this session only restored to what it found. "
                     "Run `hil state restore` to put the owner's installation back."
                     % pending)
        session.exitstatus = 1
    residual = state.get("production_state_residual") or []
    if residual:
        lines.append("")
        lines += ["PRODUCTION STATE NOT RESTORED: %s" % line for line in residual]
        lines.append("The session left the owner's installation different from "
                     "how it found it. The snapshot is tools/hil/state/"
                     "production_state_last.json; `hil state restore` retries.")
        session.exitstatus = 1
    eroded = validity.boot_heap_breaches(state.get("boot_heap") or {},
                                        state.get("boot_heap_budget") or {})
    if eroded:
        lines.append("")
        lines += ["BOOT BASELINE BELOW FLOOR: %s left %d B free, floor %d B" % row
                  for row in eroded]
        lines.append("This release leaves less internal SRAM standing at boot "
                     "than the last one did, and every later dip starts from "
                     "that floor. The failure at the bottom is ESP_ERR_HTTPD_TASK "
                     "and a boot loop. Find what the stage named above now "
                     "allocates; lowering tools/hil/boot_heap_budget.txt needs a "
                     "dated reason.")
        session.exitstatus = 1
    thin = validity.runtime_heap_breaches(state.get("runtime_heap") or {},
                                          state.get("runtime_heap_budget") or {})
    if thin:
        lines.append("")
        lines += ["RUNTIME HEAP BELOW FLOOR: %s read %d B, floor %d B" % row
                  for row in thin]
        lines.append("Internal SRAM ran lower during this boot than the floor "
                     "allows. What needs internal SRAM at that moment — a new "
                     "task stack, httpd, DMA, TLS — fails. The serial line "
                     "`internal SRAM low-water` names the moment; lowering "
                     "tools/hil/runtime_heap_budget.txt needs a dated reason.")
        session.exitstatus = 1
    config._hil_validity_report = lines
    _write_summary(config, lines)



@pytest.fixture(scope="session")
def paced():
    last = [0.0]

    def wait(seconds=1.0):
        delta = time.time() - last[0]
        if delta < seconds:
            time.sleep(seconds - delta)
        last[0] = time.time()
    return wait


def pytest_configure(config):
    config.addinivalue_line("markers", "needs_capability(name): auto-skip when absent")
    config._hil_step_results = []
    config._hil_validity = {"baseline": [], "reboots": [], "contended": []}
    config._hil_uptime = None
    config._hil_counted = []
    try:
        cfg = config_mod.load()
        stats = api_mod.Client(cfg).stats()
        config._hil_isr_baseline = (
            validity.isr_timing_counters(stats),
            (stats.get("controller") or {}).get("uptime_ms"),
            time.monotonic(),
        )
    except Exception:
        config._hil_isr_baseline = ({}, None, None)
    try:
        path = SerialLog(config_mod.load()).log_path
        config._hil_serial_offset = path.stat().st_size if path.exists() else 0
    except Exception:
        config._hil_serial_offset = None
