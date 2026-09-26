import time

import pytest

from hil.camera.calibrate import rgb_setpoint, RGB_PRIMARIES

ALL_GROUPS = "runtime_status,common_102,dt8_color,dt6_led,groups,scenes,extended"

FADE_CODE_MS = {0: 0, 1: 700, 2: 1000, 3: 1400, 4: 2000, 5: 2800, 6: 4000,
                7: 5700, 8: 8000, 9: 11300, 10: 16000, 11: 22600, 12: 32000,
                13: 45300, 14: 64000, 15: 90500}


def _attr_section(device, section):
    return (device.get("attributes") or {}).get(section) or {}


def _fade_time_value(device, section="common_102"):
    for key, leaf in _attr_section(device, section).items():
        if "fade_time" in key and isinstance(leaf, dict):
            return key, leaf.get("value")
    return None, None


def _ms_to_code(ms):
    if ms == 0:
        return 0
    return min((c for c in FADE_CODE_MS if c != 0),
               key=lambda code: abs(FADE_CODE_MS[code] - ms))


def _accepted_ms(requested_ms):
    return FADE_CODE_MS[_ms_to_code(requested_ms)]


@pytest.mark.hil_id("HIL-ATTR-01")
@pytest.mark.sniffer
def test_attribute_read_sweep_plausible(api, lamps, sniffer, paced, op_check,
                                        state_snapshot, test_artifacts):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    with sniffer.window() as win:
        paced(1.0)
        op = api.attr_read(short, groups=ALL_GROUPS, banks="identity")
        view = op_check(
            api.wait_op(op, timeout_s=90),
            retry=lambda: api.wait_op(
                api.attr_read(short, groups=ALL_GROUPS, banks="identity"),
                timeout_s=90))
        test_artifacts.attach_json("attr_op", view)
        win.expect_frame("QUERY", timeout_s=5)
    device = api.device_full(short)
    test_artifacts.attach_json("attributes", device.get("attributes"))
    gtin = (_attr_section(device, "memory_identity").get("gtin")
            or {}).get("value")
    assert gtin, "identity bank read produced no GTIN"
    _, fade = _fade_time_value(device)
    if fade is not None:
        assert fade in FADE_CODE_MS.values(), fade
    _, ext_fade = _fade_time_value(device, section="extended")
    if ext_fade is not None:
        assert 0 <= ext_fade <= 65_535, ext_fade
    ext_fade = _attr_section(device, "extended").get("fade_time_ms") or {}
    assert ext_fade.get("last_read_ms") is not None, (
        "extended.fade_time_ms was not read by the full sweep: %r" % (ext_fade,))
    outcomes = view.get("attribute_read_outcomes") or {}
    test_artifacts.attach_json("outcomes", outcomes)


@pytest.mark.hil_id("HIL-ATTR-05")
@pytest.mark.needs_capability("cct")
def test_gear_features_match_the_standards_own_checks(api, lamps, capabilities,
                                                      paced, test_artifacts):
    checked = 0
    for label in lamps.labels():
        short = lamps.by_label[label]
        if not capabilities.ensure(short, "cct") and not capabilities.ensure(short, "rgb"):
            continue
        paced(0.5)
        api.attr_read_checked(short, groups="runtime_status,dt8_color")
        device = api.attributes(short, ["dt8_color"])
        leaf = _attr_section(device, "dt8_color").get("gear_features") or {}
        test_artifacts.attach_json("gear_features_%s" % label, leaf)
        value = leaf.get("value")
        assert value is not None, (
            "%s (short %d) did not answer QUERY GEAR FEATURES/STATUS; "
            "IEC 62386-209 Table 14 makes 247 mandatory on DT8 gear" % (label, short))
        assert value & 0x3E == 0, (
            "%s: reserved bits set in GEAR FEATURES/STATUS 0x%02X "
            "(12.7.1.1 error 7112)" % (label, value))
        assert value & 0x01, (
            "%s: Automatic Activation is clear (0x%02X) — colour writes to this "
            "fixture will be accepted and do nothing (12.7.1.1 error 7113)"
            % (label, value))
        checked += 1
    assert checked, "no colour-capable fixture was available to read 247 from"


