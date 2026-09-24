
import re
import time

import pytest

from hil.wait import wait_until


@pytest.mark.hil_id("HIL-INP-01")
@pytest.mark.smoke
def test_a_scan_finds_the_panel_and_reads_its_declarations(api, panel, op_check):
    op_check(api.wait_op(api.input_scan()))

    device = api.input_device(panel)
    assert device["present"] is True, (
        "the panel did not answer its own scan: %r" % device)
    assert device["last_seen_ms"], "presence with no timestamp is not evidence"
    for field in ("device_capabilities", "device_status", "version_number"):
        assert device.get(field) is not None, (
            "%s went unread — the scan reached the device but not its "
            "declarations: %r" % (field, device))
    instances = device["instances"]
    assert len(instances) == device["instance_count"] >= 1
    for inst in instances:
        assert inst["instance_status"] is not None, (
            "instance %d has no status after a scan: %r"
            % (inst["instance_number"], inst))
        if inst["instance_type"] == 1:
            assert inst["resolution"] is None, (
                "a push button has no resolution; QUERY RESOLUTION is for the "
                "types that carry one, and asking it here is a frame into "
                "nothing: %r" % inst)


@pytest.mark.hil_id("HIL-INP-02")
@pytest.mark.smoke
def test_an_idempotent_instance_write_proves_its_operand_and_reads_back(
        api, panel, op_check):
    before = api.input_device(panel)
    inst = before["instances"][0]
    stored = (inst["instance_groups"][0] or {}).get("value")
    if stored is None:
        pytest.skip(
            "instance 0 has no stored group membership to rewrite; every other "
            "field on this path would CHANGE the panel, and this suite does not "
            "write the owner's NVM to make a test possible")

    op_check(api.wait_op(api.input_instance_patch(
        panel, inst["instance_number"], {"instance_groups": [stored]})))

    after = api.input_device(panel)["instances"][0]
    cell = after["instance_groups"][0] or {}
    assert cell.get("value") == stored, (
        "group membership moved on an idempotent write: %r -> %r"
        % (stored, cell))
    assert cell.get("read_at_ms"), (
        "no read-back timestamp: the write reported success without the "
        "sequence's own proof frame ever answering — %r" % cell)


@pytest.mark.hil_id("HIL-INP-03")
@pytest.mark.sniffer
def test_the_instance_write_goes_out_as_24_bit_frames(api, panel, op_check, sniffer):
    before = api.input_device(panel)
    inst = before["instances"][0]
    stored = (inst["instance_groups"][0] or {}).get("value")
    if stored is None:
        pytest.skip("no idempotent field on instance 0 — see HIL-INP-02")

    with sniffer.window() as win:
        op_check(api.wait_op(api.input_instance_patch(
            panel, inst["instance_number"], {"instance_groups": [stored]})))
        wide = [f for f in win.frames() if f.get("len_bits") == 24]

    assert wide, (
        "no 24-bit frame in the window: the write either never reached the "
        "wire or went out as a control-gear frame, which no control device "
        "would execute; saw %r"
        % [f["decoded"] for f in win.frames()][-8:])


@pytest.mark.hil_id("HIL-INP-04")
@pytest.mark.smoke
def test_feedback_is_probed_and_reported_honestly(api, panel, op_check):
    op_check(api.wait_op(api.input_scan()))

    inst = api.input_device(panel)["instances"][0]
    fb = inst.get("feedback") or {}
    assert fb.get("probed") is True, (
        "feedback was never probed by a scan, so `present: false` on the card "
        "is a default rather than an answer: %r" % fb)
    if not fb.get("present"):
        pytest.skip(
            "this panel reports no Part 332 feedback (probed and answered no) — "
            "the write half of R16-D needs an indicator this bench does not have")
    assert fb.get("capability") is not None, (
        "a present feedback instance with no capability byte: %r" % fb)


FREE_SHORT_SEARCH = range(63, -1, -1)

SHORT_PRESS_INFO = 0x002


@pytest.fixture()
def free_input_short(api):
    taken = {d["short_address"]
             for d in (api.input_devices().get("input_devices") or [])}
    for candidate in FREE_SHORT_SEARCH:
        if candidate not in taken:
            return candidate
    pytest.skip("every control-device short address is taken — no address to "
                "inject from without forging a real panel's events")


