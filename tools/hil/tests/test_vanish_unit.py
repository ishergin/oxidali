import dataclasses
import json
import re
import shlex
import signal
import socket
import struct
import subprocess
import sys
import threading
from pathlib import Path

import pytest

import test_ws
from hil import vanish, wsclient
from hil.config import HilConfig
from hil.wait import wait_until

NONCE = "a1b2c3"
WB = vanish.Tuple4("192.168.13.110", 45678, "192.168.13.116", 80)
BINARY = "/usr/sbin/iptables-legacy"
SERVER_WAIT_S = 30.0
SETTLE_S = 10.0
POLL_S = 0.05
FAILSAFE_TEST_S = 1
HELLO = json.dumps({"op": "hello", "channels": ["diagnostics"]}).encode()
TEXT_FIN = 0x81
CLOSE_FIN = 0x88
WIRE = (Path(__file__).resolve().parents[3] / "crates" / "dali2rust-adapters" / "src" / "http"
        / "esp_ws_wire.rs")
FIRMWARE_CONSTANT = r"const %s: i32 = (\d+);"

FAKE_IPTABLES = """#!{python}
import json, pathlib, shlex, sys
log, table = pathlib.Path({log!r}), pathlib.Path({table!r})
argv = sys.argv[1:]
with log.open("a") as fh:
    fh.write(json.dumps(argv) + "\\n")
if argv[:1] == ["-w"]:
    argv = argv[2:]
rules = table.read_text().splitlines() if table.exists() else []
verb, chain, rest = argv[0], argv[1], argv[2:]
if verb == "-S":
    print("-P %s ACCEPT" % chain)
    print("\\n".join(line for line in rules if shlex.split(line)[1] == chain))
elif verb == "-I":
    rules.insert(0, "-A %s %s" % (chain, shlex.join(rest[1:])))
elif verb == "-D":
    match = [line for line in rules if shlex.split(line) == ["-A", chain] + rest]
    if not match:
        sys.stderr.write("iptables: Bad rule (does a matching rule exist in that chain?).\\n")
        sys.exit(1)
    rules.remove(match[0])
table.write_text("".join(line + "\\n" for line in rules))
"""

LISTING = """-P INPUT ACCEPT
-P OUTPUT ACCEPT
-A INPUT -s 192.168.13.116/32 -d 192.168.13.110/32 -p tcp -m tcp --sport 80 --dport 45678 -m comment --comment hil-ws-vanish-45678-a1b2c3 -j DROP
-A INPUT -i docker0 -m comment --comment "docker bridge" -j ACCEPT
-A INPUT -p tcp -m tcp --dport 22 -j ACCEPT
-A OUTPUT -s 192.168.13.110/32 -d 192.168.13.116/32 -p tcp -m tcp --sport 45678 --dport 80 -m comment --comment "hil-ws-vanish-45678-a1b2c3" -j DROP
-A OUTPUT -m comment --comment "not hil-ws-vanish-1" -j ACCEPT
"""


def _local(cfg, remote):
    return ["sh", "-c", remote]


def _frame(first, payload):
    return bytes([first, len(payload)]) + payload


class FakeController:
    def __init__(self, first_frame):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(1)
        self.listener.settimeout(SERVER_WAIT_S)
        self.port = self.listener.getsockname()[1]
        self.peer, self.ended = None, None
        self.thread = threading.Thread(target=self._serve, args=(first_frame,), daemon=True)
        self.thread.start()

    def _serve(self, first_frame):
        try:
            conn, self.peer = self.listener.accept()
        except OSError:
            return
        with conn:
            conn.settimeout(SERVER_WAIT_S)
            request = b""
            while b"\r\n\r\n" not in request:
                request += conn.recv(4096)
            key = wsclient.header_value(request.decode("latin-1"), "Sec-WebSocket-Key")
            conn.sendall(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
                          "Connection: Upgrade\r\nSec-WebSocket-Accept: %s\r\n\r\n"
                          % wsclient.accept_for_key(key)).encode() + first_frame)
            self.ended = self._outcome(conn)

    @staticmethod
    def _outcome(conn):
        try:
            return "data" if conn.recv(4096) else "eof"
        except ConnectionResetError:
            return "reset"
        except socket.timeout:
            return "timeout"

    def close(self):
        self.listener.close()


