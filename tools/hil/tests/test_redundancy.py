import time

import pytest

from hil import pair, remote_serial
from hil.wait import wait_until

pytestmark = pytest.mark.redundancy

ROLE_HEADER = "X-Dali2rust-Role"
QUERY_ACTUAL_LEVEL = 0xA0
TAKEOVER_MARGIN_S = 2.0
STAND_DOWN_TIMEOUT_S = 90.0
REPLICATION_TIMEOUT_S = 75.0
PRIMARY_HOLD_S = 12.0
QUIET_WINDOW_S = 10.0



def _present_shorts(api):
    return api.present_addrs()


def _actual_level(client, short):
    res = client.cmd(short, QUERY_ACTUAL_LEVEL)
    if not res.get("success", True):
        return None
    return res.get("backward_frame")


def _role_header(client):
    return client.raw_response("GET", "health").headers.get(ROLE_HEADER)


def _target_state_path(client, short):
    return "adapters/%d/physical-devices/%d/target-state" % (client.adapter, short)


def _latest(state):
    return state["transitions"][0] if state["transitions"] else None


@pytest.fixture(autouse=True)
def peer_continuity(request, peer_api):
    try:
        before = float(peer_api.health()["uptime_seconds"])
    except Exception:
        before = None
    started = time.monotonic()
    yield
    if before is None:
        return
    try:
        after = float(peer_api.health()["uptime_seconds"])
    except Exception as exc:
        pytest.fail("the peer stopped answering during %s (%s) — a reboot presents "
                    "exactly this way" % (request.node.name, exc), pytrace=False)
    expected = before + (time.monotonic() - started)
    if after < expected - 5.0:
        pytest.fail("the PEER rebooted during %s: uptime %.0fs -> %.0fs across a %.0fs test"
                    % (request.node.name, before, after, time.monotonic() - started),
                    pytrace=False)


@pytest.fixture()
def pair_roles(api, peer_api):
    pair.settle(api, peer_api)
    yield
    pair.settle(api, peer_api)



@pytest.mark.hil_id("HIL-RED-01")
def test_the_pair_arbitrates_on_the_wire(api, peer_api, pair_roles, test_artifacts):
    a0, s0 = pair.roles(api, peer_api)
    assert a0["enabled"] and s0["enabled"], "redundancy is not enabled on both units"
    assert a0["role"] == "primary" and s0["role"] == "standby"
    assert a0["active"] and a0["answering"] and a0["lease_remaining_ms"] > 0, \
        "the primary is not answering probes: %s" % a0
    assert not s0["active"] and not s0["answering"], \
        "a passive unit publishes an empty answer table: %s" % s0

    time.sleep(QUIET_WINDOW_S)
    a1, s1 = pair.roles(api, peer_api)
    published = s1["probes"]["published"] - s0["probes"]["published"]
    owned = s1["probes"]["owned"] - s0["probes"]["owned"]
    unowned = s1["probes"]["unowned"] - s0["probes"]["unowned"]
    settings = peer_api.redundancy.settings()
    expected = QUIET_WINDOW_S * 1000.0 / settings["probe_interval_ms"]
    test_artifacts.attach_json("probe_window", {
        "seconds": QUIET_WINDOW_S, "published": published, "owned": owned,
        "unowned": unowned, "probe_interval_ms": settings["probe_interval_ms"],
        "primary_versions": [api.health().get("version"), peer_api.health().get("version")],
    })
    assert published >= expected * 0.5, \
        "the standby published %d probes in %.0f s at %d ms — it is not asking" \
        % (published, QUIET_WINDOW_S, settings["probe_interval_ms"])
    assert published <= expected * 1.5 + 2, \
        "the standby published %d probes in %.0f s — the interval is not being kept" \
        % (published, QUIET_WINDOW_S)
    assert owned >= 1, "not one probe was answered: the primary is silent on the wire"
    assert s1["takeovers"] == s0["takeovers"] and s1["stand_downs"] == s0["stand_downs"], \
        "the bus changed hands during a quiet window: %s" % s1["transitions"]
    assert a1["active"] and not s1["active"]

    for client, role in ((api, "active"), (peer_api, "standby")):
        assert _role_header(client) == role, "%s does not stamp its role" % client.base
        assert client.health().get("role") == role
    assert not peer_api.controller()["home_assistant"]["connected"], \
        "the standby holds an MQTT session"



