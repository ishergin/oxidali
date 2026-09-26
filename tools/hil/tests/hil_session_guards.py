import os
import time

import pytest

from hil import api as api_mod
from hil import config as config_mod
from hil import prod_state
from hil import serialmon as serialmon_mod
from hil import validity
from hil_harness import ANCHOR_TZ, UPTIME_SLACK_S, track, validity_of

BENCH_BASELINE_TZ = os.environ.get("HIL_BENCH_TZ", "MSK-3")


LEAKED_NAME_PREFIX = "hil-"


def _hardware_free(config):
    return getattr(config, "_hil_hardware_free", False)


def _standalone_client(config):
    client = getattr(config, "_hil_standalone_client", None)
    if client is None:
        client = track(config, api_mod.Client(config_mod.load()))
        config._hil_standalone_client = client
    return client


@pytest.fixture(scope="session", autouse=True)
def production_state(pytestconfig, request):
    if pytestconfig.getoption("--collect-only") or _hardware_free(pytestconfig) or \
            not prod_state.guard_enabled() or \
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
    state = validity_of(pytestconfig)
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
    if pytestconfig.getoption("--collect-only") or _hardware_free(pytestconfig):
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
    validity_of(pytestconfig)["observed"] = _observed_baseline(api)

    validity_of(pytestconfig)["baseline"] = list(findings)
    for line in findings:
        print("\nbench_baseline: %s" % line)
    try:
        if leaked:
            validity_of(pytestconfig)["baseline"].extend(leaked)
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


@pytest.fixture(autouse=True)
def dut_continuity(request):
    yield
    config = request.config
    if request.config.getoption("--collect-only") or _hardware_free(config):
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
        validity_of(config).setdefault("reboots", []).append(breach)
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
    validity_of(config).setdefault("reboots", []).append(breach)
    pytest.fail(
        "the DUT REBOOTED during this test — %s.\n"
        "Everything measured here, and in every test after it, ran against a "
        "freshly booted device. Opening the serial port asserts reset: check "
        "whether anything touched %s mid-run (ISSUE-31 п.3)."
        % (breach, serialmon_mod.effective_port(config_mod.load())[0]),
        pytrace=False)


@pytest.fixture(scope="session", autouse=True)
def _fast_fade_prep(request, production_state):
    if not request.config.getoption("--fast-fade"):
        yield
        return
    if production_state is None:
        print("hil: --fast-fade skipped: no production_state snapshot in this "
              "session, so nothing would restore the fade time")
        yield
        return
    client = track(request.config,
                    api_mod.Client(request.getfixturevalue("hil_config")))
    try:
        shorts = client.lamp_addrs()
    except Exception as exc:
        print("hil: --fast-fade skipped bench prep (DUT unavailable: %s)" % exc)
        yield
        return
    prod_state.fast_fade(client, shorts)
    yield
