import hil_session
import test_ws

UART = "rfc2217://127.0.0.1:4444"


class _Item:
    def __init__(self, markers):
        self.markers = set(markers)
        self.added = []

    def get_closest_marker(self, name):
        return name if name in self.markers else None

    def add_marker(self, marker):
        self.added.append(marker)


def _skip_reasons(item):
    return [m.kwargs["reason"] for m in item.added if m.name == "skip"]


def test_a_serial_test_is_skipped_while_the_console_is_unreachable():
    uart, plain = _Item({"smoke", "serial"}), _Item({"smoke"})
    hil_session._mark_skips([uart, plain], hil_session.NO_SERIAL, UART)
    assert _skip_reasons(uart) == ["serial port %s unreachable — %s" % (UART, hil_session.NO_SERIAL)]
    assert _skip_reasons(plain) == []


def test_a_serial_test_runs_while_the_console_answers():
    uart = _Item({"smoke", "serial"})
    hil_session._mark_skips([uart], None, UART)
    assert _skip_reasons(uart) == []


def test_the_log_channel_test_asks_for_the_serial_console():
    marks = {mark.name for mark in test_ws.test_the_log_channel_replays_and_keeps_uart_alive.pytestmark}
    assert {"smoke", "serial"} <= marks, (
        "HIL-WS-05 asserts on the UART; without the serial marker it fails, instead of "
        "skipping, on a run with no serial console"
    )
