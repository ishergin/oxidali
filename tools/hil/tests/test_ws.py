import socket
import struct
import time

import pytest

from hil import wsclient
from hil.wait import wait_until
from hil.wsclient import OP_BINARY, OP_PONG, WsError

RESERVED_OPCODE = 0x3

QUIET_CHANNEL = "diagnostics"

ACK_BUDGET_S = 8.0

CLOSE_BUDGET_S = 5.0


def _uptime(api):
    return int(api.health()["uptime_seconds"])


def _clients(api):
    return int(api.diagnostics()["websocket"]["clients"])


def _wait_clients(api, target, timeout_s=15.0):
    return bool(wait_until(lambda: _clients(api) <= target, timeout_s,
                           interval_s=0.5, desc="ws client slot release"))


@pytest.fixture()
def ws_baseline(api):
    before = {"uptime": _uptime(api), "clients": _clients(api)}

    def survived():
        after = _uptime(api)
        assert after >= before["uptime"], (
            "the controller REBOOTED: uptime went %d -> %d"
            % (before["uptime"], after))
        return after

    before["survived"] = survived
    yield before
    _wait_clients(api, before["clients"])


@pytest.mark.hil_id("HIL-WS-01")
@pytest.mark.smoke
def test_subscribe_is_acked(api, hil_config, ws_baseline, test_artifacts):
    with wsclient.connect(hil_config.base) as ws:
        hello = ws.wait_for_op("hello", deadline_s=ACK_BUDGET_S)
        assert hello is not None, "no hello frame after the upgrade"
        assert QUIET_CHANNEL in hello.get("channels", []), hello

        ws.subscribe([QUIET_CHANNEL])
        ack = ws.wait_for_op("subscribed", deadline_s=ACK_BUDGET_S)
        test_artifacts.attach_json("hello", hello)
        test_artifacts.attach_json("ack", ack)
        assert ack is not None, \
            "subscribe was never acked — check for an `error` frame above"
        assert QUIET_CHANNEL in ack.get("channels", []), ack

    ws_baseline["survived"]()


@pytest.mark.hil_id("HIL-WS-02")
def test_reserved_opcode_drops_the_client_not_the_controller(
        api, hil_config, ws_baseline, test_artifacts):
    with wsclient.connect(hil_config.base) as ws:
        assert ws.wait_for_op("hello", deadline_s=ACK_BUDGET_S) is not None

        ws.send(RESERVED_OPCODE, b"\x00")

        dropped = False
        try:
            deadline = time.monotonic() + CLOSE_BUDGET_S
            while time.monotonic() < deadline:
                opcode, _ = ws.recv(timeout=CLOSE_BUDGET_S)
                if opcode == wsclient.OP_CLOSE:
                    dropped = True
                    break
        except (WsError, socket.timeout, OSError):
            dropped = True

        test_artifacts.attach_json("outcome", {"dropped": dropped})

    uptime = ws_baseline["survived"]()
    test_artifacts.attach_json("uptime", {"before": ws_baseline["uptime"],
                                          "after": uptime})
    assert dropped, \
        "the reserved opcode was neither refused nor closed on: %s" % uptime
    assert _wait_clients(api, ws_baseline["clients"]), \
        "the refused client kept its slot: %s" % api.diagnostics()["websocket"]


@pytest.mark.hil_id("HIL-WS-03")
def test_empty_binary_frame_does_not_desync(api, hil_config, ws_baseline,
                                            test_artifacts):
    with wsclient.connect(hil_config.base) as ws:
        assert ws.wait_for_op("hello", deadline_s=ACK_BUDGET_S) is not None

        ws.send(OP_BINARY, b"")
        ws.subscribe([QUIET_CHANNEL])

        ack = ws.wait_for_op("subscribed", deadline_s=ACK_BUDGET_S)
        test_artifacts.attach_json("ack_after_empty_binary", ack)
        assert ack is not None, \
            "the empty binary frame swallowed the subscribe that followed it"

    ws_baseline["survived"]()


@pytest.mark.hil_id("HIL-WS-04")
def test_unsolicited_pong_does_not_desync(api, hil_config, ws_baseline,
                                          test_artifacts):
    with wsclient.connect(hil_config.base) as ws:
        assert ws.wait_for_op("hello", deadline_s=ACK_BUDGET_S) is not None

        ws.send(OP_PONG, b"")
        ws.subscribe([QUIET_CHANNEL])

        ack = ws.wait_for_op("subscribed", deadline_s=ACK_BUDGET_S)
        test_artifacts.attach_json("ack_after_pong", ack)
        assert ack is not None, \
            "the pong desynchronised the stream: the subscribe after it was " \
            "never acked"

    ws_baseline["survived"]()