@pytest.mark.hil_id("HIL-RED-02")
def test_a_standby_refuses_writes_and_names_the_remedy(api, peer_api, pair_roles,
                                                      state_snapshot, test_artifacts):
    shorts = _present_shorts(api)
    if not shorts:
        pytest.skip("no gear on the wire to refuse a write for")
    short = shorts[0]
    before = _actual_level(api, short)
    assert before is not None, "SA%02d did not answer QUERY ACTUAL LEVEL" % short
    level = 200 if before != 200 else 100

    res = peer_api.raw_response("PUT", _target_state_path(peer_api, short),
                                {"power": "on", "level": level})
    test_artifacts.attach_json("refusal", {
        "status": res.status_code, "headers": dict(res.headers), "body": res.text})
    assert res.status_code == 409, res.text
    assert res.json().get("error") == "controller_standby", res.text
    assert res.headers.get("Retry-After") == "1"
    assert res.headers.get(ROLE_HEADER) == "standby"
    assert isinstance(peer_api.devices_unfiltered().get("physical_devices"), list)
    sw = peer_api.raw_response("POST", "redundancy/switchover", {})
    assert sw.status_code == 409 and sw.json().get("error") == "not_the_active_controller", sw.text

    time.sleep(1.0)
    after = _actual_level(api, short)
    assert after == before, \
        "the refused write reached the gear: SA%02d %s -> %s" % (short, before, after)



def _manifest_map(client):
    return {row["name"]: (row["bytes"], row["crc32"])
            for row in client.config.manifest() if row.get("bytes") is not None}


def _peer_holds_everything(api, peer_api):
    want = _manifest_map(api)
    have = _manifest_map(peer_api)
    return all(have.get(name) == digest for name, digest in want.items())


@pytest.mark.hil_id("HIL-RED-03")
def test_configuration_replicates_from_the_active_unit(api, peer_api, pair_roles,
                                                       test_artifacts):
    assert wait_until(lambda: _peer_holds_everything(api, peer_api),
                      REPLICATION_TIMEOUT_S, 5.0), \
        "the standby never converged on the primary's manifest:\n primary %s\n peer %s" \
        % (_manifest_map(api), _manifest_map(peer_api))
    rep = peer_api.redundancy.get()["replication"]
    test_artifacts.attach_json("replication", rep)
    assert rep["passes"] >= 1 and rep["rejected"] == 0, rep
    primary_shorts = {d["short_address"] for d in api.devices_unfiltered()["physical_devices"]}
    peer_shorts = {d["short_address"] for d in peer_api.devices_unfiltered()["physical_devices"]}
    assert primary_shorts <= peer_shorts, \
        "devices missing on the standby: %s" % sorted(primary_shorts - peer_shorts)

    before = api.poller.get()
    edited = dict(interval_ms=before["interval_ms"] + 1000)
    try:
        api.poller.patch(edited)
        assert wait_until(lambda: peer_api.poller.get()["interval_ms"] == edited["interval_ms"],
                          REPLICATION_TIMEOUT_S, 5.0), \
            "the edit never reached the standby: primary %s, peer %s" \
            % (api.poller.get(), peer_api.poller.get())
    finally:
        api.poller.patch({"interval_ms": before["interval_ms"]})
    assert wait_until(lambda: peer_api.poller.get()["interval_ms"] == before["interval_ms"],
                      REPLICATION_TIMEOUT_S, 5.0)