def _translator(api):
    return api.diagnostics()["sniffer_translator"]


def _delta(before, after, key):
    return (after[key] - before[key]) % (1 << 32)


@pytest.mark.hil_id("HIL-INP-05")
@pytest.mark.foreign
def test_an_injected_event_frame_is_received_and_decoded(
        api, foreign, free_input_short, test_artifacts):
    before_phy = api.diagnostics()["phy_sniffer"]
    before = _translator(api)
    foreign.event24(free_input_short, 0, SHORT_PRESS_INFO)
    wait_until(
        lambda: api.diagnostics()["phy_sniffer"]["forward24"]
        != before_phy["forward24"],
        timeout_s=10.0, interval_s=0.5)
    after_phy = api.diagnostics()["phy_sniffer"]
    after = _translator(api)
    test_artifacts.attach_json("counters", {
        "forward24": _delta(before_phy, after_phy, "forward24"),
        "generic": _delta(before, after, "input_events_generic"),
        "typed": _delta(before, after, "input_events_typed"),
        "ambiguous": _delta(before, after, "input_events_ambiguous_scheme"),
    })

    assert _delta(before_phy, after_phy, "forward24") >= 1, (
        "the DUT's PHY received no 24-bit frame: the WB accepted the "
        "injection and the wire stayed empty, or the receiver dropped it")
    assert _delta(before, after, "input_events_generic") >= 1, (
        "no event message was decoded — the frame arrived and the translator "
        "read it as something other than an INPUT NOTIFICATION")
    assert _delta(before, after, "input_events_ambiguous_scheme") == 0, (
        "the source scheme carried no device identity; this frame is scheme "
        "2, which does, so either the encoding is wrong or a device on this "
        "segment reverted to scheme 0 mid-test (103 §9.6.2)")


RULE_LEVEL = 173

POST_COMMISSION_SCAN_ATTEMPTS = 3
POST_COMMISSION_SETTLE_S = 3.0


FORBIDDEN_LAMPS = frozenset({4, 5, 6})


@pytest.fixture()
def bound_lamp(api):
    for lamp in api.vlamps.list()["virtual_lamps"]:
        short = (lamp.get("binding") or {}).get("physical_short_address")
        lamp_id = lamp["virtual_lamp_id"]
        if short is None:
            continue
        if short in FORBIDDEN_LAMPS or lamp_id in FORBIDDEN_LAMPS:
            continue
        return lamp_id, short
    pytest.skip("no drivable virtual lamp is bound to a physical device — "
                "every binding is either absent or one of the lamps this "
                "module must not touch (%s)" % sorted(FORBIDDEN_LAMPS))


@pytest.mark.hil_id("HIL-INP-06")
@pytest.mark.foreign
def test_an_injected_event_activates_a_rule_and_reaches_the_gear(
        api, foreign, free_input_short, bound_lamp, rules_guard,
        state_snapshot, wait_state, test_artifacts):
    lamp_id, short = bound_lamp
    rules_guard(
        'rule "hil-inp-06" {\n'
        '  when input(dev=%d, inst=0) is event(data=%d)\n'
        '  do   lamp(%d).on(level=%d)\n'
        '}' % (free_input_short, SHORT_PRESS_INFO, lamp_id, RULE_LEVEL))

    api.off(short)
    wait_state(short, lambda s: not s.get("is_on"))

    before = api.diagnostics()["rules"]
    foreign.event24(free_input_short, 0, SHORT_PRESS_INFO)
    last = wait_state(short, lambda s: s.get("level") == RULE_LEVEL,
                      timeout_s=10.0)
    after = api.diagnostics()["rules"]
    test_artifacts.attach_json("activation", {
        "state": last,
        "effects_published": _delta(before, after, "effects_published"),
        "activations": _delta(before, after, "activations_published"),
    })

    assert last.get("level") == RULE_LEVEL, (
        "the rule did not reach the gear: lamp %d is %r after an injected "
        "event its trigger names. `rules.effects_published` moved by %d — a "
        "zero there puts the break above the executor (translator, funnel or "
        "engine), a non-zero one below it (the bus, the worker or the wire)"
        % (short, last, _delta(before, after, "effects_published")))