@pytest.mark.hil_id("HIL-ATTR-03")
def test_gear_bitmask_covers_applied_matrix(api, vl_bindings, capabilities,
                                            lamps, test_artifacts):
    matrix = api.groups.matrix()
    rows = {r["virtual_lamp_id"]: r for r in matrix["rows"]}
    missing, extra, doubled = [], [], []
    for label in lamps.labels():
        lid, short = label - 1, lamps.by_label[label]
        capabilities.ensure(short, "groups")
        bitmask = (((api.attributes(short, ["groups"]).get("attributes") or {})
                    .get("groups") or {}).get("membership") or {}).get("value")
        if bitmask is None:
            continue
        lo_b, hi_b = bitmask & 0xFF, (bitmask >> 8) & 0xFF
        if hi_b == lo_b and hi_b != 0:
            doubled.append({"vl": lid, "short": short, "bitmask": hex(bitmask)})
        for gid in range(16):
            gear_bit = bool((bitmask >> gid) & 1)
            applied = rows[lid]["applied"][gid]
            if applied and not gear_bit:
                missing.append({"vl": lid, "short": short, "gid": gid})
            elif gear_bit and not applied:
                extra.append({"vl": lid, "short": short, "gid": gid})
    test_artifacts.attach_json("bitmask_vs_matrix",
                               {"missing_in_gear": missing,
                                "extra_in_gear": extra,
                                "doubled_byte": doubled})
    assert not missing, missing
    assert not doubled, doubled


@pytest.mark.hil_id("HIL-ATTR-02")
@pytest.mark.sniffer
def test_write_fade_time_roundtrip(api, lamps, sniffer, paced, state_snapshot,
                                   test_artifacts, attr_guard):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    before = attr_guard(short, "fade_time_ms")["fade_time_ms"]
    for requested_ms in (500, 100, 2000, 90_500):
        expected_ms = _accepted_ms(requested_ms)
        with sniffer.window() as win:
            paced(1.0)
            op = api.write_attrs(short, {"fade_time_ms": requested_ms})
            assert op.get("type") == "attribute_write", op
            view = api.wait_op(op)
            test_artifacts.attach_json("write_op_%d" % requested_ms, view)
            assert view["status"] == "succeeded", view
            win.expect_frame("STORE DTR AS FADE TIME")
        api.attr_read_checked(short, groups="common_102")
        key, after = _fade_time_value(api.attributes(short, ["common_102"]))
        test_artifacts.attach_json("fade_roundtrip_%d" % requested_ms,
                                   {"attr_key": key, "before": before,
                                    "after": after,
                                    "requested_ms": requested_ms,
                                    "expected_ms": expected_ms})
        assert after is not None, "no fade_time attribute after write+read"
        assert after == expected_ms, (requested_ms, expected_ms, after)


@pytest.mark.hil_id("HIL-ATTR-04")
@pytest.mark.sniffer
@pytest.mark.needs_capability("rgb")
def test_color_mode_override_gates_target_state(api, capabilities,
                                                needs_capability, sniffer,
                                                paced, state_snapshot,
                                                test_artifacts):
    short = capabilities.any_lamp_with("rgb")
    before = api.state(short)
    _, red = RGB_PRIMARIES[0]
    setpoint = rgb_setpoint(red)
    try:
        patched = api.device_patch(short, {"name": "hil-pd-tmp",
                                           "notes": "hil override test"})
        assert patched["name"] == "hil-pd-tmp"
        assert patched.get("notes") == "hil override test"

        api.device_patch(short, {"color_mode_override": "cct"})
        assert api.state(short)["color_mode_effective"] == "cct"
        status, body = api.raw_request(
            "PUT", "adapters/%d/physical-devices/%d/target-state"
            % (api.adapter, short), setpoint)
        test_artifacts.attach_json("gated_rgb", {"status": status, "body": body})
        if status == 200:
            api.device_patch(short, {"color_mode_override": None})
            pytest.xfail("color_mode_override does not gate target-state on "
                         "this firmware build (expected 422 "
                         "unsupported_capability)")
        assert status == 422, (status, body)
        assert body.get("error") == "unsupported_capability", body

        api.device_patch(short, {"color_mode_override": None})
        with sniffer.window() as win:
            paced(1.0)
            api.ts(short, setpoint)
            win.expect_frame("DT8 SET TEMP RGB DIM LEVEL")
    finally:
        api.device_patch(short, {
            "name": before.get("name") or None,
            "notes": before.get("notes"),
            "color_mode_override": before.get("color_mode_override")})
        api.off(short)


