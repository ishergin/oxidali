import collections
import json
import os
import queue
import shlex
import subprocess
import sys
import threading
from urllib.parse import urlsplit

from hil import config
from hil.sniffer import ssh_argv

KEEPALIVE_IDLE_S = 30
KEEPALIVE_INTERVAL_S = 10
KEEPALIVE_PROBES = 3
KEEPALIVE_ABORT_S = KEEPALIVE_IDLE_S + KEEPALIVE_INTERVAL_S * KEEPALIVE_PROBES
EARLY_SLACK_S = 5.0
LATE_SLACK_S = 30.0
OVERRUN_S = 30.0

HOLD_S = 240
FAILSAFE_S = 360
CONNECT_TIMEOUT_S = 10.0
START_TIMEOUT_S = 30.0
REPLY_TIMEOUT_S = 20.0
XTABLES_WAIT_S = 5
NONCE_BYTES = 3
DEFAULT_HTTP_PORT = 80
SSH_FAILED = 255

WS_PATH = "/api/v1/ws"
TAG_PREFIX = "hil-ws-vanish-"
IPTABLES_CANDIDATES = ("iptables-legacy", "iptables")
CHAINS = ("OUTPUT", "INPUT")
INTERPRETER = "python3"

CONNECTED = "connected"
REFUSED = "refused"
VANISHED = "vanished"
GONE = "gone"
TROUBLE = ("unavailable", "error", GONE)

NEVER_COUNTED = "never counted"
NOISE = "noise"
NEVER_FREED = "never freed"
TOO_EARLY = "too early"
TOO_LATE = "too late"
IN_WINDOW = "in window"

USAGE = """usage: python3 -m hil.vanish sweep

Deletes every iptables rule on the WB whose comment starts with %s. HIL-WS-11
inserts two such rules and removes them itself; this is the manual recovery after a
killed session. Exits 0 when no tagged rule is left, 1 otherwise.""" % TAG_PREFIX

Tuple4 = collections.namedtuple("Tuple4", "local_ip local_port peer_ip peer_port")


class VanishUnavailable(RuntimeError):
    pass


