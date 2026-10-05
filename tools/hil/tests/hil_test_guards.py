import datetime
import os
import time

import pytest
from requests import RequestException

from hil import api as api_mod
from hil import prod_state
from hil import virtual_gear, write_log
from hil.lamp_guard import (HA_TEST_NAMESPACE, RULE_SEPARATOR, LampNotAllowed, shown_keys,
                            spell)
from hil.wait import wait_until
from hil_harness import ANCHOR_TZ
from hil_session_guards import refuse_schedule_suspension
from hil_session import LIGHT_MARKER

TEARDOWN_RETRY_S = 2.0


RULES_SOURCE_LIMIT_BYTES = 12_240


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


def _teardown_write(api, what, fn):
    try:
        return fn()
    except RequestException as exc:
        api.count_retry("teardown_write", "%s (%s)" % (what, api_mod._cause_of(exc)))
        time.sleep(TEARDOWN_RETRY_S)
        return fn()


@pytest.fixture()
def group_matrix_guard(api):
    ask_guard(api, "POST", "adapters/%d/groups/apply" % api.adapter)
    before = api.groups.matrix()
    yield before
    rows = [{"virtual_lamp_id": r["virtual_lamp_id"], "desired": r["desired"]}
            for r in before["rows"]]
    if rows:
        _teardown_write(api, "group matrix PATCH",
                        lambda: api.groups.matrix_patch(rows))
    res = _teardown_write(api, "group apply", api.groups.apply)
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
        ask_guard(api, "POST", "adapters/%d/scenes/%d/apply" % (api.adapter, scene_id))
        snap = api.scenes.matrix(scene_id)
        guarded.append((scene_id, snap))
        return snap
    yield guard
    for scene_id, before in reversed(guarded):
        rows = [{"virtual_lamp_id": r["virtual_lamp_id"],
                 "desired": _scene_desired_writeback(r["desired"])}
                for r in before["rows"]]
        if rows:
            _teardown_write(api, "scene %d matrix PATCH" % scene_id,
                            lambda: api.scenes.matrix_patch(scene_id, rows))
        res = _teardown_write(api, "scene %d apply" % scene_id,
                              lambda: api.scenes.apply(scene_id))
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
    "power_on_level": "common_102",
    "system_failure_level": "common_102",
    "dimming_curve": "dt6_led",
}


def refuse_unread(short, attrs, values, default):
    unread = sorted(a for a in attrs if values[a] is None and default is None)
    if unread:
        pytest.skip("SA%d did not report %s, so nothing could put it back: the test does "
                    "not write it" % (short, ", ".join(unread)))


@pytest.fixture()
def attr_guard(api):
    guarded = []

    def _section(short, name):
        return ((api.attributes(short, [name]).get("attributes") or {})
                .get(name) or {})

    def guard(short, *attrs, default=None, verify=False, required=False):
        for name in sorted({_ATTR_GUARD_SECTIONS[a] for a in attrs}):
            api.attr_read_checked(short, groups=name)
        sections = {name: _section(short, name)
                    for name in {_ATTR_GUARD_SECTIONS[a] for a in attrs}}
        values = {attr: (sections[_ATTR_GUARD_SECTIONS[attr]].get(attr)
                         or {}).get("value")
                  for attr in attrs}
        if required:
            refuse_unread(short, attrs, values, default)
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


DROPPED = "continuations_dropped"
U32 = 1 << 32


def rules_residue(source, now_source, toggles, now_toggles):
    out = [] if now_source == source else ["the document is not the one the test found"]
    out += ["rule %r is %s, was %s" % (name, _toggle(now_toggles.get(name)),
                                       _toggle(toggles.get(name)))
            for name in sorted(set(toggles) | set(now_toggles))
            if now_toggles.get(name) != toggles.get(name)]
    return out


def _toggle(enabled):
    return {True: "enabled", False: "disabled"}.get(enabled, "absent")


def _refuse_a_live_owner_document(toggles, pending):
    disabled = sorted(name for name, enabled in toggles.items() if not enabled)
    if disabled:
        pytest.skip("the owner's rule(s) %s are switched off, and a document commit switches "
                    "every rule back to its text until the guard reasserts it" % disabled)
    refusal = prod_state.commit_refusal(pending)
    if refusal:
        pytest.skip(refusal)


