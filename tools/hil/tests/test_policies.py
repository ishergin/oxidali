import pytest

from hil.lamp_guard import LampNotAllowed, named, spell
from hil.wait import wait_until

pytestmark = pytest.mark.destructive

POLICY_LEVELS = {"power_on_level": 254, "system_failure_level": 254}
POLICY_OP_PREFIX = "policy-apply-"
DISCOVERY_MODE = "refresh_known"
DISCOVERY_TIMEOUT_S = 180
POLICY_APPLY_TIMEOUT_S = 120
POLICY_OP_APPEAR_S = 20.0
POLICY_OP_POLL_S = 0.5
WRITE_CONFIRMED = "write_confirmed"
COMMON_102 = "common_102"


def _registered(api):
    return sorted(d["short_address"] for d in api.devices_unfiltered()["physical_devices"])


def _refused(api, shorts):
    refused = []
    for short in shorts:
        try:
            api.guard.check_target(short, False, "a policy apply to SA%d" % short)
        except LampNotAllowed:
            refused.append(short)
    return refused


def _leaves(api, short):
    section = ((api.attributes(short, [COMMON_102]).get("attributes") or {})
               .get(COMMON_102) or {})
    return {field: section.get(field) or {} for field in POLICY_LEVELS}


def _new_policy_applies(api, known):
    found = []

    def _appeared():
        found[:] = sorted(key for key in api.operations()
                          if key.startswith(POLICY_OP_PREFIX) and key not in known)
        return bool(found)

    wait_until(_appeared, POLICY_OP_APPEAR_S, interval_s=POLICY_OP_POLL_S)
    return found


def unconfirmed_levels(before, after, wanted):
    out = []
    for short, leaves in sorted(after.items()):
        for field, leaf in sorted(leaves.items()):
            was = before[short][field].get("last_write_confirmed_ms")
            now = leaf.get("last_write_confirmed_ms")
            if leaf.get("source") != WRITE_CONFIRMED or leaf.get("value") != wanted[field] \
                    or now is None or (was is not None and now <= was):
                out.append("SA%d %s %r" % (short, field, leaf))
    return out


@pytest.mark.hil_id("HIL-POLICY-01")
def test_a_discovery_run_writes_the_armed_policy_without_a_manual_apply(
        api, hil_config, attr_guard, policy_guard, op_check, test_artifacts):
    shorts = _registered(api)
    if not shorts:
        pytest.skip("no gear is registered on adapter %d, so an armed policy writes "
                    "nothing" % api.adapter)
    refused = _refused(api, shorts)
    if refused:
        pytest.skip(
            "a policy apply writes power_on_level and system_failure_level into every "
            "registered device, and the lamp guard refuses %s (HIL_LAMP_SHORTS=%s): on an "
            "installation of the owner's lamps this test never runs, by design"
            % (named(refused), spell(hil_config.lamp_short_set())))
    for short in shorts:
        attr_guard(short, *POLICY_LEVELS, verify=True, required=True)
    before = {short: _leaves(api, short) for short in shorts}
    known = set(api.operations())

    api._req("PATCH", "policies", dict(POLICY_LEVELS, apply_on_discovery=True))
    op_check(api.wait_op(api.discovery(DISCOVERY_MODE), timeout_s=DISCOVERY_TIMEOUT_S))
    applies = _new_policy_applies(api, known)
    assert len(applies) == 1, (
        "a successful %s run with apply_on_discovery armed started %d policy apply "
        "operation(s) within %.0f s, exactly one expected: %r"
        % (DISCOVERY_MODE, len(applies), POLICY_OP_APPEAR_S, applies))
    view = op_check(api.wait_op(applies[0], timeout_s=POLICY_APPLY_TIMEOUT_S))
    after = {short: _leaves(api, short) for short in shorts}
    test_artifacts.attach_json("policy_apply", {"operation": view, "before": before,
                                                "after": after})

    unconfirmed = unconfirmed_levels(before, after, POLICY_LEVELS)
    assert not unconfirmed, (
        "the policy apply the discovery run started left levels without a fresh "
        "write_confirmed provenance: %s" % unconfirmed)
