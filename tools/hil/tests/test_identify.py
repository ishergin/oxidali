import time

import pytest

QUERY_STATUS = 0x90
QUERY_ACTUAL_LEVEL = 0xA0
POWER_CYCLE_SEEN = 0x80
FORBIDDEN_ON_THE_WIRE = ("DAPC", "RECALL MAX", "RECALL MIN", "OFF",
                         "GO TO LAST ACTIVE LEVEL")
SETTLE_LEVEL = 120


def _query(api, short, opcode):
    frame = (((short << 1) | 1) << 8) | opcode
    resp = api.raw(frame, expects_backward=True)
    if not resp.get("success"):
        return None
    return resp.get("backward_frame")


@pytest.fixture()
def identify_target(api):
    shorts = api.optical_addrs()
    if not shorts:
        pytest.skip("no bench luminaires in the registry")
    short = shorts[0]
    yield short
    api.off(short)


@pytest.mark.sniffer
def test_identify_sends_the_send_twice_pair_and_no_level_command(
    api, identify_target, sniffer, paced, op_check
):
    short = identify_target
    paced()
    api.dapc(short, SETTLE_LEVEL)
    time.sleep(1.5)

    with sniffer.window() as win:
        time.sleep(1.0)
        op_check(api.wait_op(api.identify(short), timeout_s=30))
        time.sleep(2.0)
        decoded = [f.get("decoded") or "" for f in win.frames()]

    mine = [t for t in decoded if ("short %d" % short) in t]
    identify_frames = [t for t in mine if "IDENTIFY DEVICE" in t]
    level_frames = [t for t in mine if any(f in t for f in FORBIDDEN_ON_THE_WIRE)]

    assert len(identify_frames) == 2, (
        "expected the 0x25 send-twice pair, saw %d IDENTIFY frames among %r"
        % (len(identify_frames), mine)
    )
    assert not level_frames, (
        "§9.14.3.1: identification affects no variables, but the sequence sent "
        "%r" % (level_frames,)
    )


@pytest.mark.sniffer
def test_identify_leaves_power_cycle_seen_alone(api, identify_target, paced, op_check):
    short = identify_target
    paced()
    api.dapc(short, SETTLE_LEVEL)
    time.sleep(1.5)

    before = _query(api, short, QUERY_STATUS)
    if before is None:
        pytest.fail("QUERY STATUS unanswered before identify — cannot judge")

    op_check(api.wait_op(api.identify(short), timeout_s=30))
    time.sleep(1.0)

    after = _query(api, short, QUERY_STATUS)
    if after is None:
        pytest.fail("QUERY STATUS unanswered after identify — cannot judge")

    assert (before & POWER_CYCLE_SEEN) == (after & POWER_CYCLE_SEEN), (
        "powerCycleSeen changed across identify (status %#04x -> %#04x); "
        "§9.14.3.1 says the procedure affects no variables" % (before, after)
    )


def test_identify_refuses_a_duration(api, identify_target):
    status, payload = api.raw_request(
        "POST",
        "adapters/%d/commissioning/identify" % api.adapter,
        {"short_address": identify_target, "duration_ms": 5000},
    )
    assert status == 422, (
        "a field the gear cannot obey must be refused, not discarded: got %s %r"
        % (status, payload)
    )
    assert (payload or {}).get("error") == "unsupported_field", (
        "the refusal must name the field as the reason: %r" % (payload,)
    )


def test_identify_reports_the_mechanism_it_used(api, identify_target, op_check):
    view = op_check(api.wait_op(api.identify(identify_target), timeout_s=30))
    mechanism = (view.get("result") or {}).get("identify_mechanism")
    assert mechanism in (None, "identify_device"), (
        "a second identification mechanism appeared: %r. One mechanism per "
        "effect — a fallback here cannot be verified, because 0x25 has no "
        "acknowledgement and no status query." % (mechanism,)
    )