@pytest.mark.hil_id("HIL-RED-04")
def test_losing_the_peer_link_moves_no_role(api, peer_api, pair_roles, hil_config,
                                            test_artifacts):
    original = peer_api.redundancy.settings()["peer_url"]
    s0 = peer_api.redundancy.get()
    dead = "http://%s:81" % hil_config.mqtt_broker_host
    try:
        peer_api.redundancy.patch_settings({"peer_url": dead})
        grew = wait_until(
            lambda: peer_api.redundancy.get()["replication"]["peer_unreachable"]
            > s0["replication"]["peer_unreachable"], 45.0, 2.0)
        answered = wait_until(
            lambda: peer_api.redundancy.get()["probes"]["owned"] > s0["probes"]["owned"],
            5.0, 0.5)
        s1 = peer_api.redundancy.get()
        a1 = api.redundancy.get()
    finally:
        peer_api.redundancy.patch_settings({"peer_url": original})
    test_artifacts.attach_json("during_outage", {"peer": s1, "primary": a1})
    assert grew, "the standby never noticed its peer was unreachable: %s" % s1["replication"]
    assert len(s1["transitions"]) == len(s0["transitions"]), \
        "the role moved on a network fault: %s" % s1["transitions"]
    assert not s1["active"] and a1["active"]
    assert answered, \
        "the wire stopped deciding: no probe was answered during the outage"



@pytest.mark.hil_id("HIL-RED-05")
@pytest.mark.serial
def test_a_silent_primary_hands_the_bus_over_and_takes_it_back(
        api, peer_api, hil_config, pair_roles, state_snapshot, dut_reboot, test_artifacts):
    if not remote_serial.enabled(hil_config):
        pytest.skip("halting the primary needs the WB bridge's control port")
    shorts = _present_shorts(api)
    if not shorts:
        pytest.skip("no gear on the wire for the standby to drive")
    short = shorts[0]
    settings = peer_api.redundancy.settings()
    bound_s = (settings["takeover_after_missed"] + 1) * settings["probe_interval_ms"] / 1000.0
    before = _actual_level(api, short)
    s0 = peer_api.redundancy.get()
    timeline = {"bound_s": bound_s, "before_level": before}

    with api.expect_reboot():
        remote_serial.control(hil_config, "bootloader")
        t0 = time.monotonic()
        claimed = wait_until(lambda: peer_api.redundancy.get()["active"],
                             bound_s + TAKEOVER_MARGIN_S, 0.1)
        timeline["claim_seen_after_s"] = round(time.monotonic() - t0, 2)
        try:
            assert claimed, "the standby did not claim the bus within %.1f s" % (
                bound_s + TAKEOVER_MARGIN_S)
            s1 = peer_api.redundancy.get()
            last = _latest(s1)
            timeline["takeover"] = last
            assert last["now_active"] and last["reason"] == "peer_silent", last
            assert s1["takeovers"] == s0["takeovers"] + 1
            assert _role_header(peer_api) == "active"
            level = 150 if before != 150 else 90
            peer_api.ts(short, {"power": "on", "level": level})
            assert wait_until(lambda: _actual_level(peer_api, short) == level, 5.0, 0.5), \
                "the active standby did not move SA%02d to %d (reads %s)" % (
                    short, level, _actual_level(peer_api, short))
            timeline["driven_level"] = level
            remaining = PRIMARY_HOLD_S - (time.monotonic() - t0)
            if remaining > 0:
                time.sleep(remaining)
        finally:
            remote_serial.control(hil_config, "run")
            t_run = time.monotonic()
        samples = []
        deadline = t_run + STAND_DOWN_TIMEOUT_S
        primary_active_at = None
        while time.monotonic() < deadline:
            now = round(time.monotonic() - t_run, 2)
            try:
                p_active = api.redundancy.get()["active"]
            except Exception:
                p_active = None
            s_active = peer_api.redundancy.get()["active"]
            samples.append((now, p_active, s_active))
            if p_active and primary_active_at is None:
                primary_active_at = now
            if not s_active:
                break
            time.sleep(0.2)
    timeline["return_samples"] = samples
    s2 = peer_api.redundancy.get()
    timeline["stand_down"] = _latest(s2)
    both = [t for t, p, s in samples if p and s]
    timeline["both_active_span_s"] = (max(both) - min(both)) if both else 0.0
    test_artifacts.attach_json("failover_timeline", timeline)
    assert not s2["active"], "the standby never stood down after the primary returned"
    assert _latest(s2)["reason"] == "peer_answered", _latest(s2)
    assert api.redundancy.get()["active"]
    assert timeline["both_active_span_s"] <= 2 * settings["probe_interval_ms"] / 1000.0 + 1.0, \
        "both units drove the bus for %.1f s" % timeline["both_active_span_s"]