@pytest.mark.hil_id("HIL-ATTR-07")
@pytest.mark.sniffer
def test_min_level_write_confirms_accepted_value(api, lamps, sniffer, paced,
                                                 state_snapshot,
                                                 test_artifacts, attr_guard):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    before = attr_guard(short, "min_level")["min_level"]
    sec = ((api.attributes(short, ["common_102"]).get("attributes") or {})
           .get("common_102") or {})
    phm = (sec.get("physical_minimum") or {}).get("value") or 1
    target = phm + 9 if before != phm + 9 else phm + 4
    with sniffer.window() as win:
        paced(1.0)
        op = api.write_attrs(short, {"min_level": target})
        assert op.get("type") == "attribute_write", op
        view = api.wait_op(op)
        test_artifacts.attach_json("min_write_op", view)
        assert view["status"] == "succeeded", view
        win.expect_frame("STORE DTR AS MIN LEVEL")
    state = api.attributes(short, ["common_102"])
    leaf = ((state.get("attributes") or {}).get("common_102")
            or {}).get("min_level") or {}
    test_artifacts.attach_json("min_roundtrip",
                               {"before": before, "target": target,
                                "leaf": leaf})
    assert leaf.get("value") == target, leaf
    assert leaf.get("source") == "write_confirmed", leaf


@pytest.mark.hil_id("HIL-ATTR-08")
@pytest.mark.sniffer
def test_min_level_clamp_against_lowered_max_confirms_accepted(
        api, lamps, sniffer, paced, state_snapshot, test_artifacts,
        attr_guard):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    snap = attr_guard(short, "max_level", "min_level")
    before_min, before_max = snap["min_level"], snap["max_level"]
    sec = (api.attributes(short, ["common_102"]).get("attributes") or {}).get("common_102") or {}
    phm = (sec.get("physical_minimum") or {}).get("value") or 1
    lowered_max = max(phm + 20, 100)
    asked_min = min(lowered_max + 50, 254)
    with sniffer.window() as win:
        paced(1.0)
        view = api.wait_op(api.write_attrs(short, {"max_level": lowered_max}))
        assert view["status"] == "succeeded", view
        win.expect_frame("STORE DTR AS MAX LEVEL")
    view = api.wait_op(api.write_attrs(short, {"min_level": asked_min}))
    assert view["status"] == "succeeded", \
        ("a legal §9.6 clamp must not fail the operation", view)
    c102 = ((api.attributes(short, ["common_102"]).get("attributes") or {})
            .get("common_102") or {})
    got_min = (c102.get("min_level") or {}).get("value")
    got_max = (c102.get("max_level") or {}).get("value")
    test_artifacts.attach_json("clamp_roundtrip", {
        "before": {"min": before_min, "max": before_max, "phm": phm},
        "asked": {"max": lowered_max, "min": asked_min},
        "got": {"min": got_min, "max": got_max},
    })
    assert got_max == lowered_max, c102
    assert got_min == lowered_max, \
        ("accepted (clamped to the lowered max), never the request", c102)
    assert (c102.get("min_level") or {}).get("source") == "write_confirmed", c102


def _declared(api, short):
    return api.state(short).get("supported_device_types")


@pytest.mark.smoke
def test_declared_device_types_are_a_set_or_honestly_absent(api, op_check):
    shorts = api.optical_addrs()
    if not shorts:
        pytest.skip("no bench luminaires in the registry")
    for short in shorts:
        op_check(api.attr_read_checked(short, groups="common_102"))
        declared = _declared(api, short)
        assert declared is None or isinstance(declared, list), (
            "device %d: supported_device_types is %r, which is neither a set "
            "nor an honest absence" % (short, declared)
        )
        if isinstance(declared, list):
            assert declared == sorted(set(declared)), (
                "device %d declared %r — a set is ascending and unique"
                % (short, declared)
            )
            assert all(0 <= t <= 52 for t in declared), (
                "device %d declared a type outside the allocated range: %r"
                % (short, declared)
            )


@pytest.mark.smoke
def test_a_widening_device_type_override_is_refused(api, op_check):
    from hil import api as api_mod

    target = None
    for short in api.addrs():
        declared = _declared(api, short)
        if isinstance(declared, list) and 8 not in declared:
            target = (short, declared)
            break
    if target is None:
        pytest.skip(
            "every gear on the segment declares DT8 (or none finished its "
            "walk); there is nothing this rig can widen TO"
        )

    short, declared = target
    before = api.state(short).get("device_type_override")
    try:
        with pytest.raises(api_mod.ApiError) as exc:
            api.device_patch(short, {"device_type_override": "dt8_color"})
        assert exc.value.status == 422, (
            "device %d declares %r and does not support DT8, so the override "
            "must be refused rather than silently enabling frames the gear "
            "ignores; got %s" % (short, declared, exc.value.status)
        )

        api.device_patch(short, {"device_type_override": "unknown"})
        assert api.state(short).get("device_type_override") == "unknown"
    finally:
        api.device_patch(short, {"device_type_override": before})


