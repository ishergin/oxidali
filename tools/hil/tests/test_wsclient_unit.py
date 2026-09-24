import base64
import socket
import threading

import pytest

from hil import wsclient


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