@pytest.mark.hil_id("HIL-RED-06")
def test_a_planned_switchover_moves_the_bus_and_not_the_light(api, peer_api, pair_roles,
                                                              state_snapshot, test_artifacts):
    shorts = _present_shorts(api)[:2]
    if not shorts:
        pytest.skip("no gear on the wire to watch")
    before = {s: _actual_level(api, s) for s in shorts}
    assert all(v is not None for v in before.values()), before
    record = {"before": before}

    api.redundancy.switchover()
    assert wait_until(lambda: peer_api.redundancy.get()["active"]
                      and not api.redundancy.get()["active"], 10.0, 0.2), \
        "the bus did not change hands: %s / %s" % pair.roles(api, peer_api)
    a1, s1 = pair.roles(api, peer_api)
    record["after_handover"] = {"primary": _latest(a1), "peer": _latest(s1)}
    assert _latest(s1)["reason"] == "handover", _latest(s1)
    assert _latest(a1)["reason"] == "handover", _latest(a1)
    assert _role_header(api) == "standby" and _role_header(peer_api) == "active"
    during = {s: _actual_level(peer_api, s) for s in shorts}
    record["during"] = during
    assert during == before, "the light moved on handover: %s -> %s" % (before, during)
    res = api.raw_response("PUT", _target_state_path(api, shorts[0]),
                           {"power": "on", "level": before[shorts[0]] or 1})
    assert res.status_code == 409 and res.json().get("error") == "controller_standby", res.text

    peer_api.redundancy.switchover()
    assert wait_until(lambda: pair.settled(api, peer_api), 10.0, 0.2), \
        "the bus did not come back: %s / %s" % pair.roles(api, peer_api)
    a2, s2 = pair.roles(api, peer_api)
    assert _latest(a2)["now_active"] and _latest(a2)["reason"] == "handover", _latest(a2)
    assert not _latest(s2)["now_active"] and _latest(s2)["reason"] == "handover", _latest(s2)
    after = {s: _actual_level(api, s) for s in shorts}
    record["after"] = after
    test_artifacts.attach_json("switchover", record)
    assert after == before, "the light moved on the way back: %s -> %s" % (before, after)


ANSWER_OWNED_SLACK = 1


QUIET_BUS_S = 4.0
QUIET_BUS_TIMEOUT_S = 45.0


def _await_quiet_bus(api, peer_api):
    def counters():
        w = api.diagnostics().get("dali_wire") or {}
        p = peer_api.diagnostics()
        return (w.get("collisions"), w.get("exchange_retries"),
                (p.get("dali_wire") or {}).get("collisions"),
                (p.get("redundancy") or {}).get("probe_failed"))
    deadline = time.monotonic() + QUIET_BUS_TIMEOUT_S
    last = counters()
    while time.monotonic() < deadline:
        time.sleep(QUIET_BUS_S)
        now = counters()
        if now == last:
            return
        last = now
    pytest.fail("the bus never went quiet for %.0f s before the measurement: %s"
                % (QUIET_BUS_S, last), pytrace=False)


def _arb_snapshot(api, peer_api):
    d = api.diagnostics()
    p = peer_api.diagnostics()
    return {
        "primary_redundancy": d.get("redundancy") or {},
        "primary_wire": d.get("dali_wire") or {},
        "primary_sniffer": d.get("phy_sniffer") or {},
        "peer_probes": peer_api.redundancy.get().get("probes") or {},
        "peer_transitions": peer_api.redundancy.get().get("transitions") or [],
        "peer_wire": p.get("dali_wire") or {},
    }


