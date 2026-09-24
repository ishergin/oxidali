from hil.foreign import frame_hex, frames_on_wire

TOPIC = "/wb-dali/wb-dali_19_bus_1/bus_monitor"
DTR0_114 = {"bits": 16, "bytes": [0xA3, 0x72]}


def test_frame_hex_is_the_monitors_spelling():
    assert frame_hex(DTR0_114) == "a372"
    assert frame_hex({"bits": 24, "bytes": [0xFF, 0xFE, 0x3D]}) == "fffe3d"


def test_a_refused_batch_is_not_delivered_whatever_else_crossed_the_wire():
    lines = [
        TOPIC + " >>   a372 FF16 DTR0(114) (from lunatone)",
        TOPIC + " <<no response from gateway",
        TOPIC + " <<   0999 FF16 QueryDeviceType(<address (control gear) 4>) (fc: 1)",
        TOPIC + " << fffe3d FF24 QueryApplicationControlEnabled(<broadcast>) (fc: 2)",
    ]
    seen, errors = frames_on_wire(lines, [DTR0_114])
    assert seen == 0
    assert errors == ["no response from gateway"]


def test_a_frame_on_the_wire_counts_and_the_request_line_does_not():
    lines = [
        TOPIC + " >>   a372 FF16 DTR0(114) (from lunatone)",
        TOPIC + " <<   a372 FF16 DTR0(114) (fc: 29786)",
    ]
    assert frames_on_wire(lines, [DTR0_114]) == (1, [])


def test_a_truncated_batch_is_not_delivered():
    dtr1 = {"bits": 16, "bytes": [0xC3, 0x01]}
    activate = {"bits": 16, "bytes": [0x03, 0xE2]}
    lines = [
        TOPIC + " >>   a372 FF16 DTR0(114) (from lunatone)",
        TOPIC + " <<   a372 FF16 DTR0(114) (fc: 1)",
        TOPIC + " >>   c301 FF16 DTR1(1) (from lunatone)",
        TOPIC + " <<   c301 FF16 DTR1(1) (fc: 2)",
        TOPIC + " >>   03e2 FF16 Activate(1) (from lunatone)",
        TOPIC + " <<no response from gateway",
    ]
    seen, errors = frames_on_wire(lines, [DTR0_114, dtr1, activate])
    assert (seen, errors) == (2, ["no response from gateway"])


def test_someone_elses_identical_frame_is_not_ours():
    lines = [
        TOPIC + " >>   a372 FF16 DTR0(114) (from lunatone)",
        TOPIC + " <<no response from gateway",
        TOPIC + " <<   a372 FF16 DTR0(114) (fc: 7)",
    ]
    assert frames_on_wire(lines, [DTR0_114]) == (0, ["no response from gateway"])
