import json
import queue
import subprocess
import threading
import time

from hil import write_log
from hil.sniffer import ssh_argv

RAW_BROKER_PORT = 19883
RECV_BUFFER_BYTES = 1024
REPLY_TIMEOUT_S = 10.0
START_TIMEOUT_S = 15.0
STDERR_TIMEOUT_S = 2.0
POLL_S = 0.2
MARKER = "hil-raw-broker"
MARKER_PATTERN = "[h]il-raw-broker"

_SCRIPT = r"""
import json, socket, sys, threading

PORT, RCVBUF, MARKER = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
lock = threading.Lock()
state = {"connects": 0, "connected": False, "filters": [], "publishes": 0, "bytes_in": 0,
         "client_id": None, "topics_tail": []}
conn = [None]
send_lock = threading.Lock()
packet_id = [1]


def emit(doc):
    with lock:
        sys.stdout.write(json.dumps(doc) + "\n")
        sys.stdout.flush()


def varint(n):
    out = bytearray()
    while True:
        byte, n = n % 128, n // 128
        out.append(byte | (0x80 if n else 0))
        if not n:
            return bytes(out)


def send(sock, kind, body):
    with send_lock:
        sock.sendall(bytes([kind]) + varint(len(body)) + body)


def read_exact(sock, n):
    data = bytearray()
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise EOFError
        data += chunk
        state["bytes_in"] += len(chunk)
    return bytes(data)


def read_packet(sock):
    head = read_exact(sock, 1)[0]
    length, shift = 0, 0
    while True:
        byte = read_exact(sock, 1)[0]
        length += (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            break
    return head, read_exact(sock, length) if length else b""


def field(body, at):
    size = int.from_bytes(body[at:at + 2], "big")
    return body[at + 2:at + 2 + size], at + 2 + size


def on_connect(sock, body):
    _, at = field(body, 0)
    at += 4
    client_id, _ = field(body, at)
    state["connects"] += 1
    state["connected"] = True
    state["client_id"] = client_id.decode("utf-8", "replace")
    send(sock, 0x20, b"\x00\x00")
    emit({"ev": "connect", "n": state["connects"], "client_id": state["client_id"]})


def on_publish(sock, head, body):
    topic, at = field(body, 0)
    qos = (head >> 1) & 3
    if qos:
        send(sock, 0x40 if qos == 1 else 0x50, body[at:at + 2])
    state["publishes"] += 1
    state["topics_tail"] = (state["topics_tail"] + [topic.decode("utf-8", "replace")])[-20:]


def on_subscribe(sock, body):
    pid, at, granted, filters = body[:2], 2, bytearray(), []
    while at < len(body):
        topic, at = field(body, at)
        granted.append(min(body[at], 1))
        filters.append(topic.decode("utf-8", "replace"))
        at += 1
    state["filters"] = sorted(set(state["filters"]) | set(filters))
    send(sock, 0x90, pid + bytes(granted))
    emit({"ev": "subscribe", "filters": filters})


def on_unsubscribe(sock, body):
    at, gone = 2, set()
    while at < len(body):
        topic, at = field(body, at)
        gone.add(topic.decode("utf-8", "replace"))
    state["filters"] = sorted(set(state["filters"]) - gone)
    send(sock, 0xB0, body[:2])


def serve(sock):
    try:
        while True:
            head, body = read_packet(sock)
            kind = head & 0xF0
            if kind == 0x10:
                on_connect(sock, body)
            elif kind == 0x30:
                on_publish(sock, head, body)
            elif kind == 0x80:
                on_subscribe(sock, body)
            elif kind == 0x60:
                send(sock, 0x70, body[:2])
            elif kind == 0xA0:
                on_unsubscribe(sock, body)
            elif kind == 0xC0:
                send(sock, 0xD0, b"")
            elif kind == 0xE0:
                break
    except (EOFError, OSError):
        pass
    finally:
        try:
            sock.close()
        except OSError:
            pass
        if conn[0] is sock:
            conn[0] = None
            state["connected"] = False
            state["filters"] = []
            emit({"ev": "closed", "n": state["connects"]})


def accept_loop(listener):
    while True:
        sock, _ = listener.accept()
        old, conn[0] = conn[0], sock
        state["filters"] = []
        if old is not None:
            try:
                old.close()
            except OSError:
                pass
        threading.Thread(target=serve, args=(sock,), daemon=True).start()


def publish_to_client(cmd):
    sock = conn[0]
    if sock is None:
        return {"ev": "error", "error": "no client"}
    topic = bytes.fromhex(cmd["topic_hex"])
    payload = bytes.fromhex(cmd.get("payload_hex", ""))
    qos = int(cmd.get("qos", 0))
    body = len(topic).to_bytes(2, "big") + topic
    if qos:
        body += packet_id[0].to_bytes(2, "big")
        packet_id[0] = packet_id[0] % 65535 + 1
    send(sock, 0x30 | (qos << 1), body + payload)
    return {"ev": "published"}


def drop_client():
    sock = conn[0]
    if sock is not None:
        try:
            sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
    return {"ev": "dropped", "had_client": sock is not None}


def control(cmd):
    if cmd["cmd"] == "status":
        return dict(state, ev="status")
    if cmd["cmd"] == "publish":
        return publish_to_client(cmd)
    if cmd["cmd"] == "drop":
        return drop_client()
    return {"ev": "error", "error": "unknown command %r" % cmd["cmd"]}


listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, RCVBUF)
listener.bind(("0.0.0.0", PORT))
listener.listen(2)
threading.Thread(target=accept_loop, args=(listener,), daemon=True).start()
emit({"ev": "ready", "port": PORT, "marker": MARKER})
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    cmd = json.loads(line)
    if cmd["cmd"] == "quit":
        break
    try:
        reply = control(cmd)
    except OSError as exc:
        reply = {"ev": "error", "error": "socket: %s" % exc}
    reply["id"] = cmd.get("id")
    emit(reply)
"""