_SCRIPT = r"""
import base64, hashlib, json, os, select, shutil, signal, socket, struct, subprocess, sys, time

HOST, PORT, PATH = sys.argv[1], int(sys.argv[2]), sys.argv[3]
HOLD_S, CONNECT_S, WAIT_S = float(sys.argv[4]), float(sys.argv[5]), sys.argv[6]
CANDIDATES = sys.argv[7].split(",")
GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
OP_TEXT, OP_CLOSE = 0x1, 0x8
SIGNALS = (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)
DELETE_ATTEMPTS = 2
state = {"sock": None, "deletes": [], "failsafe": None, "connected_at": None}


def emit(doc):
    try:
        sys.stdout.write(json.dumps(doc) + "\n")
        sys.stdout.flush()
    except (OSError, ValueError):
        pass


def interrupted(signum, frame):
    raise SystemExit(128 + signum)


def run(argv):
    try:
        done = subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, universal_newlines=True)
    except OSError as exc:
        return 127, str(exc)
    return done.returncode, (done.stderr or "").strip()[-300:]


def pick_iptables():
    why = "none of %s is installed" % ", ".join(CANDIDATES)
    for name in CANDIDATES:
        found = shutil.which(name)
        if not found:
            continue
        code, err = run([found, "-w", WAIT_S, "-S", "OUTPUT"])
        if code == 0:
            return found, None
        why = "%s -S OUTPUT exited %d: %s" % (found, code, err)
    return None, why


def read_more(sock, buf):
    chunk = sock.recv(4096)
    if not chunk:
        raise EOFError("the controller closed the connection")
    return buf + chunk


def upgrade(sock):
    key = base64.b64encode(os.urandom(16)).decode()
    sock.sendall(("GET %s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\n"
                  "Connection: Upgrade\r\nSec-WebSocket-Key: %s\r\n"
                  "Sec-WebSocket-Version: 13\r\n\r\n" % (PATH, HOST, key)).encode())
    buf = b""
    while b"\r\n\r\n" not in buf:
        buf = read_more(sock, buf)
    head, _, rest = buf.partition(b"\r\n\r\n")
    lines = head.decode("latin-1").split("\r\n")
    if lines[0].split()[1:2] != ["101"]:
        raise ValueError("upgrade refused: %s" % lines[0])
    want = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
    accept = [line.split(":", 1)[1].strip() for line in lines[1:]
              if line.lower().startswith("sec-websocket-accept:")]
    if accept != [want]:
        raise ValueError("Sec-WebSocket-Accept was %r, expected %r" % (accept, want))
    return rest


def first_frame(sock, buf):
    while len(buf) < 2:
        buf = read_more(sock, buf)
    opcode, size, at = buf[0] & 0x0F, buf[1] & 0x7F, 2
    if size >= 126:
        at = 4 if size == 126 else 10
        while len(buf) < at:
            buf = read_more(sock, buf)
        size = struct.unpack("!H" if at == 4 else "!Q", buf[2:at])[0]
    while len(buf) < at + size:
        buf = read_more(sock, buf)
    return opcode, buf[at:at + size]


def greeting(opcode, payload):
    if opcode == OP_CLOSE:
        code = struct.unpack("!H", payload[:2])[0] if len(payload) >= 2 else None
        return {"ev": "refused", "code": code}
    if opcode == OP_TEXT:
        try:
            frame = json.loads(payload.decode())
        except ValueError:
            frame = None
        if isinstance(frame, dict) and frame.get("op") == "hello":
            return None
    return {"ev": "error", "stage": "hello",
            "error": "the first frame was opcode %d: %r" % (opcode, payload[:80])}


def connect(binary):
    sock = socket.create_connection((HOST, PORT), CONNECT_S)
    state["sock"], state["connected_at"] = sock, time.monotonic()
    opcode, payload = first_frame(sock, upgrade(sock))
    refusal = greeting(opcode, payload)
    if refusal is not None:
        return refusal
    local, peer = sock.getsockname(), sock.getpeername()
    return {"ev": "connected", "local_ip": local[0], "local_port": local[1],
            "peer_ip": peer[0], "peer_port": peer[1], "iptables": binary,
            "hello_s": round(time.monotonic() - state["connected_at"], 3)}


def commands(deadline):
    pending = b""
    while True:
        if b"\n" in pending:
            line, _, pending = pending.partition(b"\n")
            if line.strip():
                yield json.loads(line.decode())
            continue
        left = deadline - time.monotonic()
        if left <= 0:
            emit({"ev": "expired"})
            return
        if not select.select([0], [], [], left)[0]:
            continue
        chunk = os.read(0, 4096)
        if not chunk:
            return
        pending += chunk


def vanish(cmd):
    failsafe = subprocess.Popen(["sh", "-c", cmd["failsafe"]], stdin=subprocess.DEVNULL,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                start_new_session=True)
    failsafe.wait()
    state["failsafe"] = failsafe.pid
    for insert, delete in zip(cmd["insert"], cmd["delete"]):
        state["deletes"].append(delete)
        code, err = run(insert)
        if code != 0:
            state["deletes"].pop()
            return {"ev": "error", "stage": "rules",
                    "error": "%s exited %d: %s" % (" ".join(insert), code, err)}
    return {"ev": "vanished",
            "connected_to_rules_s": round(time.monotonic() - state["connected_at"], 3)}


def deleted(argv):
    return any(run(argv)[0] == 0 for _ in range(DELETE_ATTEMPTS))


def remove_rules():
    failed = [argv for argv in state["deletes"] if not deleted(argv)]
    state["deletes"] = []
    if not failed and state["failsafe"] is not None:
        try:
            os.killpg(state["failsafe"], signal.SIGTERM)
        except OSError:
            pass
        state["failsafe"] = None
    return failed


def abort_socket():
    sock, state["sock"] = state["sock"], None
    if sock is None:
        return
    try:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    except OSError:
        pass
    sock.close()


def release():
    failed = remove_rules()
    abort_socket()
    return {"ev": "released", "failed": failed}


def serve(binary):
    deadline = time.monotonic() + HOLD_S
    try:
        hello = connect(binary)
    except (OSError, EOFError, ValueError) as exc:
        emit({"ev": "error", "stage": "connect", "error": "%s: %s" % (type(exc).__name__, exc)})
        return
    emit(hello)
    if hello["ev"] != "connected":
        return
    for cmd in commands(deadline):
        if cmd.get("cmd") == "vanish":
            emit(vanish(cmd))
        elif cmd.get("cmd") == "release":
            emit(release())
            return


def main():
    for sig in SIGNALS:
        signal.signal(sig, interrupted)
    binary, why = pick_iptables()
    if binary is None:
        emit({"ev": "unavailable", "error": why})
        return
    try:
        serve(binary)
    finally:
        for sig in SIGNALS:
            signal.signal(sig, signal.SIG_IGN)
        failed = remove_rules()
        abort_socket()
        if failed:
            emit({"ev": "leftover", "failed": failed})


main()
"""


