import subprocess
import time

import pytest

from hil import api as api_mod
from hil import config as config_mod
from hil import pair
from hil.artifacts import Artifacts
from hil.gearsim import GearSim, GearSimUnavailable
from hil.seriallog import SerialLog
from hil.sniffer import SnifferTap, ssh_argv
from hil.wait import wait_until
from hil_harness import track, validity_of

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
    client = track(request.config, api_mod.Client(hil_config))
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
    client = track(request.config, api_mod.Client(peer_config))
    try:
        client.health()
    except Exception as exc:
        pytest.skip("peer unreachable at %s: %s" % (peer_config.base, exc))
    return client


@pytest.fixture(scope="session")
def peer_api_if_any(request, hil_config):
    if not hil_config.has_peer:
        return None
    return track(request.config, api_mod.Client(hil_config.peer()))


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


@pytest.fixture(scope="session")
def sniffer(request, hil_config, run_dir):
    validity_state = validity_of(request.config)
    probe = subprocess.run(ssh_argv(hil_config, "true"),
                           capture_output=True, timeout=15)
    if probe.returncode != 0:
        validity_state["wb"] = "UNREACHABLE over ssh (%s) — every sniffer " \
            "test skipped, so no wire evidence was collected" % hil_config.wb_ssh
        pytest.skip("WB sniffer host %s unreachable over ssh" % hil_config.wb_ssh)
    tap = track(request.config,
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
    validity_state = validity_of(request.config)
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


@pytest.fixture()
def op_check(request):
    def check(view, retry=None):
        if view.get("status") == "succeeded":
            return view
        if not api_mod.op_contended(view):
            pytest.fail("operation failed: %r" % (view,), pytrace=False)
        validity_of(request.config)["contended"].append(request.node.name)
        if retry is not None:
            view = retry()
            if view.get("status") == "succeeded":
                return view
            if not api_mod.op_contended(view):
                pytest.fail("operation failed on retry: %r" % (view,),
                            pytrace=False)
            validity_of(request.config)["contended"].append(request.node.name)
        pytest.fail(
            "the operation never got the DALI bus (%s) — this is bench load, "
            "not a product failure: something else was driving the wire "
            "(the WB foreign master, or a leaked poller). Re-run alone to "
            "confirm; if it is reproducible, it is ours (ISSUE-31 п.5).\n%r"
            % (api_mod.BUS_CONTENDED, view), pytrace=False)
    return check


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


@pytest.fixture()
def needs_capability(request, capabilities):
    marker = request.node.get_closest_marker("needs_capability")
    if marker:
        cap = marker.args[0]
        if capabilities.any_lamp_with(cap) is None:
            pytest.skip("capability %r unsupported by this firmware/lamps" % cap)
    return capabilities


@pytest.fixture(scope="session")
def paced():
    last = [0.0]

    def wait(seconds=1.0):
        delta = time.time() - last[0]
        if delta < seconds:
            time.sleep(seconds - delta)
        last[0] = time.time()
    return wait
