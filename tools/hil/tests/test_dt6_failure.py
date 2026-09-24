import pytest

FAILURE_STATUS_OPCODE = 0xF1
ENABLE_DEVICE_TYPE_6 = "C106"


def _dt6_short(api):
    for short in api.addrs():
        if short < 4:
            continue
        if api.state(short).get("device_type_effective") == "dt6_led":
            return short
    return None


def _failure_section(api, short):
    return (api.attributes(short, ["dt6_led"]).get("attributes") or {}).get("dt6_led") or {}


def _cell(section, field):
    return (section.get(field) or {}).get("value")


def _query_frames(win, short):
    wanted = "%02X%02X" % ((short << 1) | 0x01, FAILURE_STATUS_OPCODE)
    return [f for f in win.frames() if f.get("bytes") == wanted]


@pytest.fixture()
def faulted_gear(api, gear_sim):
    short = _dt6_short(api)
    if short is None:
        pytest.skip("no DT6 gear on the segment — the C6 fleet is off the bus")
    injected = []

    def inject(word, expected_byte):
        reply = gear_sim.command("fail %d %s" % (short, word))
        wanted = "failure_status=0x%02X" % expected_byte
        assert any(wanted in line for line in reply), (
            "the emulator did not confirm %s on A%02d: %r" % (wanted, short, reply))
        injected.append(word)
        return short

    yield inject
    if injected:
        gear_sim.command("fail %d none" % short)


@pytest.mark.hil_id("HIL-DT6-01")
def test_a_lamp_failure_escalates_a_runtime_read_to_the_207_byte(
        api, faulted_gear, op_check):
    short = faulted_gear("short", 0x01)

    op_check(api.wait_op(api.attr_read(short, groups="runtime_status")))

    section = _failure_section(api, short)
    assert _cell(section, "failure_status") == 0x01, (
        "gear %d reported no Part 207 failure byte after a runtime-only read; "
        "section: %r" % (short, section))
    assert _cell(section, "short_circuit") == 0xFF, (
        "bit 0 of the byte is the answer command 242 would give: %r" % section)
    assert _cell(section, "open_circuit") == 0, (
        "a clear bit is the gear saying no, not an unread field: %r" % section)
    assert api.state(short)["state"]["status"]["lamp_failure"] is True, (
        "207 §11.3.4.2 makes bits 0-4 raise status bit 1 — the trigger this "
        "whole mechanism reads")


@pytest.mark.hil_id("HIL-DT6-02")
def test_a_thermal_shut_down_is_found_through_the_masked_level(
        api, faulted_gear, op_check):
    short = faulted_gear("thermal", 0x20)

    op_check(api.wait_op(api.attr_read(short, groups="runtime_status")))

    core = api.state(short)["state"]
    assert core["status"]["lamp_failure"] is False, (
        "a thermal shut down is not a lamp failure (error 7185 if the gear "
        "says otherwise): %r" % core["status"])
    assert core["level"] != 255, (
        "MASK reached the registry as a level again — a dark luminaire would "
        "render at full brightness (ISSUE-66): %r" % core)
    section = _failure_section(api, short)
    assert _cell(section, "failure_status") == 0x20, (
        "the thermal bit is what the escalation went to fetch: %r" % section)
    assert _cell(section, "thermal_shutdown") == 0xFF, section


@pytest.mark.hil_id("HIL-DT6-03")
@pytest.mark.sniffer
def test_a_healthy_dt6_gear_pays_no_207_frames(api, gear_sim, op_check, sniffer):
    short = _dt6_short(api)
    if short is None:
        pytest.skip("no DT6 gear on the segment — the C6 fleet is off the bus")
    gear_sim.command("fail %d none" % short)

    with sniffer.window() as win:
        op_check(api.wait_op(api.attr_read(short, groups="runtime_status")))
        hits = _query_frames(win, short)

    assert not hits, (
        "a healthy gear drew QUERY FAILURE STATUS: %r"
        % [f["decoded"] for f in hits][:4])


@pytest.mark.hil_id("HIL-DT6-04")
@pytest.mark.sniffer
def test_the_escalation_sends_its_prelude_and_one_query(
        api, faulted_gear, op_check, sniffer):
    short = faulted_gear("open", 0x02)

    with sniffer.window() as win:
        op_check(api.wait_op(api.attr_read(short, groups="runtime_status")))
        hits = _query_frames(win, short)
        preludes = [f for f in win.frames() if f.get("bytes") == ENABLE_DEVICE_TYPE_6]

    assert hits, "no QUERY FAILURE STATUS reached the wire"
    assert preludes, (
        "the query went out with no ENABLE DEVICE TYPE 6 in the window — "
        "§11.6.1 makes a bare 0xF1 a standard opcode the gear must ignore")
    assert len(hits) <= 2, (
        "the escalation is one query (a retry makes two); %d frames means the "
        "whole dt6_led section ran: %r" % (len(hits), [f["decoded"] for f in hits]))