class Bench:
    def __init__(self, tmp_path):
        self.fake = tmp_path / "iptables"
        self.log = tmp_path / "calls.jsonl"
        self.table = tmp_path / "table"
        self.fake.write_text(FAKE_IPTABLES.format(python=sys.executable, log=str(self.log),
                                                  table=str(self.table)))
        self.fake.chmod(0o755)

    def calls(self, verb):
        if not self.log.exists():
            return []
        argvs = [json.loads(line) for line in self.log.read_text().splitlines()]
        return [argv for argv in argvs if verb in argv]

    def rules(self):
        return self.table.read_text().splitlines() if self.table.exists() else []

    def peer(self, controller, candidates=None, failsafe_s=None):
        cfg = HilConfig(base="http://127.0.0.1:%d" % controller.port, serial_remote="",
                        wb_ssh="root@127.0.0.1", allow_wb_packet_filter=True)
        return vanish.VanishingPeer(cfg, nonce=NONCE, candidates=candidates or [str(self.fake)],
                                    failsafe_s=failsafe_s)


@pytest.fixture()
def bench(tmp_path, monkeypatch):
    monkeypatch.setattr(vanish, "ssh_argv", _local)
    monkeypatch.setattr(vanish, "INTERPRETER", sys.executable)
    return Bench(tmp_path)


@pytest.fixture()
def controller():
    served = FakeController(_frame(TEXT_FIN, HELLO))
    yield served
    served.close()


def _vanished(bench, controller, failsafe_s=None):
    peer = bench.peer(controller, failsafe_s=failsafe_s)
    assert peer.start()["ev"] == vanish.CONNECTED
    assert peer.vanish()["ev"] == vanish.VANISHED
    assert len(bench.rules()) == 2
    return peer


def test_the_keepalive_constants_mirror_the_firmware():
    source = WIRE.read_text()
    for name in ("KEEPALIVE_IDLE_S", "KEEPALIVE_INTERVAL_S", "KEEPALIVE_PROBES"):
        assert int(re.search(FIRMWARE_CONSTANT % name, source).group(1)) == getattr(vanish, name)


def test_the_window_opens_at_the_abort_less_the_time_before_the_rules_and_closes_with_slack():
    assert vanish.KEEPALIVE_ABORT_S == 30 + 10 * 3
    assert vanish.window(4.0) == (60 - 4.0 - vanish.EARLY_SLACK_S, 60 + vanish.LATE_SLACK_S)


def test_the_rules_drop_exactly_the_connections_four_tuple_in_both_directions():
    specs = dict(vanish.drop_specs(WB, NONCE))
    tagged = ["-m", "comment", "--comment", "hil-ws-vanish-45678-a1b2c3", "-j", "DROP"]
    assert specs == {
        "OUTPUT": ["-p", "tcp", "-s", "192.168.13.110", "--sport", "45678",
                   "-d", "192.168.13.116", "--dport", "80"] + tagged,
        "INPUT": ["-p", "tcp", "-s", "192.168.13.116", "--sport", "80",
                  "-d", "192.168.13.110", "--dport", "45678"] + tagged,
    }
    assert [chain for chain, _ in vanish.drop_specs(WB, NONCE)] == list(vanish.CHAINS)


def test_an_insert_goes_first_in_its_chain_and_a_delete_names_the_same_rule():
    spec = dict(vanish.drop_specs(WB, NONCE))["INPUT"]
    assert vanish.insert_argv(BINARY, "INPUT", spec) == [BINARY, "-w", "5", "-I", "INPUT", "1",
                                                         *spec]
    assert vanish.delete_argv(BINARY, "INPUT", spec) == [BINARY, "-w", "5", "-D", "INPUT", *spec]


def test_the_vanish_command_pairs_each_insert_with_its_delete_and_arms_the_failsafe_on_them():
    command = vanish.vanish_command(BINARY, WB, NONCE, vanish.FAILSAFE_S)
    assert [argv[3:5] for argv in command["insert"]] == [["-I", "OUTPUT"], ["-I", "INPUT"]]
    assert [argv[3:5] for argv in command["delete"]] == [["-D", "OUTPUT"], ["-D", "INPUT"]]
    for insert, delete in zip(command["insert"], command["delete"]):
        assert insert[6:] == delete[5:]
    assert command["failsafe"] == vanish.failsafe_command(command["delete"], vanish.FAILSAFE_S)


