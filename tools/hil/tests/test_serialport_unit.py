import pytest

from hil import serialport


class _Port:
    def __init__(self, device, vid):
        self.device = device
        self.vid = vid


def _with_ports(monkeypatch, ports):
    monkeypatch.setattr(serialport, "candidates", lambda: sorted(p.device for p in ports))


def test_a_ch34x_bridge_is_a_board_port():
    assert 0x1A86 in serialport.SERIAL_VIDS
    assert 0x303A in serialport.SERIAL_VIDS


def test_resolve_finds_the_attached_board(monkeypatch):
    _with_ports(monkeypatch, [_Port("/dev/cu.usbmodem5B901574541", 0x1A86)])
    assert serialport.resolve() == "/dev/cu.usbmodem5B901574541"


def test_an_explicit_pin_beats_discovery(monkeypatch):
    _with_ports(monkeypatch, [_Port("/dev/cu.usbmodemAAA", 0x1A86)])
    assert serialport.resolve("/dev/cu.pinned") == "/dev/cu.pinned"


def test_two_boards_resolve_deterministically(monkeypatch):
    _with_ports(monkeypatch, [_Port("/dev/cu.usbmodemB", 0x1A86),
                              _Port("/dev/cu.usbmodemA", 0x303A)])
    assert serialport.resolve() == "/dev/cu.usbmodemA"
    assert serialport.resolve() == "/dev/cu.usbmodemA"


def test_nothing_attached_still_answers(monkeypatch):
    _with_ports(monkeypatch, [])
    assert serialport.resolve() == serialport.FALLBACK_PORT


def test_describe_names_a_recorded_pin(monkeypatch):
    monkeypatch.setattr(serialport, "candidates",
                        lambda: ["/dev/cu.usbmodemA", "/dev/cu.usbmodemB"])

    class _Cfg:
        serial_port = "/dev/cu.usbmodemA"
        serial_port_pinned = False

    line = serialport.describe(_Cfg(), recorded="/dev/cu.usbmodemB")
    assert line.startswith("/dev/cu.usbmodemB ")
    assert "serial_port.pin" in line


@pytest.mark.parametrize("pinned,ports,expected", [
    (False, ["/dev/cu.usbmodemA"], "auto-discovered"),
    (False, [], "no board serial port attached"),
    (False, ["/dev/cu.usbmodemA", "/dev/cu.usbmodemB"], "pin HIL_SERIAL_PORT"),
    (True, ["/dev/cu.usbmodemA"], "pinned via HIL_SERIAL_PORT"),
])
def test_describe_says_how_the_port_was_decided(monkeypatch, pinned, ports, expected):
    monkeypatch.setattr(serialport, "candidates", lambda: ports)

    class _Cfg:
        serial_port = ports[0] if ports else serialport.FALLBACK_PORT
        serial_port_pinned = pinned

    assert expected in serialport.describe(_Cfg())
