import datetime
import os
import time

import pytest
from requests import RequestException

from hil import api as api_mod
from hil.wait import wait_until
from hil_harness import ANCHOR_TZ

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
def rules_guard(api):
    doc = api.rules_get()
    if doc.get("diagnostic"):
        pytest.skip("the stored rules document does not compile (%s) — a test "
                    "rule appended to it would be refused for that reason"
                    % doc["diagnostic"])
    original = doc.get("source") or ""

    def _commit(source, what):
        view = api.rules_replace(source, api.rules_get()["revision"])
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