def test_the_failsafe_sleeps_past_the_hold_then_deletes_both_rules_detached():
    deletes = vanish.vanish_command(BINARY, WB, NONCE, vanish.FAILSAFE_S)["delete"]
    words = shlex.split(vanish.failsafe_command(deletes, vanish.FAILSAFE_S))
    assert words[:3] == ["nohup", "sh", "-c"]
    assert words[4:] == ["</dev/null", ">/dev/null", "2>&1", "&"]
    assert words[3] == "; ".join(["sleep %d" % vanish.FAILSAFE_S]
                                 + [shlex.join(argv) for argv in deletes])
    assert vanish.FAILSAFE_S > vanish.HOLD_S > (vanish.KEEPALIVE_ABORT_S + vanish.LATE_SLACK_S
                                                + vanish.OVERRUN_S)


def test_the_failsafe_runs_its_deletes_after_the_shell_that_started_it_is_gone(bench):
    bench.table.write_text("".join(
        "-A %s %s\n" % (chain, shlex.join(spec)) for chain, spec in vanish.drop_specs(WB, NONCE)))
    deletes = [vanish.delete_argv(str(bench.fake), chain, spec)
               for chain, spec in vanish.drop_specs(WB, NONCE)]
    started = subprocess.run(["sh", "-c", vanish.failsafe_command(deletes, 0)], timeout=SETTLE_S)
    assert started.returncode == 0
    assert wait_until(lambda: not bench.rules(), SETTLE_S, interval_s=POLL_S), bench.rules()


def test_only_json_lines_naming_an_event_are_events():
    assert vanish.parse_event('{"ev": "connected", "local_port": 45678}\n') == {
        "ev": "connected", "local_port": 45678}
    assert vanish.parse_event("Warning: Permanently added '192.168.13.110'\n") is None
    assert vanish.parse_event('{"op": "hello"}') is None
    assert vanish.parse_event('["ev"]') is None
    assert vanish.parse_event("") is None


def test_the_connected_event_becomes_the_four_tuple():
    event = {"ev": "connected", "local_ip": "192.168.13.110", "local_port": 45678,
             "peer_ip": "192.168.13.116", "peer_port": 80}
    assert vanish.tuple4(event) == WB


def test_the_controller_address_comes_from_the_base_url():
    assert vanish.controller_address("http://192.168.13.116") == ("192.168.13.116", 80)
    assert vanish.controller_address("192.168.13.116:8080") == ("192.168.13.116", 8080)


BEFORE = 1
EARLIEST, LATEST = vanish.window(4.0)


def _held_until(freed, then=BEFORE, step=1.0):
    samples, at = [], 0.0
    while at < freed:
        samples.append((at, BEFORE + 1))
        at += step
    return samples + [(freed, then)]


def test_a_slot_freed_inside_the_window_passes():
    assert vanish.judge(True, BEFORE, _held_until(57.0), EARLIEST, LATEST) == (
        vanish.IN_WINDOW, 57.0)


def test_a_slot_freed_before_keepalive_can_abort_is_too_early():
    samples = _held_until(20.0) + [(at, BEFORE) for at in (40.0, 60.0, LATEST)]
    assert vanish.judge(True, BEFORE, samples, EARLIEST, LATEST) == (vanish.TOO_EARLY, 20.0)


def test_a_slot_freed_after_the_slack_is_too_late():
    assert vanish.judge(True, BEFORE, _held_until(100.0), EARLIEST, LATEST) == (
        vanish.TOO_LATE, 100.0)


def test_a_slot_never_freed_and_a_client_never_counted_are_their_own_verdicts():
    held = [(at, BEFORE + 1) for at in range(0, 121, 10)]
    assert vanish.judge(True, BEFORE, held, EARLIEST, LATEST) == (vanish.NEVER_FREED, None)
    assert vanish.judge(False, BEFORE, [], None, None) == (vanish.NEVER_COUNTED, None)


def test_a_co_tenant_that_joins_or_leaves_makes_the_window_noise():
    joined = [(0.0, BEFORE + 1), (10.0, BEFORE + 2), (57.0, BEFORE + 1)]
    assert vanish.judge(True, BEFORE, joined, EARLIEST, LATEST)[0] == vanish.NOISE
    left_early = _held_until(20.0) + [(40.0, BEFORE), (57.0, BEFORE - 1)]
    assert vanish.judge(True, BEFORE, left_early, EARLIEST, LATEST)[0] == vanish.NOISE
    came_back = _held_until(20.0) + [(30.0, BEFORE + 1), (57.0, BEFORE)]
    assert vanish.judge(True, BEFORE, came_back, EARLIEST, LATEST)[0] == vanish.NOISE