@pytest.mark.hil_id("HIL-ATTR-09")
@pytest.mark.sniffer
def test_fade_time_actually_lasts_what_it_says(api, lamps, sniffer, paced,
                                               state_snapshot, test_artifacts,
                                               attr_guard):
    label = lamps.labels()[0]
    short = lamps.by_label[label]
    nominal_ms, min_s, max_s = 2000, 1.8, 2.2
    poll_s = 0.12

    attr_guard(short, "fade_time_ms")
    try:
        api.wait_op(api.write_attrs(short, {"fade_time_ms": nominal_ms}))
        api.ts(short, {"power": "on", "level": 254})
        time.sleep(1.0)

        with sniffer.window() as win:
            paced(1.0)
            api.dapc(short, 0)
            started = time.monotonic()
            last_seen_running = started
            first_seen_clear = None
            while time.monotonic() - started < max_s + 1.5:
                body = api.raw(((short << 1) | 1) << 8 | 0x90,
                               expects_backward=True)
                if body.get("success") is not True:
                    continue
                if not body.get("backward_frame", 0) & 0x10:
                    first_seen_clear = time.monotonic() - started
                    break
                last_seen_running = time.monotonic() - started
                time.sleep(poll_s)
            win.expect_frame("DAPC short %d -> level 0" % short)

        test_artifacts.attach_json("fade_duration", {
            "requested_ms": nominal_ms,
            "still_running_at_s": last_seen_running,
            "seen_clear_at_s": first_seen_clear,
            "table4_min_s": min_s, "table4_max_s": max_s,
        })
        assert first_seen_clear is not None, (
            "fadeRunning never cleared within %.1f s of a DAPC to 0 with "
            "fade_time_ms=%d" % (max_s + 1.5, nominal_ms))
        assert last_seen_running <= max_s and first_seen_clear >= min_s, (
            "fade of %d ms landed in [%.2f, %.2f] s, which does not overlap "
            "Table 4's %.1f-%.1f s"
            % (nominal_ms, last_seen_running, first_seen_clear, min_s, max_s))
    finally:
        api.off(short)


@pytest.mark.hil_id("HIL-ATTR-10")
@pytest.mark.sniffer
@pytest.mark.needs_capability("rgb")
def test_rgbwaf_control_byte_before_and_after_a_colour_write(api, capabilities,
                                                             needs_capability,
                                                             state_snapshot,
                                                             test_artifacts):
    short = capabilities.any_lamp_with("rgb")
    rgbwaf_control_selector = 15

    def read_control():
        api.raw(0xA300 | rgbwaf_control_selector)
        api.raw(0xC108)
        body = api.raw(((short << 1) | 1) << 8 | 0xFA, expects_backward=True)
        return body.get("backward_frame") if body.get("success") is True else None

    before = read_control()
    api.ts(short, {"power": "on", "level": 120,
                   "color_mode": "rgb", "rgb": {"r": 0, "g": 0, "b": 254}})
    time.sleep(1.0)
    after = read_control()
    channels = api.rgbwaf_channel_count(short)
    caps = api.state(short).get("capabilities")
    test_artifacts.attach_json("rgbwaf_control", {
        "short": short, "before": before, "after": after,
        "declared_channels": channels, "capabilities": caps,
        "note": "0x80 = normalised colour control, 0xC0 = reserved type; "
                "the product writes 0x80 when dt8_rgbwaf_control_assert is on",
    })

    assert before is not None, (
        "gear did not answer QUERY COLOUR VALUE(15) before the write — the "
        "byte cannot be managed on this fixture at all, which is a fact about "
        "it and belongs on its card")
    assert after is not None, "gear stopped answering QUERY COLOUR VALUE(15)"
    api.off(short)


def _dimming_curve(api, short):
    section = (api.attributes(short, ["dt6_led"]).get("attributes") or {}).get("dt6_led") or {}
    cell = section.get("dimming_curve") or {}
    return cell.get("value"), cell.get("source")


