import base64
import hashlib
import json
import os
import socket
import struct
import time
from urllib.parse import urlsplit

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

OP_CONTINUE = 0x0
OP_TEXT = 0x1
OP_BINARY = 0x2
OP_CLOSE = 0x8
OP_PING = 0x9
OP_PONG = 0xA

MAX_FRAME_BYTES = 256 * 1024


class WsError(RuntimeError):
    pass


class WsClient:
    def __init__(self, sock, timeout):
        self._sock = sock
        self._buf = b""
        self._timeout = timeout
        self._sock.settimeout(timeout)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def close(self):
        try:
            self.send(OP_CLOSE, b"")
        except (OSError, WsError):
            pass
        try:
            self._sock.close()
        except OSError:
            pass

    def abort(self):
        try:
            self._sock.close()
        except OSError:
            pass

    def send(self, opcode, payload=b"", fin=True):
        if isinstance(payload, str):
            payload = payload.encode()
        first = (0x80 if fin else 0x00) | (opcode & 0x0F)
        n = len(payload)
        if n < 126:
            head = struct.pack("!BB", first, 0x80 | n)
        elif n < (1 << 16):
            head = struct.pack("!BBH", first, 0x80 | 126, n)
        else:
            head = struct.pack("!BBQ", first, 0x80 | 127, n)
        key = os.urandom(4)
        masked = bytes(b ^ key[i % 4] for i, b in enumerate(payload))
        self._sock.sendall(head + key + masked)

    def send_json(self, obj):
        self.send(OP_TEXT, json.dumps(obj))

    def subscribe(self, channels, log_level=None):
        frame = {"op": "subscribe", "channels": list(channels)}
        if log_level is not None:
            frame["logs"] = {"min_level": log_level}
        self.send_json(frame)

    def wait_for_type(self, event_type, timeout=5.0, deadline_s=10.0):
        end = time.monotonic() + deadline_s
        while time.monotonic() < end:
            try:
                frame = self.recv_json(
                    timeout=min(timeout, max(0.1, end - time.monotonic())))
            except socket.timeout:
                return None
            if frame and frame.get("type") == event_type:
                return frame
        return None

    def recv(self, timeout=None):
        if timeout is not None:
            self._sock.settimeout(timeout)
        try:
            first, second = self._read_exactly(2)
            opcode = first & 0x0F
            masked = bool(second & 0x80)
            n = second & 0x7F
            if n == 126:
                (n,) = struct.unpack("!H", self._read_exactly(2))
            elif n == 127:
                (n,) = struct.unpack("!Q", self._read_exactly(8))
            if n > MAX_FRAME_BYTES:
                raise WsError("server frame of %d bytes exceeds the cap" % n)
            key = self._read_exactly(4) if masked else b""
            payload = self._read_exactly(n) if n else b""
            if masked:
                payload = bytes(b ^ key[i % 4] for i, b in enumerate(payload))
            return opcode, payload
        finally:
            self._sock.settimeout(self._timeout)

    def recv_json(self, timeout=None):
        opcode, payload = self.recv(timeout)
        if opcode != OP_TEXT:
            return None
        return json.loads(payload.decode())

    def wait_for_op(self, op, timeout=5.0, deadline_s=10.0):
        end = time.monotonic() + deadline_s
        while time.monotonic() < end:
            try:
                frame = self.recv_json(
                    timeout=min(timeout, max(0.1, end - time.monotonic())))
            except socket.timeout:
                return None
            if frame and frame.get("op") == op:
                return frame
        return None

    def _read_exactly(self, n):
        while len(self._buf) < n:
            chunk = self._sock.recv(max(4096, n - len(self._buf)))
            if not chunk:
                raise WsError("connection closed after %d of %d bytes"
                              % (len(self._buf), n))
            self._buf += chunk
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _read_head(self):
        while b"\r\n\r\n" not in self._buf:
            chunk = self._sock.recv(4096)
            if not chunk:
                raise WsError("connection closed during the handshake")
            self._buf += chunk
        head, _, rest = self._buf.partition(b"\r\n\r\n")
        self._buf = rest
        return head.decode("latin-1")


def header_value(head, name):
    wanted = name.lower()
    for line in head.split("\r\n")[1:]:
        key, sep, value = line.partition(":")
        if sep and key.strip().lower() == wanted:
            return value.strip()
    return None


def accept_for_key(key):
    return base64.b64encode(
        hashlib.sha1((key + WS_GUID).encode()).digest()).decode()


def connect(base, path="/api/v1/ws", timeout=5.0):
    parts = urlsplit(base if "//" in base else "http://" + base)
    host = parts.hostname
    port = parts.port or 80
    key = base64.b64encode(os.urandom(16)).decode()
    request = (
        "GET %s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\n"
        "Connection: Upgrade\r\nSec-WebSocket-Key: %s\r\n"
        "Sec-WebSocket-Version: 13\r\n\r\n" % (path, host, key)
    )
    sock = socket.create_connection((host, port), timeout)
    client = WsClient(sock, timeout)
    sock.sendall(request.encode())
    head = client._read_head()
    status = head.split("\r\n", 1)[0]
    if "101" not in status:
        client.abort()
        raise WsError("upgrade refused: %s" % status)
    expected = accept_for_key(key)
    accept = header_value(head, "Sec-WebSocket-Accept")
    if accept != expected:
        client.abort()
        raise WsError(
            "Sec-WebSocket-Accept was %r, expected %r for the key we sent"
            % (accept, expected))
    return client
