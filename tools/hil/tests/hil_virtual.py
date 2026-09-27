import os
import time
from dataclasses import dataclass

import pytest

from hil import api as api_mod
from hil import config as config_mod
from hil import role, serialmon, tripwire, virtual_gear
from hil.config import _parse_shorts
from hil.gearsim import GearOracle, GearSim, GearSimUnavailable
from hil.lamp_guard import VirtualFence, spell
from hil.seriallog import LogWindow
from hil.wait import wait_until
from hil_harness import validity_of

VIRTUAL_ENV = "HIL_VIRTUAL_GEAR"
COMMISSIONING_ENV = "HIL_ALLOW_VIRTUAL_COMMISSIONING"
OWNER_SHORTS_ENV = "HIL_OWNER_SHORTS"
SHORT_ENVS = ("HIL_LAMP_SHORTS", "HIL_GEAR_SHORTS", "HIL_OPTICAL_SHORTS")
MARKER = "virtual_gear"
EXCLUSIVE_MARKERS = ("redundancy",)
EXIT_SETUP, EXIT_SAFETY, EXIT_PEER_RETURNED, EXIT_BLIND = 3, 4, 5, 6
QUIET_S = 1.5
SETTLE_MAX_S = 10.0
BARRIER_TIMEOUT_S = 10.0
POLL_S = 0.2
SAFETY_NOTE = "the owner's own use of the lights during a test trips it too"


def enabled():
    return os.environ.get(VIRTUAL_ENV) == "1"


def commissioning_allowed():
    return os.environ.get(COMMISSIONING_ENV) == "1"


def owner_shorts():
    spec = os.environ.get(OWNER_SHORTS_ENV, "").strip()
    return _parse_shorts(spec, OWNER_SHORTS_ENV) if spec else frozenset()


@dataclass
class VirtualBench:
    park: list
    groups: list
    vl_of_short: dict
    sim: GearSim
    oracle: GearOracle

    def vl(self, short):
        return self.vl_of_short[str(short)]


def pytest_configure(config):
    config.addinivalue_line("markers", "%s: runs on the gear the peer emulates" % MARKER)
    if not enabled() or config.getoption("--collect-only"):
        return
    cfg = config_mod.load()
    api = api_mod.Client(cfg)
    reserved = virtual_gear.reserve(virtual_gear.registry_shorts(api),
                                    virtual_gear.wb_shorts(cfg), owner_shorts())
    park = virtual_gear.park_shorts(reserved, sum(virtual_gear.DEFAULT_PARK))
    for name in SHORT_ENVS:
        os.environ[name] = spell(park)
    config._hil_virtual_plan = {"reserve": sorted(reserved), "park": park}


def pytest_collection_modifyitems(config, items):
    if not enabled():
        return
    kept, dropped = [], []
    for item in items:
        if item.get_closest_marker(MARKER) is None:
            dropped.append(item)
            continue
        clash = [m for m in EXCLUSIVE_MARKERS if item.get_closest_marker(m)]
        if clash:
            raise pytest.UsageError("%s is marked %s and %s: the peer cannot be both"
                                    % (item.nodeid, MARKER, clash[0]))
        kept.append(item)
    if dropped:
        config.hook.pytest_deselected(items=dropped)
        items[:] = kept


@pytest.fixture(scope="session", autouse=True)
def virtual_gear_session(request, production_state, bench_baseline):
    if not enabled() or request.config.getoption("--collect-only"):
        yield None
        return
    cfg = config_mod.load()
    if production_state is None:
        pytest.exit("virtual gear: no installation snapshot was taken (HIL_STATE_GUARD=0 or "
                    "the DUT was unreachable), so nothing could restore it",
                    returncode=EXIT_SETUP)
    ledger = virtual_gear.Ledger.of(cfg)
    if ledger.exists():
        pytest.exit("virtual gear: a previous session left %s — `hil state restore` finishes "
                    "its teardown first" % ledger.path, returncode=EXIT_SETUP)
    session, opened, state = _open_session(request, cfg)
    try:
        yield VirtualBench(opened["park"], opened["groups"], opened["vl_of_short"],
                           session.sim, GearOracle(session.sim))
    finally:
        stats = _safe(session.sim.stats)
        state["virtual_gear_answers"] = {k: stats.get(k) for k in
                                         ("sent", "stale", "expired", "late", "late_ticks",
                                          "log_dropped")} if stats else "unread"
        state["virtual_gear_residual"] = session.close()