@pytest.mark.hil_id("HIL-ATTR-11")
@pytest.mark.smoke
def test_dimming_curve_writes_and_reports_write_confirmed_provenance(
        api, op_check, attr_guard):
    short = next((s for s in api.optical_addrs()
                  if 6 in (_declared(api, s) or [])), None)
    if short is None:
        pytest.skip("no fixture on this bench declares device type 6")

    before = attr_guard(short, "dimming_curve",
                        default=0, verify=True)["dimming_curve"]
    target = 1 if (before or 0) == 0 else 0
    op_check(
        api.wait_op(api.write_attrs(short, {"dimming_curve": target})),
        retry=lambda: api.wait_op(
            api.write_attrs(short, {"dimming_curve": target})))
    value, source = _dimming_curve(api, short)
    assert value == target, (
        "gear %d kept dimming curve %r after a write of %d — the pair did "
        "not execute (declared DT6, so it should have)" % (short, value, target))
    assert source == "write_confirmed", (
        "dimming curve %d landed but reports provenance %r; a confirmed "
        "write must say so, or the operator cannot tell a write that landed "
        "from a read that happened to agree" % (target, source))


@pytest.mark.smoke
def test_a_reserved_dimming_curve_never_reaches_the_bus(api):
    short = next(iter(api.optical_addrs()), None)
    if short is None:
        pytest.skip("no optical fixture configured")
    status, body = api.raw_request(
        "POST", "adapters/%d/physical-devices/%d/write-attributes"
        % (api.adapter, short), {"dimming_curve": 2})
    assert status == 422, (
        "a reserved dimming-curve value must be refused at ingress, got %d: %r"
        % (status, body))
    assert body.get("error") == "invalid_value", body


@pytest.mark.hil_id("HIL-ATTR-12")
@pytest.mark.sniffer
def test_srgb_channels_reach_the_wire_as_linear_dim_levels(
        api, lamps, capabilities, sniffer, paced, state_snapshot, op_check):
    short = capabilities.any_lamp_with("rgb")
    if short is None:
        pytest.skip("no rgb-capable fixture on this bench")
    before = api.device_full(short).get("state", {})
    try:
        api.ts(short, {"power": "on", "level": 200})
        with sniffer.window() as win:
            paced(1.0)
            api.ts(short, {"color_mode": "rgb", "rgb": {"r": 254, "g": 0, "b": 120}})
            win.expect_frame("DTR0 data=0xFC")
            win.expect_frame("DTR2 data=0x30")
            win.expect_frame("DT8 SET TEMP RGB DIM LEVEL")
        op_check(api.attr_read_checked(short, groups="dt8_color"))
        rgb = api.device_full(short).get("state", {}).get("rgb") or {}
        assert (rgb.get("r"), rgb.get("g"), rgb.get("b")) == (254, 0, 120), (
            "the gear's own channel readback must decode back to the sRGB that "
            "was asked for; got %r" % (rgb,))
    finally:
        api.off(short)
        if before.get("level"):
            api.ts(short, {"power": "on", "level": before["level"]})
            api.off(short)


@pytest.mark.hil_id("HIL-ATTR-13")
def test_a_dt8_colour_read_keeps_its_transactions_inside_the_92_budget(
        api, lamps, capabilities, state_snapshot, test_artifacts):
    short = next(
        (s for s in (lamps.by_label[l] for l in lamps.labels())
         if capabilities.ensure(s, "cct") or capabilities.ensure(s, "rgb")),
        None,
    )
    if short is None:
        pytest.skip("no DT8 gear on this bench — no colour bracket to measure")

    before = api.diagnostics().get("dali_wire", {})
    api.attr_read_checked(short, groups="runtime_status,dt8_color")
    after = api.diagnostics().get("dali_wire", {})

    delta = {
        key: after.get(key, 0) - before.get(key, 0)
        for key in ("transactions_started", "transactions_completed",
                    "transaction_budget_exceeded",
                    "transaction_should_exceedances", "transaction_leaks")
    }
    test_artifacts.attach_json("dt8_read_transactions", delta)

    assert delta["transactions_started"] > 0, (
        "a DT8 colour read is a series of transactions; none was recorded — "
        "the read did not reach the wire, so nothing below means anything"
    )
    assert delta["transaction_budget_exceeded"] == 0, (
        "the DT8 sample bracket ran past §9.2's 400 ms: it is five exchanges "
        "since ISSUE-58 п.5, and this is the measurement that says whether "
        "five fits. Shorten the unit — do not widen the guidance"
    )
    assert delta["transaction_leaks"] == 0, (
        "a transaction outlived its command; priority 1 was armed outside one"
    )