@pytest.mark.hil_id("HIL-WS-05")
def test_closing_a_client_frees_its_slot(api, hil_config, ws_baseline,
                                         test_artifacts):
    before = ws_baseline["clients"]
    ws = wsclient.connect(hil_config.base)
    assert ws.wait_for_op("hello", deadline_s=ACK_BUDGET_S) is not None

    wait_until(lambda: _clients(api) > before, ACK_BUDGET_S, interval_s=0.5,
               desc="ws client to be counted")
    assert _clients(api) == before + 1, \
        "the connection was not counted: %s" % api.diagnostics()["websocket"]

    ws.close()
    freed = _wait_clients(api, before)
    test_artifacts.attach_json("websocket", api.diagnostics()["websocket"])
    assert freed, "the client slot was never released after a clean close"

    ws_baseline["survived"]()


LOG_CHANNEL = "logs"
LOG_LEVEL = "info"

LOG_BUDGET_S = 8.0


def _log_lines(frame):
    return (frame or {}).get("payload", {}).get("lines", [])


@pytest.mark.hil_id("HIL-WS-05")
@pytest.mark.smoke
def test_the_log_channel_replays_and_keeps_uart_alive(
        api, hil_config, ws_baseline, serial_log, test_artifacts):
    with serial_log.window() as serial:
        with wsclient.connect(hil_config.base) as ws:
            hello = ws.wait_for_op("hello", deadline_s=ACK_BUDGET_S)
            assert hello is not None, "no hello frame after the upgrade"
            assert LOG_CHANNEL in hello.get("channels", []), hello

            ws.subscribe([LOG_CHANNEL], log_level=LOG_LEVEL)
            ack = ws.wait_for_op("subscribed", deadline_s=ACK_BUDGET_S)
            assert ack is not None and LOG_CHANNEL in ack.get("channels", []), ack

            batch = ws.wait_for_type("LogBatch", deadline_s=LOG_BUDGET_S)
            test_artifacts.attach_json("log_batch", batch)
            assert batch is not None, (
                "no LogBatch inside %.0f s — either the ring was empty (it is "
                "armed at warn, so that is itself a finding) or the flush "
                "never ran" % LOG_BUDGET_S)

            lines = _log_lines(batch)
            assert lines, "a LogBatch with no lines and no gap is not a batch"
            for line in lines:
                assert line.get("level"), line
                assert isinstance(line.get("seq"), int), line
            seqs = [line["seq"] for line in lines]
            assert seqs == sorted(seqs), "lines must arrive in sequence: %r" % seqs

        assert serial.expect(r"dali2rust", timeout_s=LOG_BUDGET_S), (
            "nothing reached UART while the log channel was subscribed — the "
            "vprintf hook consumed the arguments and did not print them")

    ws_baseline["survived"]()


@pytest.mark.hil_id("HIL-WS-06")
def test_the_log_channel_carries_an_esp_idf_component_line(
        api, hil_config, ws_baseline, test_artifacts):
    with wsclient.connect(hil_config.base) as ws:
        assert ws.wait_for_op("hello", deadline_s=ACK_BUDGET_S) is not None
        ws.subscribe([LOG_CHANNEL], log_level=LOG_LEVEL)
        assert ws.wait_for_op("subscribed", deadline_s=ACK_BUDGET_S) is not None

        rude = wsclient.connect(hil_config.base)
        rude.wait_for_op("hello", deadline_s=ACK_BUDGET_S)
        rude._sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER,
                              struct.pack("ii", 1, 0))
        rude._sock.close()

        foreign = []
        deadline = time.monotonic() + LOG_BUDGET_S * 2
        while time.monotonic() < deadline and not foreign:
            batch = ws.wait_for_type("LogBatch", deadline_s=LOG_BUDGET_S)
            for line in _log_lines(batch):
                target = line.get("target", "")
                if target and not target.startswith("dali2rust"):
                    foreign.append(line)

        test_artifacts.attach_json("foreign_lines", foreign)
        assert foreign, (
            "no ESP-IDF component line reached the channel — the hook is "
            "installed above the C log path, or the component's own level "
            "filter is quieter than the ring's")

    ws_baseline["survived"]()