class RawBrokerError(RuntimeError):
    pass


def _quote(text):
    return "'" + text.replace("'", "'\\''") + "'"


class RawBroker:
    def __init__(self, cfg, port=RAW_BROKER_PORT):
        self.cfg, self.port = cfg, port
        self.host = cfg.mqtt_broker_host
        self.events = []
        self._replies = queue.Queue()
        self._next_id = 0
        self._proc = None

    def start(self):
        self._kill_leftover()
        remote = "exec python3 -u -c %s %d %d %s" % (
            _quote(_SCRIPT), self.port, RECV_BUFFER_BYTES, MARKER)
        self._proc = subprocess.Popen(ssh_argv(self.cfg, remote), stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                      text=True, bufsize=1)
        threading.Thread(target=self._read, daemon=True).start()
        if not self.wait_event(lambda e: e.get("ev") == "ready", START_TIMEOUT_S):
            self._proc.kill()
            why = self._stderr()
            self.stop()
            raise RawBrokerError("the raw broker on %s did not start: %s" % (self.host, why))
        return self

    def stop(self):
        if self._proc is None:
            return
        try:
            self._proc.stdin.write(json.dumps({"cmd": "quit"}) + "\n")
            self._proc.stdin.flush()
            self._proc.wait(timeout=REPLY_TIMEOUT_S)
        except (OSError, subprocess.TimeoutExpired, ValueError):
            self._proc.kill()
        self._kill_leftover()
        self._proc = None

    def _kill_leftover(self):
        pattern = _quote(MARKER_PATTERN)
        subprocess.run(ssh_argv(self.cfg, "pkill -f %s; while pgrep -f %s >/dev/null; "
                                          "do sleep %s; done" % (pattern, pattern, POLL_S)),
                       capture_output=True, timeout=REPLY_TIMEOUT_S)

    def _stderr(self):
        try:
            return (self._proc.communicate(timeout=STDERR_TIMEOUT_S)[1] or "")[:300]
        except (OSError, ValueError, subprocess.TimeoutExpired):
            return ""

    def _read(self):
        for line in self._proc.stdout:
            try:
                doc = json.loads(line)
            except ValueError:
                continue
            if doc.get("id") is not None:
                self._replies.put(doc)
            else:
                doc["at"] = time.monotonic()
                self.events.append(doc)

    def request(self, cmd, **fields):
        self._next_id += 1
        doc = dict(fields, cmd=cmd, id=self._next_id)
        self._proc.stdin.write(json.dumps(doc) + "\n")
        self._proc.stdin.flush()
        deadline = time.monotonic() + REPLY_TIMEOUT_S
        while time.monotonic() < deadline:
            try:
                reply = self._replies.get(timeout=POLL_S)
            except queue.Empty:
                continue
            if reply.get("id") == self._next_id:
                if reply.get("ev") == "error":
                    raise RawBrokerError(reply.get("error"))
                return reply
        raise RawBrokerError("no reply to %r from the raw broker" % cmd)

    def wait_event(self, pred, timeout_s):
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            if any(pred(e) for e in self.events):
                return True
            time.sleep(POLL_S)
        return any(pred(e) for e in self.events)

    def connects(self):
        return self.request("status")["connects"]

    def status(self):
        return self.request("status")

    def publish(self, topic, payload=b"", qos=0):
        write_log.note(None, write_log.topic_keys(topic.decode("utf-8", "replace")))
        return self.request("publish", topic_hex=topic.hex(), payload_hex=payload.hex(),
                            qos=qos)

    def drop(self):
        return self.request("drop")
