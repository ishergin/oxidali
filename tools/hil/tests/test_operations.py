import time

import pytest

from hil.wait import wait_until

STATUS_ORDER = {"accepted": 0, "running": 1,
                "succeeded": 2, "failed": 2, "timed_out": 2, "cancelled": 2}


TERMINAL = {"succeeded", "failed", "timed_out", "cancelled"}


@pytest.mark.hil_id("HIL-OP-01")
def test_operations_listed_and_forward_only(api, lamps, state_snapshot,
                                            test_artifacts):
    labels = lamps.labels()
    short_a = lamps.by_label[labels[0]]
    short_b = lamps.by_label[labels[-1]]
    attempts = []
    for attempt in (1, 2):
        op_a = api.attr_read(short_a, groups="runtime_status,common_102")
        op_b = api.attr_read(short_b, groups="runtime_status")
        keys = api.operations()
        assert op_a["operation_id"] in keys and op_b["operation_id"] in keys, keys

        seen = {op_a["operation_id"]: [], op_b["operation_id"]: []}

        def _all_terminal():
            pending = False
            for op_id, trail in seen.items():
                status, view = api.raw_request("GET", "operations/%s" % op_id)
                if status != 200:
                    continue
                state = view.get("status")
                if not trail or trail[-1] != state:
                    trail.append(state)
                if STATUS_ORDER.get(state, 0) < 2:
                    pending = True
            return not pending

        wait_until(_all_terminal, 45, interval_s=0.5)
        attempts.append(seen)
        if seen[op_b["operation_id"]] and \
                seen[op_b["operation_id"]][-1] == "succeeded":
            break
    test_artifacts.attach_json("status_trails", attempts)
    for op_id, trail in seen.items():
        assert trail and trail[-1] in TERMINAL, (op_id, trail)
        ranks = [STATUS_ORDER[s] for s in trail]
        assert ranks == sorted(ranks), "status went backwards: %s" % trail
    assert seen[op_b["operation_id"]][-1] == "succeeded", attempts


@pytest.mark.hil_id("HIL-OP-02")
def test_finished_operation_evicted(api, lamps, test_artifacts, op_check):
    short = lamps.by_label[lamps.labels()[0]]
    op = api.attr_read(short, groups="runtime_status")
    view = api.wait_op(op)
    view = op_check(view)
    finished_at = time.time()

    time.sleep(max(0.0, 55 - (time.time() - finished_at)))
    status_55, _ = api.raw_request("GET", "operations/%s" % op["operation_id"])
    time.sleep(max(0.0, 70 - (time.time() - finished_at)))
    status_70, body_70 = api.raw_request("GET",
                                         "operations/%s" % op["operation_id"])
    listed_70 = op["operation_id"] in api.operations()
    test_artifacts.attach_json("eviction", {
        "at_55s": status_55, "at_70s": status_70, "listed_at_70s": listed_70,
        "body_70": body_70})
    assert status_55 == 200, "finished op evicted before 55 s"
    assert status_70 == 404, "finished op still readable after 70 s"
    assert not listed_70


@pytest.mark.hil_id("HIL-OP-03")
def test_untracked_commands_create_no_operations(api, lamps, state_snapshot):
    short = lamps.by_label[lamps.labels()[0]]
    before = set(api.operations())
    api.ts(short, {"power": "on", "level": 140})
    api.dapc(short, 90)
    api.off(short)
    after = set(api.operations())
    assert after - before == set(), after - before


@pytest.mark.hil_id("HIL-OP-04")
@pytest.mark.sniffer
def test_group_apply_paced_no_ring_loss(api, vl_bindings, lamps, free_group,
                                        group_matrix_guard, sniffer,
                                        ops_quiesce, state_snapshot,
                                        test_artifacts, op_check):
    api.groups.join([label - 1 for label in lamps.labels()[:4]], free_group,
                    apply=False)
    with sniffer.window() as win:
        op = api.groups.apply()
        assert "operation_id" in op, op
        view = api.wait_op(op, timeout_s=90)
        view = op_check(view)
        time.sleep(1.0)
        stats = win.stats()
    test_artifacts.attach_json("apply_window_stats", stats)
    assert stats["counter_missed"] == 0, stats