@pytest.mark.hil_id("HIL-INP-07")
@pytest.mark.sniffer
def test_a_rule_drives_part_332_indication_on_the_wire(
        api, panel, rules_guard, sniffer, op_check, test_artifacts):
    inst = api.input_device(panel)["instances"][-1]
    number = inst["instance_number"]
    feedback = inst.get("feedback") or {}
    if not feedback.get("present"):
        pytest.skip("this panel reports no Part 332 feedback — there is no "
                    "indicator for a drive to reach")

    rules_guard(
        'rule "hil-inp-07-on" {\n'
        '  when http trigger\n'
        '  do   input(dev=%d, inst=%d).feedback.on()\n'
        '}\n\n'
        'rule "hil-inp-07-off" {\n'
        '  when http trigger\n'
        '  do   input(dev=%d, inst=%d).feedback.off()\n'
        '}' % (panel, number, panel, number))

    expected_on = "%02X%02X%02X" % ((panel << 1) | 1, 0x20 | number, 0x10)
    try:
        with sniffer.window() as win:
            op_check(api.wait_op(api.rules_run("hil-inp-07-on")))
            win.expect_frame("forward24")
            wide = [f["bytes"] for f in win.frames() if f.get("len_bits") == 24]
        test_artifacts.attach_json("wire", {"expected": expected_on,
                                            "seen": wide})
        assert expected_on in wide, (
            "no ACTIVATE frame for instance %d of panel %d: expected %s, the "
            "window carried %r. A 24-bit frame that is not this one is the "
            "wrong address, the wrong instance form or the wrong opcode — the "
            "three things this encoding gets wrong independently"
            % (number, panel, expected_on, wide))
    finally:
        op_check(api.wait_op(api.rules_run("hil-inp-07-off")))


def _wire_frames(win):
    frames = win.frames()
    counters = [f["counter"] for f in frames]
    wrapped = bool(counters) and (max(counters) - min(counters)) > 0x8000

    def key(f):
        c = f["counter"]
        return c + 0x10000 if wrapped and c < 0x8000 else c

    return [(key(f), f.get("len_bits"), f["bytes"]) for f in sorted(frames, key=key)]


def _tap_gaps(ordered):
    seen = [c for c, _, _ in ordered]
    return [c for a, b in zip(seen, seen[1:]) for c in range(a + 1, b)]



TRIGGER_RE = re.compile(r"when\s+input\(([^)]*)\)")


def _keyed_instances(source, device):
    keyed = set()
    for args in TRIGGER_RE.findall(source):
        dev = re.search(r"dev=(\d+)", args)
        inst = re.search(r"inst=(\d+)", args)
        if dev is None or inst is None:
            return None
        if int(dev.group(1)) == device:
            keyed.add(int(inst.group(1)))
    return keyed


@pytest.fixture()
def unkeyed_instance(api, panel):
    device = api.input_device(panel)
    instances = [i["instance_number"] for i in device["instances"]]
    keyed = _keyed_instances(api.rules_get().get("source") or "", panel)
    if keyed is None:
        pytest.skip(
            "a rule triggers on an input this cannot attribute to a device "
            "number, so no instance of panel %d can be shown to be unkeyed; "
            "injecting would risk firing a rule nobody asked to fire" % panel)
    free = [i for i in instances if i not in keyed]
    if not free:
        pytest.skip(
            "every instance of panel %d has a rule keyed on its events (%s) — "
            "a forged event would fire it, and to this bus a forged press is "
            "a press" % (panel, sorted(keyed)))
    return max(free)