def test_the_watch_stops_at_a_crossing_inside_the_window_and_on_visible_noise():
    assert not vanish.watch_over(BEFORE, _held_until(30.0)[:-1], EARLIEST, LATEST)
    assert vanish.watch_over(BEFORE, _held_until(57.0), EARLIEST, LATEST)
    assert vanish.watch_over(BEFORE, [(0.0, BEFORE + 1), (3.0, BEFORE + 2)], EARLIEST, LATEST)
    assert vanish.watch_over(BEFORE, _held_until(20.0) + [(30.0, BEFORE - 1)], EARLIEST, LATEST)


def test_the_watch_follows_an_early_crossing_to_the_ceiling_and_gives_up_after_the_overrun():
    early = _held_until(20.0)
    assert not vanish.watch_over(BEFORE, early + [(LATEST - 1, BEFORE)], EARLIEST, LATEST)
    assert vanish.watch_over(BEFORE, early + [(LATEST, BEFORE)], EARLIEST, LATEST)
    held = [(0.0, BEFORE + 1), (LATEST + vanish.OVERRUN_S - 1, BEFORE + 1)]
    assert not vanish.watch_over(BEFORE, held, EARLIEST, LATEST)
    held.append((LATEST + vanish.OVERRUN_S, BEFORE + 1))
    assert vanish.watch_over(BEFORE, held, EARLIEST, LATEST)


def test_the_listing_yields_only_rules_tagged_by_the_test_quoted_or_not():
    rules = vanish.tagged_rules(LISTING)
    assert rules == [
        ("INPUT", ["-s", "192.168.13.116/32", "-d", "192.168.13.110/32", "-p", "tcp", "-m", "tcp",
                   "--sport", "80", "--dport", "45678", "-m", "comment", "--comment",
                   "hil-ws-vanish-45678-a1b2c3", "-j", "DROP"]),
        ("OUTPUT", ["-s", "192.168.13.110/32", "-d", "192.168.13.116/32", "-p", "tcp", "-m",
                    "tcp", "--sport", "45678", "--dport", "80", "-m", "comment", "--comment",
                    "hil-ws-vanish-45678-a1b2c3", "-j", "DROP"]),
    ]
    assert vanish.tagged_rules("-A INPUT -m comment --comment 'unbalanced -j DROP\n") == []


def test_the_sweep_deletes_every_tagged_rule_and_nothing_else(bench):
    bench.table.write_text(LISTING.replace("-P INPUT ACCEPT\n-P OUTPUT ACCEPT\n", ""))
    cfg = HilConfig(base="http://127.0.0.1:1", serial_remote="", wb_ssh="root@127.0.0.1")
    removed, left = vanish.sweep(cfg, candidates=[str(bench.fake)])
    assert [(chain, spec[-3]) for _, chain, spec in removed] == [
        ("OUTPUT", "hil-ws-vanish-45678-a1b2c3"), ("INPUT", "hil-ws-vanish-45678-a1b2c3")]
    assert left == []
    assert [line for line in bench.rules() if vanish.TAG_PREFIX in line] == [
        '-A OUTPUT -m comment --comment "not hil-ws-vanish-1" -j ACCEPT']


def _failsafe_alive(peer):
    waiting = vanish.tag(peer.connected["local_port"], NONCE)
    return subprocess.run(["pgrep", "-f", waiting], capture_output=True).returncode == 0


def test_the_wb_client_holds_its_four_tuple_silent_then_releases_with_a_reset(bench, controller):
    peer = _vanished(bench, controller)
    try:
        assert vanish.tuple4(peer.connected) == vanish.Tuple4(
            "127.0.0.1", controller.peer[1], "127.0.0.1", controller.port)
        assert peer.connected["iptables"] == str(bench.fake)
        assert all(vanish.tag(controller.peer[1], NONCE) in rule for rule in bench.rules())
        assert controller.ended is None, "the client spoke or closed before its release"
        assert _failsafe_alive(peer), "no failsafe is waiting to delete the rules"
        assert peer.release() == {"ev": "released", "failed": []}
        assert bench.rules() == [] and len(bench.calls("-D")) == 2
        assert wait_until(lambda: controller.ended, SETTLE_S, interval_s=POLL_S) == "reset"
        assert wait_until(lambda: not _failsafe_alive(peer), SETTLE_S, interval_s=POLL_S), \
            "the failsafe outlived a clean release"
    finally:
        peer.stop()


@pytest.mark.parametrize("end", ["eof", "sigterm"])
def test_the_rules_go_when_the_channel_closes_or_the_client_is_told_to_stop(bench, controller,
                                                                            end):
    peer = _vanished(bench, controller)
    try:
        if end == "eof":
            peer._proc.stdin.close()
        else:
            peer._proc.send_signal(signal.SIGTERM)
        assert wait_until(lambda: not bench.rules(), SETTLE_S, interval_s=POLL_S), bench.rules()
        assert wait_until(lambda: controller.ended, SETTLE_S, interval_s=POLL_S) == "reset"
    finally:
        peer.stop()