def controller_address(base):
    parts = urlsplit(base if "//" in base else "http://" + base)
    return parts.hostname, parts.port or DEFAULT_HTTP_PORT


def remote_command(host, port, candidates, hold_s):
    args = [host, str(port), WS_PATH, str(hold_s), str(CONNECT_TIMEOUT_S),
            str(XTABLES_WAIT_S), ",".join(candidates)]
    return "exec %s -u -c %s %s" % (INTERPRETER, shlex.quote(_SCRIPT),
                                    " ".join(shlex.quote(arg) for arg in args))


def tag(local_port, nonce):
    return "%s%d-%s" % (TAG_PREFIX, local_port, nonce)


def drop_specs(peer, nonce):
    verdict = ["-m", "comment", "--comment", tag(peer.local_port, nonce), "-j", "DROP"]
    out = ["-p", "tcp", "-s", peer.local_ip, "--sport", str(peer.local_port),
           "-d", peer.peer_ip, "--dport", str(peer.peer_port)]
    back = ["-p", "tcp", "-s", peer.peer_ip, "--sport", str(peer.peer_port),
            "-d", peer.local_ip, "--dport", str(peer.local_port)]
    return [("OUTPUT", out + verdict), ("INPUT", back + verdict)]


def insert_argv(binary, chain, spec):
    return [binary, "-w", str(XTABLES_WAIT_S), "-I", chain, "1", *spec]


def delete_argv(binary, chain, spec):
    return [binary, "-w", str(XTABLES_WAIT_S), "-D", chain, *spec]


def listing_argv(binary, chain):
    return [binary, "-w", str(XTABLES_WAIT_S), "-S", chain]


def failsafe_command(deletes, seconds):
    inner = "; ".join(["sleep %d" % seconds] + [shlex.join(argv) for argv in deletes])
    return "nohup sh -c %s </dev/null >/dev/null 2>&1 &" % shlex.quote(inner)


def vanish_command(binary, peer, nonce, failsafe_s):
    specs = drop_specs(peer, nonce)
    deletes = [delete_argv(binary, chain, spec) for chain, spec in specs]
    return {"cmd": "vanish",
            "insert": [insert_argv(binary, chain, spec) for chain, spec in specs],
            "delete": deletes,
            "failsafe": failsafe_command(deletes, failsafe_s)}


def parse_event(line):
    try:
        doc = json.loads(line)
    except ValueError:
        return None
    if isinstance(doc, dict) and isinstance(doc.get("ev"), str):
        return doc
    return None


def tuple4(event):
    return Tuple4(str(event["local_ip"]), int(event["local_port"]),
                  str(event["peer_ip"]), int(event["peer_port"]))


def tagged_rules(listing):
    rules = []
    for line in listing.splitlines():
        try:
            words = shlex.split(line)
        except ValueError:
            continue
        if len(words) < 2 or words[0] != "-A":
            continue
        comments = [value for flag, value in zip(words, words[1:]) if flag == "--comment"]
        if any(value.startswith(TAG_PREFIX) for value in comments):
            rules.append((words[1], words[2:]))
    return rules


def window(connected_to_rules_s):
    return (KEEPALIVE_ABORT_S - connected_to_rules_s - EARLY_SLACK_S,
            KEEPALIVE_ABORT_S + LATE_SLACK_S)


def first_freed(before, samples):
    return next((at for at, clients in samples if clients <= before), None)


def watch_over(before, samples, earliest, latest):
    at, clients = samples[-1]
    if clients > before + 1 or clients < before:
        return True
    freed = first_freed(before, samples)
    if freed is None:
        return at >= latest + OVERRUN_S
    return freed >= earliest or at >= latest


def judge(counted, before, samples, earliest, latest):
    if not counted:
        return NEVER_COUNTED, None
    freed = first_freed(before, samples)
    held = [n for at, n in samples if freed is None or at < freed]
    after = [n for at, n in samples if freed is not None and at >= freed]
    if any(n != before + 1 for n in held) or any(n != before for n in after):
        return NOISE, freed
    if freed is None:
        return NEVER_FREED, None
    if freed < earliest:
        return TOO_EARLY, freed
    if freed > latest:
        return TOO_LATE, freed
    return IN_WINDOW, freed