class _RulesCommits:
    def __init__(self, api, doc, toggles):
        self.api, self.original, self.toggles = api, doc.get("source") or "", toggles
        self.base, self.ours, self.owed = doc.get("revision"), None, {}

    def commit(self, source, base, what):
        wanted = {**self.api.rules_toggles(), **self.owed}
        try:
            view = self.api.rules_replace(source, base)
        except api_mod.ApiError as exc:
            pytest.fail("rules %s did not commit (%s): the document changed while the test "
                        "held it, so the owner's edit is kept and the test rules %s stay, "
                        "switched off, for a hand to remove"
                        % (what, exc, prod_state.switch_off_test_rules(self.api)),
                        pytrace=False)
        if view.get("status") != "succeeded":
            pytest.fail("rules %s did not commit: %r" % (what, view), pytrace=False)
        self.ours = (base + 1) % U32
        now = self.api.rules_toggles()
        self.owed = {n: e for n, e in wanted.items() if n in now and now[n] != e}
        for name, enabled in sorted(dict(self.owed).items()):
            self.api.rule_enable(name, enabled)
            self.ours = (self.ours + 1) % U32
            del self.owed[name]

    def append(self, fragment):
        source = self.original + RULE_SEPARATOR + fragment if self.original else fragment
        if len(source.encode()) > RULES_SOURCE_LIMIT_BYTES:
            pytest.skip(
                "the stored document plus a test rule exceeds the %d-byte "
                "source limit; this bench cannot add one without evicting the "
                "owner's" % RULES_SOURCE_LIMIT_BYTES)
        _refuse_a_live_owner_document(self.api.rules_toggles(),
                                      prod_state.continuations_pending(self.api))
        self.commit(source, self.base if self.ours is None else self.ours, "append")

    def restore(self):
        if self.ours is None:
            return []
        source = self.api.rules_get().get("source") or ""
        if source != self.original:
            refusal = prod_state.commit_refusal(prod_state.continuations_pending(self.api))
            if refusal:
                return ["the test rules %s stay in the document, switched off, for the "
                        "session restore, because %s"
                        % (prod_state.switch_off_test_rules(self.api), refusal)]
            self.commit(self.original, self.ours, "restore")
        return rules_residue(self.original, self.api.rules_get().get("source") or "",
                             self.toggles, self.api.rules_toggles())


@pytest.fixture()
def rules_guard(api):
    doc = api.rules_get()
    if doc.get("diagnostic"):
        pytest.skip("the stored rules document does not compile (%s) — a test "
                    "rule appended to it would be refused for that reason"
                    % doc["diagnostic"])
    toggles, rules_stats = api.rules_toggles(), api.stats().get("rules") or {}
    _refuse_a_live_owner_document(toggles, prod_state.continuations_pending(api))
    commits = _RulesCommits(api, doc, toggles)
    try:
        yield commits.append
    finally:
        residue = commits.restore()
        dropped = (int((api.stats().get("rules") or {}).get(DROPPED) or 0)
                   - int(rules_stats.get(DROPPED) or 0)) % U32
        if commits.ours is not None and dropped:
            residue.append("%d pending delayed action(s) of the owner's rules were dropped"
                           % dropped)
        if residue:
            pytest.fail("RULES NOT RESTORED: %s — a document commit resets every rule's "
                        "toggle to its text and drops what after/wait still owed"
                        % "; ".join(residue), pytrace=False)


@pytest.fixture()
def policy_guard(api):
    before = api._req("GET", "policies")
    try:
        yield before
    finally:
        prod_state.restore_policies(api, before)
        left = prod_state.policy_patch(before, api._req("GET", "policies"))
        if left:
            pytest.fail("POLICY NOT RESTORED: %r still differ from %r — an armed policy is "
                        "written into every registered device by the next scan"
                        % (sorted(left), before), pytrace=False)


@pytest.fixture()
def adapter_enabled_guard(api):
    yield
    api.adapter_patch({"enabled": True})


def ask_guard(api, method, path, body=None):
    try:
        api.guard.check_request(method, path, body)
    except LampNotAllowed as exc:
        pytest.skip(str(exc))


def drive_allowed(api, target, what):
    try:
        reached = api.guard.check_target(target, True, what)
    except LampNotAllowed as exc:
        pytest.skip("%s — the controller drives it past the client, so the guard is asked "
                    "before the test starts" % exc)
    write_log.note(api.base, shown_keys(reached))


def allowed_bound_lamp(api, what, wanted=None):
    allowed = set(api.lamp_addrs())
    for lamp in api.vlamps.list()["virtual_lamps"]:
        short = (lamp.get("binding") or {}).get("physical_short_address")
        if short in allowed and (wanted is None or wanted(lamp)):
            drive_allowed(api, short, what)
            return lamp["virtual_lamp_id"], short
    pytest.skip("no fitting virtual lamp is bound to a lamp of HIL_LAMP_SHORTS=%s for %s"
                % (spell(allowed), what))


