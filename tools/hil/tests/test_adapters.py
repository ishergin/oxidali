import pytest

ADAPTER_DISABLED = "adapter_disabled"
HTTP_CONFLICT = 409
QUIET_SETTLE_S = 2.5


@pytest.mark.hil_id("HIL-SYS-02")
def test_controller_and_adapters_consistent(api, test_artifacts):
    ctrl = api.controller()
    adapters = api.adapters()["adapters"]
    test_artifacts.attach_json("controller", ctrl)
    test_artifacts.attach_json("adapters", adapters)
    assert ctrl["adapter_count"] == len(adapters) >= 1
    assert ctrl["firmware_version"] == api.health()["version"]
    a0 = api.adapter_info()
    assert a0 == next(a for a in adapters if a["adapter_id"] == api.adapter)
    assert a0["bus_status"] in ("idle", "disabled")
    for key in ("commands", "timeouts", "errors"):
        assert isinstance(a0["counters"][key], int)


@pytest.mark.hil_id("HIL-SYS-03")
def test_adapter_patch_name_roundtrip(api):
    before = api.adapter_info()
    try:
        patched = api.adapter_patch({"name": "hil-tmp-name"})
        assert patched["name"] == "hil-tmp-name"
        assert api.adapter_info()["name"] == "hil-tmp-name"
    finally:
        api.adapter_patch({"name": before["name"]})
    assert api.adapter_info()["name"] == before["name"]


@pytest.mark.hil_id("HIL-SYS-04")
@pytest.mark.sniffer
def test_adapter_disable_gates_commands(api, lamps, sniffer, paced,
                                        state_snapshot, adapter_enabled_guard,
                                        test_artifacts):
    short = lamps.by_label[lamps.labels()[0]]
    api.adapter_patch({"enabled": False})
    assert api.adapter_info()["enabled"] is False

    with sniffer.window() as win:
        paced(1.0)
        op_view = api.wait_op(api.attr_read(short, groups="runtime_status"))
        ts_status, ts_body = api.raw_request(
            "PUT", "adapters/%d/physical-devices/%d/target-state"
            % (api.adapter, short), {"power": "on", "level": 120})
        dapc_status, dapc_body = api.raw_request(
            "POST", "dali/level", {"wire_address": short << 1, "level": 100})
        observed = {"attr_read_op": op_view,
                    "target_state": {"status": ts_status, "body": ts_body},
                    "dali_level": {"status": dapc_status, "body": dapc_body}}
        test_artifacts.attach_json("disabled_contract", observed)
        win.expect_quiet("DAPC short %d" % short, settle_s=QUIET_SETTLE_S)
    assert op_view.get("status") == "failed", observed
    assert (op_view.get("error") or {}).get("message") == ADAPTER_DISABLED, observed
    assert (ts_status, ts_body.get("error")) == (HTTP_CONFLICT, "conflict"), observed
    assert dapc_body.get("success") is False, observed
    assert dapc_body.get("message") == ADAPTER_DISABLED, observed

    api.adapter_patch({"enabled": True})
    assert api.adapter_info()["enabled"] is True
    setpoint = {"power": "on", "level": 130}
    with sniffer.window() as win:
        paced(1.0)
        api.ts(short, setpoint)
        win.expect_frame("DAPC short %d -> level 130" % short,
                         resend=lambda: api.ts(short, setpoint))
    api.off(short)