def _open_session(request, cfg):
    try:
        sim = GearSim(cfg.peer())
    except GearSimUnavailable as exc:
        pytest.exit("virtual gear: %s" % exc, returncode=EXIT_SETUP)
    session = virtual_gear.VirtualSession(cfg, api_mod.Client(cfg), sim, owner_shorts())
    state = validity_of(request.config)
    try:
        opened = session.open()
    except Exception as exc:
        state["virtual_gear_residual"] = session.close()
        pytest.exit("virtual gear: %s" % exc, returncode=EXIT_SETUP)
    state["virtual_gear"] = "park SA%s, groups %s, reserve %s, VLs %s" % (
        spell(opened["park"]), spell(opened["groups"]), spell(opened["reserve"]),
        spell(opened["vl_of_short"].values()))
    return session, opened, state


@pytest.fixture(scope="session", autouse=True)
def virtual_gear_fence(request, virtual_gear_session):
    if virtual_gear_session is None:
        yield None
        return
    api = request.getfixturevalue("api")
    bench = virtual_gear_session
    api.guard.fence = VirtualFence(bench.park, bench.groups, bench.vl_of_short.values(),
                                   commissioning=commissioning_allowed(), pending=_pending(api))
    try:
        yield api.guard.fence
    finally:
        api.guard.fence = None


def _pending(api):
    def pending(kind, scene):
        rows = api.groups.matrix().get("rows", []) if kind == "group" \
            else api.scenes.matrix(scene).get("rows", [])
        return {r["virtual_lamp_id"] for r in rows if r.get("desired") != r.get("applied")}
    return pending


@pytest.fixture()
def virtual_bench(virtual_gear_session):
    if virtual_gear_session is None:
        pytest.skip("not a virtual-gear session (%s=1 and `hil --peer role gear-sim`)"
                    % VIRTUAL_ENV)
    return virtual_gear_session


@pytest.fixture(autouse=True)
def virtual_tripwire(request, virtual_gear_session, virtual_gear_fence):
    if virtual_gear_session is None:
        yield
        return
    cfg = config_mod.load()
    admin = api_mod.Client(cfg)
    before = tripwire.log_losses(admin.stats())
    with LogWindow(serialmon.log_path(cfg)) as window:
        yield
        flushed = _flush(window, admin, virtual_gear_session.park[0])
        found = tripwire.violations(window.lines(), virtual_gear_fence)
    _judge(request, cfg, admin, before, flushed, found)


def _flush(window, admin, short):
    _await_quiet(window)
    seen = tripwire.barrier_count(window.lines(), short)
    admin.cmd(short, tripwire.QUERY_STATUS)
    return bool(wait_until(lambda: tripwire.barrier_count(window.lines(), short) > seen,
                           BARRIER_TIMEOUT_S, POLL_S))


def _await_quiet(window):
    deadline = time.monotonic() + SETTLE_MAX_S
    count, still_since = -1, time.monotonic()
    while time.monotonic() < deadline:
        now = len(tripwire.sent_frames(window.lines()))
        if now != count:
            count, still_since = now, time.monotonic()
        elif time.monotonic() - still_since >= QUIET_S:
            return
        time.sleep(POLL_S)


def _judge(request, cfg, admin, before, flushed, found):
    state, node = validity_of(request.config), request.node.nodeid
    lost = tripwire.lost_lines(before, tripwire.log_losses(admin.stats()))
    if lost:
        state.setdefault("virtual_gear_inconclusive", []).append(
            "%s: the DUT lost log lines %s, so the tripwire cannot vouch" % (node, lost))
    if found:
        state.setdefault("virtual_gear_safety", []).extend("%s: %s" % (node, v) for v in found)
        pytest.exit("SAFETY: %s (%s)" % (found[0], SAFETY_NOTE), returncode=EXIT_SAFETY)
    if not flushed:
        state.setdefault("virtual_gear_inconclusive", []).append(
            "%s: the barrier frame never reached the DUT's log (monitor %s): the tripwire is "
            "blind" % (node, "alive" if serialmon.alive(cfg) else "down"))
        pytest.exit("the tripwire went blind: inconclusive", returncode=EXIT_BLIND)
    if role.controller_health(cfg.peer()) is not None:
        state.setdefault("virtual_gear_inconclusive", []).append(
            "%s: the peer answers HTTP again — it left the emulator role" % node)
        pytest.exit("the peer returned to the controller mid-session: inconclusive",
                    returncode=EXIT_PEER_RETURNED)


def _safe(action):
    try:
        return action()
    except Exception:
        return None
