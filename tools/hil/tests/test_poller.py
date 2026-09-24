import time

import pytest

from hil.wait import wait_until

FAST_INTERVAL_MS = 1000


def _bound_short(api):
    for lamp in api.vlamps.list()["virtual_lamps"]:
        binding = lamp.get("binding") or {}
        short = binding.get("physical_short_address")
        if short is not None:
            return short
    return None


@pytest.mark.hil_id("HIL-POL-01")
@pytest.mark.smoke
def test_settings_contract_holds_on_real_firmware(api, poller_guard):
    dto = api.poller.get()
    assert set(dto) == {
        "enabled", "interval_ms", "attribute_groups_default",
        "include_dt8_color", "include_energy", "include_diagnostics",
        "skip_unbound_virtual_lamps"}, dto
    assert isinstance(dto["attribute_groups_default"], list), dto
    assert dto["attribute_groups_default"], "the group list may never be empty"

    for body, expected in (({"interval_ms": 100}, 422),
                           ({"interval_ms": 3_600_001}, 422),
                           ({"max_concurrent": 2}, 400),
                           ({"attribute_groups_default": []}, 422),
                           ({"attribute_groups_default": ["nonsense"]}, 422),
                           ({"backoff_ms": 1000}, 400)):
        status, _ = api.raw_request("PATCH", "settings/poller", body)
        assert status == expected, \
            "PATCH %r -> %d, expected %d" % (body, status, expected)
    assert api.poller.get() == dto, "a rejected PATCH changed the settings"


@pytest.mark.hil_id("HIL-POL-02")
@pytest.mark.smoke
def test_disabled_poller_still_ticks_but_reads_nothing(api, poller_guard,
                                                       poller_counters):
    poller_guard(enabled=False)
    time.sleep(2.0)

    cycles = poller_counters.get("cycles_total")
    published = poller_counters.get("reads_published")
    assert poller_counters.wait("cycles_total", cycles + 2, timeout_s=180), \
        "a disabled poller must keep ticking; counters=%s" % poller_counters.all()
    assert poller_counters.get("reads_published") == published, \
        "a disabled poller published reads: %s" % poller_counters.all()


def _wait_for_fresh_observation(api, short, seen_before, timeout_s=90):
    state = {}

    def _observed():
        nonlocal state
        state = api.state(short).get("state") or {}
        return state.get("last_seen_ms") != seen_before

    if wait_until(_observed, timeout_s, interval_s=1):
        return state
    raise AssertionError(
        "short %d was never re-observed within %d s (rotation or cadence stalled): %r"
        % (short, timeout_s, state))


@pytest.mark.hil_id("HIL-POL-03")
def test_enabled_poller_reads_real_gear_as_poller_source(api, poller_guard,
                                                         poller_counters,
                                                         state_snapshot,
                                                         test_artifacts):
    short = _bound_short(api)
    if short is None:
        pytest.skip("no virtual lamp is bound — the poller would skip every device")

    completed = poller_counters.get("reads_completed")
    failed = poller_counters.get("reads_failed")
    seen_before = (api.state(short).get("state") or {}).get("last_seen_ms")
    poller_guard(enabled=True, interval_ms=FAST_INTERVAL_MS)
    assert poller_counters.wait("reads_completed", completed + 1, timeout_s=60), \
        "no poller read completed on real gear: %s" % poller_counters.all()

    state = _wait_for_fresh_observation(api, short, seen_before)
    test_artifacts.attach_json("state", state)
    test_artifacts.attach_json("counters", poller_counters.all())
    assert state.get("value_source") == "poller", state
    assert poller_counters.get("reads_failed") == failed, \
        "a read failed against real gear while this test ran: %s" \
        % poller_counters.all()


