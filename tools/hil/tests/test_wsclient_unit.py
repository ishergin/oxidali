import base64
import socket
import threading

import pytest

import test_ws
from hil import wsclient
from hil.wait import wait_until

DRAIN_WAIT_S = 5.0


def test_header_value_matches_the_name_case_insensitively():
    head = "HTTP/1.1 101 Switching Protocols\r\nSEC-WebSocket-Accept:  abc  "
    assert wsclient.header_value(head, "Sec-WebSocket-Accept") == "abc"


def test_header_value_does_not_read_the_status_line_or_missing_headers():
    head = "HTTP/1.1 101 Sec-WebSocket-Accept: not-a-header\r\nUpgrade: websocket"
    assert wsclient.header_value(head, "Sec-WebSocket-Accept") is None


def test_accept_matches_the_rfc_6455_example():
    assert wsclient.accept_for_key("dGhlIHNhbXBsZSBub25jZQ==") == \
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="


def _serve_one(accept_line, ready):
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    ready.append(listener.getsockname()[1])

    def run():
        conn, _ = listener.accept()
        with conn:
            request = b""
            while b"\r\n\r\n" not in request:
                chunk = conn.recv(4096)
                if not chunk:
                    return
                request += chunk
            key = ""
            for line in request.decode("latin-1").split("\r\n")[1:]:
                name, sep, value = line.partition(":")
                if sep and name.strip().lower() == "sec-websocket-key":
                    key = value.strip()
            conn.sendall(
                ("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
                 "Connection: Upgrade\r\n" + accept_line(key) + "\r\n\r\n").encode())
        listener.close()

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    return thread


def _connect_to(accept_line):
    ready = []
    thread = _serve_one(accept_line, ready)
    try:
        return wsclient.connect("http://127.0.0.1:%d" % ready[0], timeout=5.0)
    finally:
        thread.join(timeout=5.0)


def test_a_correct_accept_is_upgraded():
    client = _connect_to(
        lambda key: "Sec-WebSocket-Accept: " + wsclient.accept_for_key(key))
    client.abort()


def test_a_lowercased_accept_is_refused():
    with pytest.raises(wsclient.WsError, match="Sec-WebSocket-Accept"):
        _connect_to(
            lambda key: "Sec-WebSocket-Accept: "
            + wsclient.accept_for_key(key).lower())


def test_the_right_digest_in_the_wrong_header_is_refused():
    with pytest.raises(wsclient.WsError, match="Sec-WebSocket-Accept"):
        _connect_to(lambda key: "X-Echo: " + wsclient.accept_for_key(key))


def test_a_padded_accept_is_refused():
    with pytest.raises(wsclient.WsError, match="Sec-WebSocket-Accept"):
        _connect_to(
            lambda key: "Sec-WebSocket-Accept: xx" + wsclient.accept_for_key(key))


def test_a_digest_for_another_key_is_refused():
    other = base64.b64encode(b"0123456789abcdef").decode()
    with pytest.raises(wsclient.WsError, match="Sec-WebSocket-Accept"):
        _connect_to(lambda _key: "Sec-WebSocket-Accept: "
                    + wsclient.accept_for_key(other))


@pytest.mark.parametrize("opcode,payload,kind", [
    (wsclient.OP_TEXT, b'{"type":"StatsSnapshot","channel":"stats"}', "StatsSnapshot"),
    (wsclient.OP_TEXT, b'{"op":"subscribed","channels":["stats"]}', "subscribed"),
    (wsclient.OP_TEXT, b"[1]", wsclient.UNDECODABLE),
    (wsclient.OP_TEXT, b"\xff{", wsclient.UNDECODABLE),
    (wsclient.OP_CLOSE, b"", "opcode 0x8"),
])
def test_a_frame_is_counted_by_its_type(opcode, payload, kind):
    assert wsclient.frame_kind(opcode, payload) == kind


class _FakeWs:
    def __init__(self, frames, drops=False):
        self.frames, self.drops = list(frames), drops
        self.channels, self.closed = None, False

    def subscribe(self, channels):
        self.channels = list(channels)

    def recv(self, timeout=None):
        if self.frames:
            return self.frames.pop(0)
        if self.drops:
            raise wsclient.WsError("connection closed after 0 of 2 bytes")
        threading.Event().wait(0.01)
        raise socket.timeout()

    def close(self):
        self.closed = True


def _text(kind):
    return wsclient.OP_TEXT, ('{"type":"%s"}' % kind).encode()


def test_subscribers_count_what_each_client_received_and_note_a_drop():
    clients = [_FakeWs([_text("SnifferBatch"), _text("DiagnosticsSnapshot")]),
               _FakeWs([_text("SnifferBatch")], drops=True)]
    opened = iter(clients)
    load = wsclient.Subscribers("http://127.0.0.1:9", 2, opener=lambda base: next(opened))
    assert wait_until(lambda: not any(client.frames for client in clients) and load.errors,
                      DRAIN_WAIT_S, interval_s=0.01)
    load.close()
    assert [dict(kinds) for kinds in load.kinds] == [
        {"SnifferBatch": 1, "DiagnosticsSnapshot": 1}, {"SnifferBatch": 1}]
    assert load.errors == ["subscriber 1 dropped: connection closed after 0 of 2 bytes"]
    assert all(client.closed and client.channels == list(wsclient.LOAD_CHANNELS)
               for client in clients)


def test_a_client_that_cannot_open_closes_the_ones_that_did():
    first = _FakeWs([])

    def opener(base):
        if first.channels is None:
            return first
        raise wsclient.WsError("upgrade refused: HTTP/1.1 503")
    with pytest.raises(wsclient.WsError, match="upgrade refused"):
        wsclient.Subscribers("http://127.0.0.1:9", 2, opener=opener)
    assert first.closed


def test_a_subscriber_without_a_sniffer_batch_or_enough_snapshots_proves_nothing():
    busy = {"SnifferBatch": 3, "DiagnosticsSnapshot": test_ws.MIN_DIAGNOSTICS}
    kinds = [busy, dict(busy, SnifferBatch=0), {"SnifferBatch": 1, "DiagnosticsSnapshot": 1}]
    assert test_ws._idle_subscribers(kinds) == [1, 2]
