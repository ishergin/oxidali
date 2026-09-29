import os
from dataclasses import dataclass

import pytest
import requests

from hil import api as api_mod
from hil import config as config_mod
from hil import role, serialmon, tripwire, virtual_gear
from hil.config import PeerUnconfigured, _parse_shorts
from hil.gearsim import GearOracle, GearSim, GearSimUnavailable
from hil.lamp_guard import RulesBaseline, VirtualFence, spell
from hil.seriallog import LogWindow
from hil.wait import wait_until
from hil_harness import validity_of

VIRTUAL_ENV = virtual_gear.VIRTUAL_ENV
COMMISSIONING_ENV = "HIL_ALLOW_VIRTUAL_COMMISSIONING"
OWNER_SHORTS_ENV = "HIL_OWNER_SHORTS"
PARK_ENV = "HIL_VIRTUAL_PARK"
SHORT_ENVS = ("HIL_LAMP_SHORTS", "HIL_GEAR_SHORTS", "HIL_OPTICAL_SHORTS")
MARKER = "virtual_gear"
EXCLUSIVE_MARKERS = ("redundancy",)
EXIT_SETUP, EXIT_SAFETY = virtual_gear.EXIT_SETUP, virtual_gear.EXIT_SAFETY
EXIT_PEER_RETURNED, EXIT_BLIND = virtual_gear.EXIT_PEER_RETURNED, virtual_gear.EXIT_BLIND
QUIET_S = 1.5
SETTLE_MAX_S = 10.0
BARRIER_TIMEOUT_S = 10.0
POLL_S = 0.2
SAFETY_NOTE = "the owner's own use of the lights during a test trips it too"
RESTORE_APPLY_S = 90


def enabled():
    return virtual_gear.run_enabled()


def commissioning_allowed():
    return os.environ.get(COMMISSIONING_ENV) == "1"


def owner_shorts():
    spec = os.environ.get(OWNER_SHORTS_ENV, "").strip()
    return _parse_shorts(spec, OWNER_SHORTS_ENV) if spec else frozenset()


def park_shape():
    spec = os.environ.get(PARK_ENV, "").strip()
    if not spec:
        return virtual_gear.DEFAULT_PARK
    try:
        return virtual_gear.parse_park(spec)
    except virtual_gear.VirtualGearError as exc:
        raise pytest.UsageError("%s: %s" % (PARK_ENV, exc))


@dataclass
class VirtualBench:
    park: list
    groups: list
    vl_of_short: dict
    shape: tuple
    sim: GearSim
    oracle: GearOracle

    def vl(self, short):
        return self.vl_of_short[str(short)]

    def of_kind(self, kind):
        return virtual_gear.park_of_kind(self.park, self.shape, kind)


def pytest_configure(config):
    config.addinivalue_line("markers", "%s: runs on the gear the peer emulates" % MARKER)
    if not enabled() or config.getoption("--collect-only"):
        return
    cfg = config_mod.load()
    api = api_mod.Client(cfg)
    shape = park_shape()
    reserved = virtual_gear.reserve(virtual_gear.registry_shorts(api),
                                    virtual_gear.wb_shorts(cfg), owner_shorts())
    park = virtual_gear.park_shorts(reserved, sum(shape))
    for name in SHORT_ENVS:
        os.environ[name] = spell(park)
    config._hil_virtual_plan = {"reserve": sorted(reserved), "park": park, "shape": shape}


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
    admin = api_mod.Client(cfg)
    losses = tripwire.log_losses(admin.stats())
    with LogWindow(serialmon.log_path(cfg)) as whole:
        try:
            yield VirtualBench(opened["park"], opened["groups"], opened["vl_of_short"],
                               tuple(opened["shape"]), session.sim, GearOracle(session.sim))
        finally:
            _sweep_with(whole, admin, opened, state, losses)
            _close_session(session, state)


def _sweep_with(whole, admin, opened, state, losses):
    fence = VirtualFence(opened["park"], opened["groups"], opened["vl_of_short"].values(),
                         commissioning=commissioning_allowed())
    if not _flush(whole, admin, opened["park"][0]):
        state.setdefault("virtual_gear_inconclusive", []).append(
            "session: the final barrier never reached the DUT's log, so frames after the "
            "last test went unjudged")
    lost = tripwire.lost_lines(losses, tripwire.log_losses(admin.stats()))
    if lost:
        state.setdefault("virtual_gear_inconclusive", []).append(
            "session: the DUT lost log lines %s, so the sweep cannot vouch" % lost)
    seen = {line.split(": ", 1)[-1] for line in state.get("virtual_gear_safety", [])}
    fresh = [v for v in tripwire.violations(whole.lines(), fence) if v not in seen]
    if fresh:
        state.setdefault("virtual_gear_safety", []).extend("session: %s" % v for v in fresh)


def _close_session(session, state):
    stats = _safe(session.sim.stats)
    state["virtual_gear_answers"] = {k: stats.get(k) for k in
                                     ("sent", "stale", "expired", "late", "late_ticks",
                                      "log_dropped")} if stats else "unread"
    state["virtual_gear_residual"] = session.close()


def _open_session(request, cfg):
    try:
        sim = GearSim(cfg.peer())
    except (GearSimUnavailable, PeerUnconfigured) as exc:
        pytest.exit("virtual gear: %s" % exc, returncode=EXIT_SETUP)
    session = virtual_gear.VirtualSession(cfg, api_mod.Client(cfg), sim, owner_shorts(),
                                          park=park_shape())
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
                                   commissioning=commissioning_allowed(),
                                   pending=api.pending_lamps, rules=_rules_baseline(api))
    try:
        yield api.guard.fence
    finally:
        api.guard.fence = None


def _rules_baseline(api):
    return RulesBaseline(api.rules_get().get("source") or "", api.rules_toggles())


@pytest.fixture()
def virtual_bench(virtual_gear_session):
    if virtual_gear_session is None:
        pytest.skip("not a virtual-gear session (%s=1 and `hil --peer role gear-sim`)"
                    % VIRTUAL_ENV)
    return virtual_gear_session


def session_rows(matrix, lamp_ids):
    return {row["virtual_lamp_id"]: row for row in matrix.get("rows", [])
            if row["virtual_lamp_id"] in set(lamp_ids)}


def rows_to_restore(before, now):
    return [{"virtual_lamp_id": lamp_id, "desired": row["desired"]}
            for lamp_id, row in sorted(before.items()) if lamp_id in now
            and (now[lamp_id]["desired"], now[lamp_id]["applied"])
            != (row["desired"], row["applied"])]


@pytest.fixture()
def session_rows_guard(api, virtual_bench):
    lamp_ids = list(virtual_bench.vl_of_short.values())
    before = session_rows(api.groups.matrix(), lamp_ids)
    yield before
    rows = rows_to_restore(before, session_rows(api.groups.matrix(), lamp_ids))
    if rows:
        api.groups.matrix_patch(rows)
        applied = api.groups.apply()
        if "operation_id" in applied:
            api.wait_op(applied, timeout_s=RESTORE_APPLY_S)


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
    try:
        admin.cmd(short, tripwire.QUERY_STATUS)
    except (api_mod.ApiError, requests.RequestException):
        return False
    return bool(wait_until(lambda: tripwire.barrier_count(window.lines(), short) > seen,
                           BARRIER_TIMEOUT_S, POLL_S))


def _await_quiet(window):
    tripwire.settled_frames(window, QUIET_S, SETTLE_MAX_S, POLL_S)


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