@pytest.mark.hil_id("HIL-POL-04")
def test_unbound_devices_are_skipped_when_configured(api, poller_guard,
                                                     poller_counters,
                                                     test_artifacts):
    bound = {l.get("binding", {}).get("physical_short_address")
             for l in api.vlamps.list()["virtual_lamps"] if l.get("binding")}
    unbound = [d["short_address"] for d in api.devices()["physical_devices"]
               if d["short_address"] not in bound]
    if not unbound:
        pytest.skip("every device on the rig is bound — nothing to skip")

    poller_guard(enabled=True, interval_ms=FAST_INTERVAL_MS,
                 skip_unbound_virtual_lamps=True)
    assert poller_counters.wait("targets_excluded", len(unbound),
                                timeout_s=60), \
        "unbound devices were not excluded: %s" % poller_counters.all()
    test_artifacts.attach_json("unbound", unbound)
    test_artifacts.attach_json("counters", poller_counters.all())


@pytest.mark.hil_id("HIL-POL-05")
def test_poller_reads_create_no_operation_rows(api, poller_guard,
                                               poller_counters,
                                               test_artifacts):
    if _bound_short(api) is None:
        pytest.skip("no bound lamp — the poller would issue no reads at all")

    before = set(api.operations())
    completed = poller_counters.get("reads_completed")
    poller_guard(enabled=True, interval_ms=FAST_INTERVAL_MS)
    assert poller_counters.wait("reads_completed", completed + 2, timeout_s=90), \
        "poller did not complete two reads: %s" % poller_counters.all()

    new_rows = [op for op in api.operations() if op not in before]
    test_artifacts.attach_json("new_operations", new_rows)
    assert not new_rows, "the poller created operation rows: %s" % new_rows


@pytest.mark.hil_id("HIL-POL-06")
def test_live_disable_stops_reads_without_restarting_the_worker(
        api, poller_guard, poller_counters, test_artifacts):
    if _bound_short(api) is None:
        pytest.skip("no bound lamp — nothing to stop")

    completed = poller_counters.get("reads_completed")
    poller_guard(enabled=True, interval_ms=FAST_INTERVAL_MS)
    assert poller_counters.wait("reads_completed", completed + 1, timeout_s=60), \
        "poller never started reading: %s" % poller_counters.all()

    cycles_before = poller_counters.get("cycles_total")
    poller_guard(enabled=False)
    time.sleep(FAST_INTERVAL_MS / 1000.0 * 3)

    assert poller_counters.quiet("reads_published", 6.0), \
        "reads continued after disable: %s" % poller_counters.all()
    after = poller_counters.get("cycles_total")
    test_artifacts.attach_json("counters", poller_counters.all())
    assert after > cycles_before, \
        "cycle counter did not advance — the worker restarted or died: %d -> %d" \
        % (cycles_before, after)


@pytest.mark.hil_id("HIL-POL-07")
def test_poller_settings_survive_reboot(api, poller_guard, dut_reboot,
                                        test_artifacts):
    poller_guard(enabled=False, interval_ms=45000,
                 attribute_groups_default=["runtime_status", "common_102"])
    before = api.poller.get()
    dut_reboot()
    after = api.poller.get()
    test_artifacts.attach_json("before", before)
    test_artifacts.attach_json("after", after)
    for field in ("enabled", "interval_ms",
                  "attribute_groups_default", "include_dt8_color",
                  "skip_unbound_virtual_lamps"):
        assert after[field] == before[field], \
            "%s did not survive the reboot: %r -> %r" % (field, before[field],
                                                         after[field])