@pytest.mark.hil_id("HIL-INP-08")
@pytest.mark.foreign
def test_a_scheme_2_event_is_retyped_by_the_registry(
        api, foreign, panel, unkeyed_instance, test_artifacts):
    before = _translator(api)
    foreign.button24(panel, unkeyed_instance, "short_press")

    def runtime():
        for inst in api.input_device(panel)["instances"]:
            if inst["instance_number"] == unkeyed_instance:
                return inst.get("runtime") or {}
        return {}

    wait_until(lambda: runtime().get("last_event_info") == SHORT_PRESS_INFO,
               timeout_s=10.0, interval_s=0.5)
    inst = runtime()
    after = _translator(api)
    test_artifacts.attach_json("retyped", {
        "runtime": inst,
        "generic": _delta(before, after, "input_events_generic"),
        "ambiguous": _delta(before, after, "input_events_ambiguous_scheme"),
    })

    assert inst.get("last_event_info") == SHORT_PRESS_INFO, (
        "the registry did not record the event against instance %d of panel "
        "%d: %r" % (unkeyed_instance, panel, inst))
    assert inst.get("last_event_at_ms"), (
        "an event with no timestamp is not evidence it arrived: %r" % inst)
    assert inst.get("event_count"), (
        "an event recorded without being counted: %r" % inst)
    assert _delta(before, after, "input_events_generic") >= 1, (
        "scheme 2 arrives untyped and must be counted as such: the "
        "enrichment names the event without rewriting what the decoder saw")
    assert _delta(before, after, "input_events_ambiguous_scheme") == 0, (
        "scheme 2 names its device, so this frame must not be counted "
        "ambiguous: %r" % inst)


@pytest.mark.hil_id("HIL-INP-09")
@pytest.mark.sniffer
def test_commissioning_opens_and_closes_its_session(
        api, panel, sniffer, op_check, test_artifacts):
    with sniffer.window() as win:
        op_check(api.wait_op(api.input_commission(include_addressed=False)))
        time.sleep(2.0)
        ordered = _wire_frames(win)
    wide = [b for _, bits, b in ordered if bits == 24]
    gaps = _tap_gaps(ordered)
    test_artifacts.attach_json("bracket", {"frames": wide, "tap_gaps": gaps})

    order = ["FFFE1D", "C1017F", "C10200", "C10300", "C10000", "FFFE1E"]
    names = ["START QUIESCENT MODE", "INITIALISE(unaddressed)", "RANDOMISE",
             "COMPARE", "TERMINATE", "STOP QUIESCENT MODE"]
    at = -1
    for frame, name in zip(order, names):
        if frame not in wide:
            assert not gaps, (
                "%s (%s) is not in the window, and the tap dropped %d frame(s) "
                "in the same span (counters %r) — this is the sniffer losing "
                "frames under load, not the DUT failing to send them. Re-run; "
                "if it repeats with NO gaps it is ours. Saw %r"
                % (name, frame, len(gaps), gaps, wide))
            pytest.fail("%s (%s) never went out, and the tap delivered an "
                        "unbroken counter sequence — so it was not sent: %r"
                        % (name, frame, wide), pytrace=False)
        position = wide.index(frame)
        assert position > at, (
            "%s (%s) is out of order — the bracket must open with quiescent "
            "mode and close with TERMINATE then STOP: %r" % (name, frame, wide))
        at = position

    device, view = {}, {}
    for attempt in range(POST_COMMISSION_SCAN_ATTEMPTS):
        view = api.wait_op(api.input_scan())
        if view.get("status") == "succeeded":
            device = api.input_device(panel)
            if device.get("present") is True:
                break
        if attempt + 1 < POST_COMMISSION_SCAN_ATTEMPTS:
            api.retries["input_post_commission_settle"] += 1
            time.sleep(POST_COMMISSION_SETTLE_S)

    assert view.get("status") == "succeeded", (
        "the scan after commissioning never succeeded in %d attempts: %r. "
        "This is the segment's own recovery window if it clears, and ours if "
        "it does not" % (POST_COMMISSION_SCAN_ATTEMPTS, view))
    assert device.get("present") is True, (
        "panel %d does not answer after a commissioning run: the session was "
        "left open and its devices are withdrawn — up to fifteen minutes of a "
        "segment that looks empty and is not (%r)" % (panel, device))