def test_the_failsafe_removes_the_rules_of_a_client_killed_outright(bench, controller):
    peer = _vanished(bench, controller, failsafe_s=FAILSAFE_TEST_S)
    try:
        peer._proc.kill()
        assert wait_until(lambda: not bench.rules(), FAILSAFE_TEST_S + SETTLE_S,
                          interval_s=POLL_S), bench.rules()
    finally:
        peer.stop()


def test_a_full_pool_is_a_refusal_with_its_close_code(bench):
    refused = FakeController(_frame(CLOSE_FIN, struct.pack("!H", wsclient.TRY_AGAIN_LATER)))
    peer = bench.peer(refused)
    try:
        assert peer.start() == {"ev": vanish.REFUSED, "code": wsclient.TRY_AGAIN_LATER}
        assert peer.connected is None
    finally:
        peer.stop()
        refused.close()
    assert bench.rules() == []


def test_a_wb_without_iptables_is_unavailable_before_any_upgrade(bench, controller):
    peer = bench.peer(controller, candidates=[str(bench.fake) + "-missing"])
    with pytest.raises(vanish.VanishUnavailable, match="is installed"):
        peer.start()
    assert controller.peer is None, "the upgrade took a slot although no rule could go in"


def test_the_hardware_test_is_slow_carries_its_id_and_does_not_need_the_serial_console():
    marks = {mark.name: mark for mark in
             test_ws.test_a_client_gone_from_the_network_is_reclaimed_by_keepalive.pytestmark}
    assert "slow" in marks and "serial" not in marks
    assert marks["hil_id"].args == ("HIL-WS-11",)


class Untouchable:
    def __init__(self):
        self.reached = []

    def __getattr__(self, name):
        self.reached.append(name)
        raise AssertionError("the gate let the run reach %s" % name)


def _recording(reached, name):
    def record(*args, **kwargs):
        reached.append(name)
        raise AssertionError("the gate let the run reach %s" % name)
    return record


def test_the_go_ahead_is_read_from_its_own_flag_and_off_by_default(monkeypatch):
    monkeypatch.delenv(vanish.PACKET_FILTER_ENV, raising=False)
    assert not HilConfig(serial_remote="").allow_wb_packet_filter
    monkeypatch.setenv(vanish.PACKET_FILTER_ENV, "0")
    assert not HilConfig(serial_remote="").allow_wb_packet_filter
    monkeypatch.setenv(vanish.PACKET_FILTER_ENV, "1")
    assert HilConfig(serial_remote="").allow_wb_packet_filter


def test_without_the_go_ahead_nothing_reaches_the_wb_command_builders(monkeypatch):
    reached = []
    for name in ("ssh_argv", "remote_command", "vanish_command", "failsafe_command"):
        monkeypatch.setattr(vanish, name, _recording(reached, name))
    monkeypatch.setattr(vanish.subprocess, "Popen", _recording(reached, "Popen"))
    cfg = HilConfig(base="http://127.0.0.1:1", serial_remote="", wb_ssh="root@127.0.0.1",
                    allow_wb_packet_filter=False)
    with pytest.raises(vanish.PacketFilterNotAllowed, match=vanish.PACKET_FILTER_ENV):
        vanish.VanishingPeer(cfg).start()
    assert reached == []
    assert "production WB" in vanish.packet_filter_refusal(cfg)
    assert vanish.packet_filter_refusal(dataclasses.replace(cfg, allow_wb_packet_filter=True)) \
        is None


def test_the_hardware_test_skips_on_the_missing_go_ahead_before_it_touches_anything(
        monkeypatch):
    built = []
    monkeypatch.setattr(vanish, "VanishingPeer", _recording(built, "VanishingPeer"))
    cfg = HilConfig(base="http://127.0.0.1:1", serial_remote="", wb_ssh="root@127.0.0.1",
                    allow_wb_packet_filter=False)
    api, baseline, artifacts = Untouchable(), Untouchable(), Untouchable()
    with pytest.raises(pytest.skip.Exception, match=vanish.PACKET_FILTER_ENV):
        test_ws.test_a_client_gone_from_the_network_is_reclaimed_by_keepalive(
            api, cfg, baseline, artifacts)
    assert built == [] and api.reached == baseline.reached == artifacts.reached == []