def free_group_of(api):
    devices = api.devices_unfiltered()["physical_devices"]
    used = virtual_gear.used_groups(devices, api.groups.matrix().get("rows", []))
    if used is None:
        pytest.skip("SA%s report no group membership, so no group can be shown free"
                    % spell(d["short_address"] for d in devices
                            if d.get("groups_membership") is None))
    ruled = virtual_gear.owner_rule_groups(api)
    free = [g for g in range(virtual_gear.GROUP_COUNT - 1, -1, -1)
            if g not in used and g not in ruled]
    if not free:
        pytest.skip("no DALI group is free of registered gear and of the owner's rules")
    try:
        virtual_gear.prove_groups_empty(api, free[:1], used)
    except virtual_gear.VirtualGearError as exc:
        pytest.skip("group %d cannot be shown empty on the wire: %s" % (free[0], exc))
    return free[0]


@pytest.fixture()
def free_group(api, capabilities):
    for d in api.devices()["physical_devices"]:
        capabilities.ensure(d["short_address"], "groups")
    return free_group_of(api)


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


def namespace_controller_id():
    return HA_TEST_NAMESPACE["controller_id"]


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
        **HA_TEST_NAMESPACE,
        "publish_qos": 1,
        "retain_state": True,
        "retain_discovery": True,
    }

    class Guard:
        retained_filters = ("%s/#" % HA_TEST_NAMESPACE["discovery_prefix"],
                            "%s/#" % HA_TEST_NAMESPACE["state_topic_prefix"])

        def enter(self, **overrides):
            body = dict(namespace)
            body.update(overrides)
            return api.ha.patch(body)

        @staticmethod
        def topic(suffix):
            return "%s/%s/%s" % (HA_TEST_NAMESPACE["state_topic_prefix"],
                                 HA_TEST_NAMESPACE["controller_id"], suffix)

        @staticmethod
        def config_topic(component, object_id):
            return "%s/%s/%s/%s/config" % (HA_TEST_NAMESPACE["discovery_prefix"], component,
                                           HA_TEST_NAMESPACE["controller_id"], object_id)

    try:
        yield Guard()
    finally:
        for filt in Guard.retained_filters:
            try:
                for topic in mqtt_tap.collect_retained(hil_config, filt):
                    mqtt_tap.clear_retained(hil_config, topic)
            except Exception:
                pass
        api.ha.patch({k: before[k] for k in api_mod._HomeAssistantSettings.RESTORABLE
                      if k in before})


class _Clock:
    def __init__(self, api):
        self.api, self.before = api, None

    def at(self, hour, minute, day_offset=0):
        if self.api.cfg.lamps_read_only:
            pytest.skip("a read-only run never moves the controller's clock: a move fires "
                        "the owner's timed rules and publishes every HCL schedule's point")
        timed = api_mod.rules_on(self.api._req("GET", "rules?format=json"),
                                 api_mod.TIMED_TRIGGERS)
        if timed:
            pytest.skip("the owner's rule(s) %s fire at a time of day or at the sun, and a "
                        "clock move fires each whose time falls in the hour before the new "
                        "time" % timed)
        if self.before is None:
            self.before = self.api.time_get()
            self.api.time_set(timezone=ANCHOR_TZ)
        day = (datetime.datetime.now(datetime.timezone.utc)
               + datetime.timedelta(days=day_offset)).date()
        target = datetime.datetime(day.year, day.month, day.day, hour, minute,
                                   tzinfo=datetime.timezone.utc)
        unix_ms = int(target.timestamp() * 1000)
        self.api.time_set(unix_ms=unix_ms)
        return unix_ms

    def restore(self):
        if self.before is None:
            return
        self.api.time_set(unix_ms=int(time.time() * 1000))
        if self.before.get("timezone"):
            self.api.time_set(timezone=self.before["timezone"])


@pytest.fixture()
def clock_guard(api):
    clock = _Clock(api)
    try:
        yield clock.at
    finally:
        clock.restore()


@pytest.fixture(autouse=True)
def owner_rules_ignore_the_test_lamps(request):
    if request.node.get_closest_marker(LIGHT_MARKER) is None:
        return
    api = request.getfixturevalue("api")
    conflicts = virtual_gear.real_tier_conflicts(api, api.cfg.lamp_short_set())
    if conflicts:
        pytest.skip("an owner rule reacts to the lamps this test drives: %s"
                    % "; ".join(conflicts))


@pytest.fixture()
def hcl_guard(api):
    refuse_schedule_suspension(api)
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
