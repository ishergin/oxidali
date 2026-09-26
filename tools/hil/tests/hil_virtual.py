import os
from dataclasses import dataclass

import pytest
import requests

from hil import api as api_mod
from hil import config as config_mod
from hil import serialmon, tripwire, virtual_gear
from hil.config import _parse_shorts
from hil.gearsim import GearOracle, GearSim, GearSimUnavailable
from hil.lamp_guard import VirtualFence
from hil.seriallog import LogWindow
from hil_harness import validity_of

VIRTUAL_ENV = "HIL_VIRTUAL_GEAR"
COMMISSIONING_ENV = "HIL_ALLOW_VIRTUAL_COMMISSIONING"
OWNER_SHORTS_ENV = "HIL_OWNER_SHORTS"
SHORT_ENVS = ("HIL_LAMP_SHORTS", "HIL_GEAR_SHORTS", "HIL_OPTICAL_SHORTS")
MARKER = "virtual_gear"
EXCLUSIVE_MARKERS = ("redundancy",)
EXIT_SETUP, EXIT_SAFETY, EXIT_PEER_RETURNED = 3, 4, 5
PEER_PROBE_TIMEOUT_S = 2


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
        os.environ[name] = virtual_gear.spell(park)
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
    admin, peer = api_mod.Client(cfg), cfg.peer()
    try:
        sim = GearSim(peer)
    except GearSimUnavailable as exc:
        pytest.exit("virtual gear: %s" % exc, returncode=EXIT_SETUP)
    session = virtual_gear.VirtualSession(cfg, admin, peer, sim, owner_shorts())
    state = validity_of(request.config)
    try:
        opened = session.open()
    except Exception as exc:
        residual = session.close()
        state["virtual_gear_residual"] = residual
        pytest.exit("virtual gear: %s" % exc, returncode=EXIT_SETUP)
    state["virtual_gear"] = "park SA%s, groups %s, reserve %s, VLs %s" % (
        virtual_gear.spell(opened["park"]), opened["groups"],
        virtual_gear.spell(opened["reserve"]), sorted(opened["vl_of_short"].values()))
    try:
        yield VirtualBench(opened["park"], opened["groups"], opened["vl_of_short"], sim,
                           GearOracle(sim))
    finally:
        stats = _safe(sim.stats)
        state["virtual_gear_answers"] = {k: stats.get(k) for k in
                                         ("sent", "stale", "expired", "late", "late_ticks",
                                          "log_dropped")} if stats else "unread"
        state["virtual_gear_residual"] = session.close()


@pytest.fixture(scope="session", autouse=True)
def virtual_gear_fence(request, virtual_gear_session, production_state):
    if virtual_gear_session is None:
        return None
    api = request.getfixturevalue("api")
    snap = production_state or {}
    api.guard.fence = VirtualFence(
        virtual_gear_session.park, virtual_gear_session.groups,
        virtual_gear_session.vl_of_short.values(),
        owner_rules=(snap.get("rules") or {}).get("source", ""),
        owner_schedules=[s.get("schedule_id") for s in snap.get("hcl") or []],
        commissioning=commissioning_allowed(), pending=_pending(api))
    return api.guard.fence


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
def virtual_tripwire(request, virtual_gear_session):
    if virtual_gear_session is None:
        yield
        return
    cfg = config_mod.load()
    admin = api_mod.Client(cfg)
    before = tripwire.log_losses(admin.stats())
    with LogWindow(serialmon.log_path(cfg)) as window:
        yield
        found = tripwire.violations(window.lines(), virtual_gear_session.park,
                                    virtual_gear_session.groups)
    lost = tripwire.lost_lines(before, tripwire.log_losses(admin.stats()))
    state = validity_of(request.config)
    if lost:
        state.setdefault("virtual_gear_inconclusive", []).append(
            "%s: the DUT lost log lines %s, so the tripwire cannot vouch" % (
                request.node.nodeid, lost))
    if found:
        state.setdefault("virtual_gear_safety", []).extend(
            "%s: %s" % (request.node.nodeid, v) for v in found)
        pytest.exit("SAFETY: %s" % found[0], returncode=EXIT_SAFETY)
    if _peer_answers_http(cfg):
        state.setdefault("virtual_gear_inconclusive", []).append(
            "%s: the peer answers HTTP again — it left the emulator role" % request.node.nodeid)
        pytest.exit("the peer returned to the controller mid-session: inconclusive",
                    returncode=EXIT_PEER_RETURNED)


def _peer_answers_http(cfg):
    try:
        base = cfg.peer().base
        return bool(base) and requests.get(base.rstrip("/") + "/api/v1/health",
                                           timeout=PEER_PROBE_TIMEOUT_S).ok
    except Exception:
        return False


def _safe(action):
    try:
        return action()
    except Exception:
        return None
