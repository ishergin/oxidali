import time

import pytest

FULL_GROUPS = "runtime_status,common_102,dt8_color,dt6_led,extended,groups,scenes"
PUT_BUDGET_MS = 1000


def _first_short(api):
    addrs = api.addrs()
    return addrs[0] if addrs else None


def _dali_counters(api):
    return api.diagnostics()["dali_worker"]


def _delta(before, after, name):
    return after.get(name, 0) - before.get(name, 0)


def test_operator_setpoint_is_not_delayed_by_an_attribute_read(api, test_artifacts):
    short = _first_short(api)
    if short is None:
        pytest.skip("no physical device on the rig")

    before = _dali_counters(api)
    api.attr_read(short, groups=FULL_GROUPS, banks="all")

    samples, failures = [], []
    for i in range(10):
        started = time.time()
        status, payload = api.raw_request(
            "PUT", "adapters/%d/physical-devices/%d/target-state" % (api.adapter, short),
            {"power": "on", "level": 200 if i % 2 else 120})
        elapsed_ms = (time.time() - started) * 1000
        samples.append(elapsed_ms)
        if status != 200:
            failures.append((status, payload, round(elapsed_ms)))

    after = _dali_counters(api)
    delta = {
        name: _delta(before, after, name)
        for name in ("read_attributes_preempted", "read_attributes_transport_aborts",
                     "read_attributes_contended_aborts", "read_attributes_device_absent",
                     "write_attributes_preempted")
    }
    test_artifacts.attach_json("put_latency_ms", {
        "samples": [round(s) for s in samples],
        "max": round(max(samples)),
        "failures": failures,
    })
    test_artifacts.attach_json("dali_worker_delta", delta)

    assert not failures, \
        "an operator PUT failed while an attribute read was running: %r" % failures
    assert max(samples) < PUT_BUDGET_MS, \
        "worst PUT %d ms — the read is still holding the wire: %s" % (
            round(max(samples)), delta)
    assert delta["read_attributes_preempted"] > 0, \
        "no read was preempted — the PUTs did not race anything: %s" % delta
    assert delta["read_attributes_transport_aborts"] == 0, \
        "a yield was miscounted as a transport abort: %s" % delta
    assert delta["read_attributes_device_absent"] == 0, \
        "a yield was miscounted as an absent device: %s" % delta


def test_a_preempted_read_leaves_the_readings_untouched(api, test_artifacts):
    short = _first_short(api)
    if short is None:
        pytest.skip("no physical device on the rig")

    path = "adapters/%d/physical-devices/%d" % (api.adapter, short)
    before_view = api.attributes(short)["attributes"]

    op = api.attr_read(short, groups=FULL_GROUPS, banks="all")
    api.raw_request("PUT", "%s/target-state" % path, {"power": "on", "level": 160})
    view = api.wait_op(op)

    test_artifacts.attach_json("preempted_operation", view)
    if view.get("status") != "failed":
        pytest.skip("the read completed before the setpoint reached the worker")

    assert view["error"]["code"] == "preempted", \
        "a displaced read must not report as a gear or wire fault: %r" % view["error"]
    after_view = api.attributes(short)["attributes"]
    assert after_view == before_view, \
        "a preempted read committed something: the operator's readings moved"