@pytest.mark.hil_id("HIL-POL-08")
def test_classification_survives_reboot_and_polling_does_not_revoke_it(
        api, poller_guard, poller_counters, dut_reboot, test_artifacts):
    short = _bound_short(api)
    if short is None:
        pytest.skip("no bound lamp — the poller would read nothing")

    dut_reboot()
    restored = next(d for d in api.devices()["physical_devices"]
                    if d["short_address"] == short)
    test_artifacts.attach_json("after_reboot", restored)
    assert restored.get("device_type_effective") not in (None, "unknown"), \
        "classification did not survive the reboot: %s" % restored
    caps_before = restored.get("capabilities", {})

    poller_guard(enabled=True, interval_ms=FAST_INTERVAL_MS,
                 attribute_groups_default=["runtime_status", "common_102"])
    completed = poller_counters.get("reads_completed")
    assert poller_counters.wait("reads_completed", completed + 2, timeout_s=120), \
        "poller did not read after reboot: %s" % poller_counters.all()

    device = next(d for d in api.devices()["physical_devices"]
                  if d["short_address"] == short)
    test_artifacts.attach_json("after_polling", device)
    assert device.get("device_type_effective") == restored.get("device_type_effective"), \
        "polling changed the device type: %s" % device
    for cap, was in caps_before.items():
        if was:
            assert device.get("capabilities", {}).get(cap), \
                "polling revoked capability %r: %s" % (cap, device)


@pytest.mark.hil_id("HIL-POL-09")
@pytest.mark.slow
@pytest.mark.foreign
def test_sustained_poll_under_foreign_traffic(api, poller_guard,
                                              poller_counters, foreign,
                                              test_artifacts):
    if _bound_short(api) is None:
        pytest.skip("no bound lamp — nothing to poll")

    run_s = 300
    uptime_before = api.health()["uptime_seconds"]
    dali_before = api.diagnostics()["dali_worker"]
    counters_before = poller_counters.all()

    poller_guard(enabled=True, interval_ms=2000)
    deadline = time.monotonic() + run_s
    while time.monotonic() < deadline:
        assert api.health()["uptime_seconds"] >= uptime_before, \
            "the DUT rebooted mid-run — uptime went backwards"
        time.sleep(10)

    counters_after = poller_counters.all()
    dali_after = api.diagnostics()["dali_worker"]
    delta = {k: poller_counters.delta(counters_before.get(k, 0), v)
             for k, v in counters_after.items()}
    test_artifacts.attach_json("poller_delta", delta)
    test_artifacts.attach_json("dali_worker", {
        k: dali_after[k] - dali_before.get(k, 0) for k in dali_after})

    assert delta["reads_completed"] > 0, \
        "no read completed in %ds under foreign traffic: %s" % (run_s, delta)
    assert delta["device_cooldowns"] <= delta["reads_completed"], \
        "cooldowns outpaced completions — the bus is not usable: %s" % delta
    assert api.health()["uptime_seconds"] >= uptime_before + run_s * 0.8, \
        "the DUT did not stay up for the whole run"


@pytest.mark.hil_id("HIL-POL-10")
@pytest.mark.slow
def test_poller_never_delays_an_interactive_put(api, poller_guard, poller_counters,
                                                test_artifacts):
    short = _bound_short(api)
    if short is None:
        pytest.skip("no virtual lamp is bound — the poller would skip every device")

    poller_guard(enabled=True, interval_ms=200, include_dt8_color=True)
    assert poller_counters.wait("reads_published", poller_counters.get("reads_published") + 2,
                                timeout_s=60), poller_counters.all()

    samples, failures = [], []
    for i in range(20):
        started = time.time()
        status, payload = api.raw_request(
            "PUT", "adapters/%d/physical-devices/%d/target-state" % (api.adapter, short),
            {"power": "on", "level": 200 if i % 2 else 120})
        elapsed_ms = (time.time() - started) * 1000
        samples.append(elapsed_ms)
        if status != 200:
            failures.append((status, payload, round(elapsed_ms)))

    counters = poller_counters.all()
    test_artifacts.attach_json("put_latency_ms", {
        "samples": [round(s) for s in samples],
        "max": round(max(samples)),
        "failures": failures,
    })
    test_artifacts.attach_json("poller_counters", counters)

    assert not failures, \
        "an interactive PUT failed while the poller was running: %r" % failures
    assert max(samples) < 1000, \
        "worst PUT %d ms — the poller is still holding the bus: %s" % (
            round(max(samples)), counters)
    assert counters["reads_published"] > 0, counters