def _d(after, before, section, key):
    return (after[section].get(key, 0) or 0) - (before[section].get(key, 0) or 0)


def _assert_every_probe_answered(before, after, test_artifacts, label, collision_free=False):
    new_transitions = len(after["peer_transitions"]) - len(before["peer_transitions"])
    silent = [t for t in after["peer_transitions"][:max(new_transitions, 0)]
              if t.get("reason") == "peer_silent"]
    unowned = _d(after, before, "peer_probes", "unowned")
    answered = _d(after, before, "primary_redundancy", "answered")
    owned = _d(after, before, "peer_probes", "owned")
    cell_busy = _d(after, before, "primary_redundancy", "cell_busy")
    window_closed = _d(after, before, "primary_redundancy", "window_closed")
    report = {
        "new_peer_transitions": new_transitions,
        "unowned_delta": unowned,
        "answered_delta": answered,
        "owned_delta": owned,
        "cell_busy_delta": cell_busy,
        "window_closed_delta": window_closed,
        "suppressed_delta": _d(after, before, "primary_redundancy", "suppressed"),
        "late_delta": _d(after, before, "primary_redundancy", "late"),
        "aborted_delta": _d(after, before, "primary_redundancy", "aborted"),
        "primary_collisions_delta": _d(after, before, "primary_wire", "collisions"),
        "peer_collisions_delta": _d(after, before, "peer_wire", "collisions"),
        "primary_exchange_retries_delta": _d(after, before, "primary_wire", "exchange_retries"),
        "primary_bus_releases_delta": _d(after, before, "primary_wire", "bus_releases"),
    }
    undecoded = (_d(after, before, "primary_sniffer", "unsupported_len")
                 + _d(after, before, "primary_sniffer", "decode_failed"))
    report["primary_undecoded_delta"] = undecoded
    test_artifacts.attach_json(label, report)
    assert not silent, \
        "the standby took the bus mid-sequence (%d peer_silent): %s" % (len(silent), report)
    assert unowned <= undecoded, \
        "the peer saw %d unanswered probes and only %d of them are captures we could not " \
        "decode: %s" % (unowned, undecoded, report)
    assert cell_busy == 0, "an answer could not be staged: %s" % report
    assert window_closed == 0, "an answer missed its window: %s" % report
    assert abs(answered - owned) <= ANSWER_OWNED_SLACK, \
        "answered (%d) and owned (%d) disagree by more than a probe in flight: %s" \
        % (answered, owned, report)
    if collision_free:
        assert report["primary_collisions_delta"] <= 1 and report["peer_collisions_delta"] <= 1, \
            "the probe and the sequence collided: %s" % report


@pytest.mark.hil_id("HIL-RED-07")
@pytest.mark.redundancy
def test_a_103_walk_on_the_primary_leaves_every_probe_answered(
        api, peer_api, pair_roles, panel, test_artifacts):
    _await_quiet_bus(api, peer_api)
    before = _arb_snapshot(api, peer_api)
    for _ in range(2):
        api.wait_op(api.input_scan())
    after = _arb_snapshot(api, peer_api)
    _assert_every_probe_answered(before, after, test_artifacts, "walk_103", collision_free=True)


@pytest.mark.hil_id("HIL-RED-08")
@pytest.mark.redundancy
def test_a_16_bit_long_sequence_on_the_primary_leaves_every_probe_answered(
        api, peer_api, pair_roles, test_artifacts):
    shorts = _present_shorts(api)
    if not shorts:
        pytest.skip("no gear on the wire to read")
    _await_quiet_bus(api, peer_api)
    before = _arb_snapshot(api, peer_api)
    from hil.api import ATTR_GROUPS_DEFAULT
    for short in shorts[:3]:
        api.wait_op(api.attr_read(short, groups=ATTR_GROUPS_DEFAULT, banks="all"))
    after = _arb_snapshot(api, peer_api)
    _assert_every_probe_answered(before, after, test_artifacts, "read_16bit")