class VanishingPeer:
    def __init__(self, cfg, nonce=None, candidates=None, hold_s=None, failsafe_s=None):
        self.cfg = cfg
        self.nonce = nonce or os.urandom(NONCE_BYTES).hex()
        self.candidates = candidates or IPTABLES_CANDIDATES
        self.hold_s = hold_s or HOLD_S
        self.failsafe_s = failsafe_s or FAILSAFE_S
        self.seen = []
        self.connected = None
        self.sent_rules = False
        self._events = queue.Queue()
        self._proc = None

    def start(self):
        host, port = controller_address(self.cfg.base)
        remote = remote_command(host, port, self.candidates, self.hold_s)
        try:
            self._proc = subprocess.Popen(ssh_argv(self.cfg, remote), stdin=subprocess.PIPE,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                          text=True, bufsize=1)
        except OSError as exc:
            raise VanishUnavailable("ssh to %s did not start: %s" % (self.cfg.wb_ssh, exc))
        threading.Thread(target=self._read, daemon=True).start()
        event = self._next(START_TIMEOUT_S)
        if event is None or event["ev"] in TROUBLE:
            why = event if event is not None else "no answer in %.0f s" % START_TIMEOUT_S
            self.stop()
            raise VanishUnavailable("the WB-side client did not connect: %s %s"
                                    % (why, self._stderr()))
        if event["ev"] == CONNECTED:
            self.connected = event
        return event

    def vanish(self):
        command = vanish_command(self.connected["iptables"], tuple4(self.connected),
                                 self.nonce, self.failsafe_s)
        self.sent_rules = True
        self._send(command)
        event = self._next(REPLY_TIMEOUT_S)
        if event is None or event["ev"] != VANISHED:
            raise VanishUnavailable("the WB did not black-hole the connection: %r" % (event,))
        return event

    def release(self):
        if self._proc is None or self._proc.poll() is not None:
            return None
        try:
            self._send({"cmd": "release"})
        except (OSError, ValueError):
            return None
        return self._next(REPLY_TIMEOUT_S)

    def stop(self):
        if self._proc is None:
            return
        try:
            self._proc.stdin.close()
        except (OSError, ValueError):
            pass
        try:
            self._proc.wait(timeout=REPLY_TIMEOUT_S)
        except subprocess.TimeoutExpired:
            self._proc.kill()
            self._proc.wait()

    def _send(self, doc):
        self._proc.stdin.write(json.dumps(doc) + "\n")
        self._proc.stdin.flush()

    def _next(self, timeout_s):
        try:
            event = self._events.get(timeout=timeout_s)
        except queue.Empty:
            return None
        self.seen.append(event)
        return event

    def _read(self):
        for line in self._proc.stdout:
            event = parse_event(line)
            if event is not None:
                self._events.put(event)
        self._events.put({"ev": GONE})

    def _stderr(self):
        try:
            return (self._proc.stderr.read() or "")[-300:]
        except (OSError, ValueError):
            return ""


def _remote(cfg, argv_list):
    command = "; ".join(shlex.join(argv) for argv in argv_list)
    try:
        done = subprocess.run(ssh_argv(cfg, command), capture_output=True, text=True,
                              timeout=REPLY_TIMEOUT_S)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise VanishUnavailable("ssh to %s failed: %s" % (cfg.wb_ssh, exc))
    if done.returncode == SSH_FAILED:
        raise VanishUnavailable("ssh to %s failed: %s" % (cfg.wb_ssh, done.stderr[-300:]))
    return done


def present_binaries(cfg, candidates=None):
    probe = [["command", "-v", name] for name in candidates or IPTABLES_CANDIDATES]
    return [line.strip() for line in _remote(cfg, probe).stdout.splitlines() if line.strip()]


def listed(cfg, binary):
    done = _remote(cfg, [listing_argv(binary, chain) for chain in CHAINS])
    if done.returncode != 0:
        raise VanishUnavailable("%s -S on the WB exited %d: %s"
                                % (binary, done.returncode, done.stderr[-300:]))
    return tagged_rules(done.stdout)


def sweep(cfg, candidates=None):
    removed, left = [], []
    for binary in present_binaries(cfg, candidates):
        for chain, spec in listed(cfg, binary):
            _remote(cfg, [delete_argv(binary, chain, spec)])
            removed.append((binary, chain, spec))
        left += [(binary, chain, spec) for chain, spec in listed(cfg, binary)]
    return removed, left


def main(argv):
    if argv[1:] != ["sweep"]:
        print(USAGE, file=sys.stderr)
        return 64
    removed, left = sweep(config.load())
    for binary, chain, spec in removed:
        print("removed: %s" % shlex.join(delete_argv(binary, chain, spec)))
    for binary, chain, spec in left:
        print("LEFT: %s -A %s %s" % (binary, chain, shlex.join(spec)))
    return 1 if left else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