@pytest.mark.hil_id("HIL-INP-10")
def test_the_button_vocabulary_reaches_a_rule_from_a_real_panel(
        api, foreign, panel, unkeyed_instance, bound_lamp, rules_guard,
        state_snapshot, wait_state, test_artifacts):
    lamp_id, short = bound_lamp
    rules_guard(
        'rule "hil-inp-10" {\n'
        '  when input(dev=%d, inst=%d) is short_press\n'
        '  do   lamp(%d).on(level=%d)\n'
        '}' % (panel, unkeyed_instance, lamp_id, RULE_LEVEL))

    api.off(short)
    wait_state(short, lambda s: not s.get("is_on"))

    before = api.diagnostics()["rules"]
    foreign.button24(panel, unkeyed_instance, "short_press")
    last = wait_state(short, lambda s: s.get("level") == RULE_LEVEL,
                      timeout_s=10.0)
    after = api.diagnostics()["rules"]
    test_artifacts.attach_json("activation", {
        "state": last,
        "effects_published": _delta(before, after, "effects_published"),
    })

    assert last.get("level") == RULE_LEVEL, (
        "a rule keyed on `short_press` did not fire for a real press on panel "
        "%d instance %d. The event arrives generic by design, so a zero here "
        "with the registry's own record present means the funnel stopped "
        "taking the instance type from the enumeration: %r"
        % (panel, unkeyed_instance, last))


@pytest.mark.hil_id("HIL-INP-11")
@pytest.mark.foreign
def test_a_power_notification_is_decoded_as_a_lifecycle_fact(
        api, foreign, free_input_short, test_artifacts):
    before = _translator(api)
    foreign.raw24(0xFE, 0xE0, 0x40 | free_input_short)
    wait_until(
        lambda: _delta(before, _translator(api), "input_lifecycle") >= 1,
        timeout_s=10.0, interval_s=0.5)
    after = _translator(api)
    test_artifacts.attach_json("counters", {
        "lifecycle": _delta(before, after, "input_lifecycle"),
        "generic": _delta(before, after, "input_events_generic"),
        "typed": _delta(before, after, "input_events_typed"),
    })

    assert _delta(before, after, "input_lifecycle") >= 1, (
        "the power notification was not recognised as one")
    assert _delta(before, after, "input_events_generic") == 0, (
        "a power notification was counted as an instance event, which means "
        "the decoder read its address byte as a scheme-0 source — the exact "
        "collision Table 20's ordering exists to avoid")


@pytest.mark.hil_id("HIL-INP-12")
@pytest.mark.foreign
def test_a_scheme_0_event_is_counted_as_unattributable(
        api, foreign, test_artifacts):
    before = _translator(api)
    foreign.raw24(0x80 | (1 << 1), 0x80, SHORT_PRESS_INFO)
    wait_until(
        lambda: _delta(before, _translator(api),
                       "input_events_ambiguous_scheme") >= 1,
        timeout_s=10.0, interval_s=0.5)
    after = _translator(api)
    test_artifacts.attach_json("counters", {
        "ambiguous": _delta(before, after, "input_events_ambiguous_scheme"),
        "generic": _delta(before, after, "input_events_generic"),
        "typed": _delta(before, after, "input_events_typed"),
    })

    assert _delta(before, after, "input_events_ambiguous_scheme") >= 1, (
        "a scheme-0 event was not counted as carrying no device identity — "
        "so the one signal an operator gets for a reverted panel is dead")
    assert (_delta(before, after, "input_events_typed")
            + _delta(before, after, "input_events_generic")) >= 1, (
        "the frame was counted ambiguous and not counted as an event at all; "
        "the two are different questions and a frame answers both")


@pytest.mark.hil_id("HIL-INP-13")
@pytest.mark.foreign
@pytest.mark.ha_bridge
def test_a_press_reaches_home_assistant(
        api, foreign, panel, unkeyed_instance, ha_guard, hil_config,
        test_artifacts):
    from hil import mqtt_tap

    ha_guard.enter(expose_input_devices=True)
    topic = ha_guard.topic("a0/in/%d/%d/state" % (panel, unkeyed_instance))
    seen = mqtt_tap.collect_live(
        hil_config, topic,
        during=lambda: foreign.button24(panel, unkeyed_instance,
                                        "short_press"))
    test_artifacts.attach_json("mqtt", {"topic": topic, "seen": seen})

    assert seen, (
        "no publish on %s for a press the wire carried. If the registry and "
        "the WebSocket feed both show the event, the bridge dropped it above "
        "them — which is what it did for every press until the instance type "
        "reached the payload builder" % topic)
    payloads = [p for _, p in seen]
    assert any('"event_type":"short_press"' in p.replace(" ", "")
               for p in payloads), (
        "the bridge published something other than the press that happened: "
        "%r. `release` here would mean the value came out of a field the "
        "enrichment did not fill — the failure `SNIF-048` pins on the host"
        % payloads)
