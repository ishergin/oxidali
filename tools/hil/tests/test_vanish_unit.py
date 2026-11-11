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
import time
import types
from pathlib import Path

import pytest

import test_ws
from hil import vanish, wsclient
from hil.config import HilConfig
from hil.wait import wait_until

NONCE = "a1b2c3"
WB = vanish.Tuple4("192.168.13.110", 45678, "192.168.13.116", 80)
TAG = vanish.tag(WB.local_port, NONCE)
BINARY = "/usr/sbin/iptables-legacy"
SERVER_WAIT_S = 30.0
SETTLE_S = 10.0
POLL_S = 0.05
QUICK_S = 0.05
FAILSAFE_TEST_S = 1
HOLD_TEST_S = 4
HELLO = json.dumps({"op": "hello", "channels": ["diagnostics"]}).encode()
TEXT_FIN = 0x81
CLOSE_FIN = 0x88
WIRE = (Path(__file__).resolve().parents[3] / "crates" / "dali2rust-adapters" / "src" / "http"
        / "esp_ws_wire.rs")
FIRMWARE_CONSTANT = r"const %s: i32 = (\d+);"
FAILSAFE_SHELL = "^sh -c .*sleep [0-9]+; .*%s"
SIGNALS = {"sigterm": signal.SIGTERM, "sighup": signal.SIGHUP}
SWEEP_CONFIG = HilConfig(base="http://127.0.0.1:1", serial_remote="", wb_ssh="root@127.0.0.1")
STUB_CONFIG = HilConfig(base="http://127.0.0.1:1", serial_remote="", wb_ssh="nobody@wb.invalid",
                        allow_wb_packet_filter=True)
LEFT_RULE = [BINARY, "INPUT", ["-m", "comment", "--comment", TAG, "-j", "DROP"]]

FAKE_IPTABLES = r"""
import json, pathlib, shlex, subprocess, sys, time

HERE = pathlib.Path(sys.argv[0]).parent
TABLE, LOG, FLAKED = HERE / "table", HERE / "calls.jsonl", HERE / "flaked"


def knob(name):
    return (HERE / name).exists()


def failsafe_alive(spec):
    tag = spec[spec.index("--comment") + 1]
    found = subprocess.run(["pgrep", "-f", "@FAILSAFE@" % tag], capture_output=True)
    return found.returncode == 0


def flaked(line):
    seen = FLAKED.read_text().splitlines() if FLAKED.exists() else []
    if not knob("flaky-delete") or line in seen:
        return False
    FLAKED.write_text("".join(item + "\n" for item in seen + [line]))
    return True


def main(argv):
    entry = {"argv": argv}
    if argv[:1] == ["-w"]:
        argv = argv[2:]
    verb, chain, rest = argv[0], argv[1], argv[2:]
    rules = TABLE.read_text().splitlines() if TABLE.exists() else []
    code = 0
    if verb == "-S" and knob("fail-list-" + chain):
        code = 1
    elif verb == "-S":
        print("-P %s ACCEPT" % chain)
        print("\n".join(line for line in rules if shlex.split(line)[1] == chain))
    elif verb == "-I":
        entry["failsafe"] = failsafe_alive(rest)
        rules.insert(0, "-A %s %s" % (chain, shlex.join(rest[1:])))
    else:
        match = [line for line in rules if shlex.split(line) == ["-A", chain] + rest]
        if not match or knob("refuse-delete") or flaked(match[0]):
            code = 1
        else:
            rules.remove(match[0])
    TABLE.write_text("".join(line + "\n" for line in rules))
    with LOG.open("a") as fh:
        fh.write(json.dumps(dict(entry, code=code, at=time.time())) + "\n")
    if code:
        sys.stderr.write("iptables: Bad rule (does a matching rule exist in that chain?).\n")
    return code


sys.exit(main(sys.argv[1:]))
"""

LISTING = """-P INPUT ACCEPT
-P OUTPUT ACCEPT
-A INPUT -s 192.168.13.116/32 -d 192.168.13.110/32 -p tcp -m tcp --sport 80 --dport 45678 -m comment --comment hil-ws-vanish-45678-a1b2c3 -j DROP
-A INPUT -i docker0 -m comment --comment "docker bridge" -j ACCEPT
-A INPUT -p tcp -m tcp --dport 22 -j ACCEPT
-A OUTPUT -s 192.168.13.110/32 -d 192.168.13.116/32 -p tcp -m tcp --sport 45678 --dport 80 -m comment --comment "hil-ws-vanish-45678-a1b2c3" -j DROP
-A OUTPUT -m comment --comment "not hil-ws-vanish-1" -j ACCEPT
"""
TABLE = LISTING.replace("-P INPUT ACCEPT\n-P OUTPUT ACCEPT\n", "")


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
        self.peer, self.ended, self.ended_at = None, None, None
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
            outcome = self._outcome(conn)
            self.ended_at = time.time()
            self.ended = outcome

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
        self.dir = tmp_path
        self.fake = tmp_path / "iptables"
        self.log = tmp_path / "calls.jsonl"
        self.table = tmp_path / "table"
        self.fake.write_text("#!%s\n%s" % (sys.executable,
                                           FAKE_IPTABLES.replace("@FAILSAFE@", FAILSAFE_SHELL)))
        self.fake.chmod(0o755)

    def knob(self, name):
        (self.dir / name).touch()

    def entries(self, verb):
        if not self.log.exists():
            return []
        entries = [json.loads(line) for line in self.log.read_text().splitlines()]
        return [entry for entry in entries if verb in entry["argv"]]

    def calls(self, verb):
        return [entry["argv"] for entry in self.entries(verb)]

    def rules(self):
        return self.table.read_text().splitlines() if self.table.exists() else []

    def failsafe_alive(self):
        found = subprocess.run(["pgrep", "-f", FAILSAFE_SHELL % self.fake], capture_output=True)
        return found.returncode == 0

    def disarm(self):
        subprocess.run(["pkill", "-f", FAILSAFE_SHELL % self.fake], capture_output=True)

    def peer(self, controller, candidates=None, failsafe_s=None, hold_s=None):
        cfg = HilConfig(base="http://127.0.0.1:%d" % controller.port, serial_remote="",
                        wb_ssh="root@127.0.0.1", allow_wb_packet_filter=True)
        return vanish.VanishingPeer(cfg, nonce=NONCE, candidates=candidates or [str(self.fake)],
                                    hold_s=hold_s, failsafe_s=failsafe_s)


@pytest.fixture()
def bench(tmp_path, monkeypatch):
    monkeypatch.setattr(vanish, "ssh_argv", _local)
    monkeypatch.setattr(vanish, "INTERPRETER", sys.executable)
    served = Bench(tmp_path)
    yield served
    served.disarm()


@pytest.fixture()
def controller():
    served = FakeController(_frame(TEXT_FIN, HELLO))
    yield served
    served.close()


def _vanished(bench, controller, failsafe_s=None, hold_s=None):
    peer = bench.peer(controller, failsafe_s=failsafe_s, hold_s=hold_s)
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
    assert command["armed"] == vanish.FAILSAFE_ARMED


def test_the_failsafe_is_the_shell_the_checks_look_for_and_outlives_the_hold():
    deletes = vanish.vanish_command(BINARY, WB, NONCE, vanish.FAILSAFE_S)["delete"]
    argv = vanish.failsafe_command(deletes, vanish.FAILSAFE_S)
    assert argv[:3] == ["nohup", "sh", "-c"]
    assert re.search(FAILSAFE_SHELL % TAG, " ".join(argv[1:]))
    assert vanish.FAILSAFE_S > vanish.HOLD_S > (vanish.KEEPALIVE_ABORT_S + vanish.LATE_SLACK_S
                                                + vanish.OVERRUN_S)


def test_the_failsafe_reports_itself_armed_then_deletes_both_rules_on_its_own(bench):
    bench.table.write_text("".join(
        "-A %s %s\n" % (chain, shlex.join(spec)) for chain, spec in vanish.drop_specs(WB, NONCE)))
    deletes = [vanish.delete_argv(str(bench.fake), chain, spec)
               for chain, spec in vanish.drop_specs(WB, NONCE)]
    failsafe = subprocess.Popen(vanish.failsafe_command(deletes, 0), stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                start_new_session=True)
    with failsafe.stdout:
        assert failsafe.stdout.readline().decode().strip() == vanish.FAILSAFE_ARMED
    assert wait_until(lambda: not bench.rules(), SETTLE_S, interval_s=POLL_S), bench.rules()
    assert failsafe.wait(SETTLE_S) == 0


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
WINDOW = (EARLIEST, LATEST)
COUNTED = [BEFORE, BEFORE + 1]


def _held_until(freed, then=BEFORE, step=1.0):
    samples, at = [], 0.0
    while at < freed:
        samples.append((at, BEFORE + 1))
        at += step
    return samples + [(freed, then)]


def _judged(samples, released=vanish.CLEAN_RELEASE, settled=BEFORE, counting=COUNTED):
    return vanish.judge(BEFORE, counting, samples, WINDOW, released, settled)


def test_a_slot_freed_inside_the_window_passes():
    assert _judged(_held_until(57.0)) == (vanish.IN_WINDOW, 57.0)


def test_a_slot_freed_on_either_edge_of_the_window_is_in_it():
    for edge in WINDOW:
        assert _judged(_held_until(edge)) == (vanish.IN_WINDOW, edge)
    assert vanish.watch_over(BEFORE, _held_until(EARLIEST), EARLIEST, LATEST)
    assert not vanish.watch_over(BEFORE, _held_until(EARLIEST - 1), EARLIEST, LATEST)


def test_a_slot_freed_before_keepalive_can_abort_is_too_early():
    samples = _held_until(20.0) + [(at, BEFORE) for at in (40.0, 60.0, LATEST)]
    assert _judged(samples) == (vanish.TOO_EARLY, 20.0)


def test_a_slot_freed_after_the_slack_is_too_late():
    assert _judged(_held_until(100.0)) == (vanish.TOO_LATE, 100.0)


def test_a_slot_never_freed_is_its_own_verdict():
    held = [(at, BEFORE + 1) for at in range(0, 121, 10)]
    assert _judged(held) == (vanish.NEVER_FREED, None)


@pytest.mark.parametrize("counting", [[], [BEFORE, BEFORE], [BEFORE + 2], [BEFORE + 1, BEFORE]])
def test_a_client_the_count_never_settled_on_is_inconclusive(counting):
    assert _judged(_held_until(57.0), counting=counting) == (vanish.NEVER_COUNTED, None)
    assert vanish.NEVER_COUNTED in vanish.INCONCLUSIVE


def test_only_the_slot_verdicts_that_blame_keepalive_are_red():
    assert set(vanish.INCONCLUSIVE) == {vanish.NEVER_COUNTED, vanish.NOISE, vanish.UNCLEAN}
    red = {vanish.NEVER_FREED, vanish.TOO_EARLY, vanish.TOO_LATE, vanish.IN_WINDOW}
    assert not red & set(vanish.INCONCLUSIVE)


@pytest.mark.parametrize("released", [
    None, {"ev": vanish.GONE}, {"ev": "expired"}, {"ev": "released", "failed": [["-D"]]}])
def test_a_slot_verdict_after_a_release_that_was_not_clean_is_inconclusive(released):
    assert _judged(_held_until(57.0), released=released) == (vanish.UNCLEAN, 57.0)
    early = _held_until(20.0) + [(LATEST, BEFORE)]
    assert _judged(early, released=released)[0] == vanish.UNCLEAN


@pytest.mark.parametrize("settled", [BEFORE - 1, BEFORE + 1])
def test_a_slot_verdict_needs_the_count_back_at_the_baseline_after_the_release(settled):
    assert _judged(_held_until(57.0), settled=settled) == (vanish.NOISE, 57.0)
    early = _held_until(20.0) + [(LATEST, BEFORE)]
    assert _judged(early, settled=settled)[0] == vanish.NOISE
    held = [(at, BEFORE + 1) for at in range(0, 121, 10)]
    assert _judged(held, settled=settled)[0] == vanish.NOISE


def test_a_co_tenant_that_joins_or_leaves_makes_the_window_noise():
    joined = [(0.0, BEFORE + 1), (10.0, BEFORE + 2), (57.0, BEFORE + 1)]
    assert _judged(joined)[0] == vanish.NOISE
    left_early = _held_until(20.0) + [(40.0, BEFORE), (57.0, BEFORE - 1)]
    assert _judged(left_early)[0] == vanish.NOISE
    came_back = _held_until(20.0) + [(30.0, BEFORE + 1), (57.0, BEFORE)]
    assert _judged(came_back)[0] == vanish.NOISE


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
    bench.table.write_text(TABLE)
    removed, left = vanish.sweep(SWEEP_CONFIG, candidates=[str(bench.fake)])
    assert [(chain, spec[-3]) for _, chain, spec in removed] == [("OUTPUT", TAG), ("INPUT", TAG)]
    assert left == []
    assert [line for line in bench.rules() if vanish.TAG_PREFIX in line] == [
        '-A OUTPUT -m comment --comment "not hil-ws-vanish-1" -j ACCEPT']


def test_a_rule_the_sweep_could_not_delete_is_reported_left(bench):
    bench.table.write_text(TABLE)
    bench.knob("refuse-delete")
    removed, left = vanish.sweep(SWEEP_CONFIG, candidates=[str(bench.fake)])
    assert [(chain, spec[-3]) for _, chain, spec in left] == [("OUTPUT", TAG), ("INPUT", TAG)]
    assert len(removed) == 2


@pytest.mark.parametrize("chain", vanish.CHAINS)
def test_a_listing_that_fails_in_either_chain_stops_the_sweep(bench, chain):
    bench.table.write_text(TABLE)
    bench.knob("fail-list-" + chain)
    with pytest.raises(vanish.VanishUnavailable, match="-S %s" % chain):
        vanish.sweep(SWEEP_CONFIG, candidates=[str(bench.fake)])
    assert bench.calls("-D") == []


def test_the_wb_client_holds_its_four_tuple_silent_then_releases_with_a_reset(bench, controller):
    peer = _vanished(bench, controller)
    try:
        assert vanish.tuple4(peer.connected) == vanish.Tuple4(
            "127.0.0.1", controller.peer[1], "127.0.0.1", controller.port)
        assert peer.connected["iptables"] == str(bench.fake)
        assert all(vanish.tag(controller.peer[1], NONCE) in rule for rule in bench.rules())
        assert [entry["failsafe"] for entry in bench.entries("-I")] == [True, True], \
            "a rule went in before the failsafe that removes it was running"
        assert controller.ended is None, "the client spoke or closed before its release"
        assert peer.release() == vanish.CLEAN_RELEASE
        assert bench.rules() == [] and len(bench.calls("-D")) == 2
        assert wait_until(lambda: controller.ended, SETTLE_S, interval_s=POLL_S) == "reset"
        assert controller.ended_at > max(entry["at"] for entry in bench.entries("-D")), \
            "the reset left while a DROP rule could still swallow it"
        assert wait_until(lambda: not bench.failsafe_alive(), SETTLE_S, interval_s=POLL_S), \
            "the failsafe outlived a clean release"
    finally:
        peer.stop()


def test_a_delete_that_fails_once_is_tried_again(bench, controller):
    peer = _vanished(bench, controller)
    try:
        bench.knob("flaky-delete")
        assert peer.release() == vanish.CLEAN_RELEASE
        assert bench.rules() == [] and len(bench.calls("-D")) == 4
    finally:
        peer.stop()


def test_a_rule_that_stays_keeps_its_failsafe_armed_after_the_client_ends(bench, controller):
    peer = _vanished(bench, controller)
    try:
        bench.knob("refuse-delete")
        released = peer.release()
        assert [argv[3:5] for argv in released["failed"]] == [["-D", "OUTPUT"], ["-D", "INPUT"]]
    finally:
        peer.stop()
    assert len(bench.rules()) == 2
    assert [event["ev"] for event in peer.seen][-2:] == ["leftover", vanish.GONE]
    assert bench.failsafe_alive(), "the failsafe was disarmed while the rules stayed in"


@pytest.mark.parametrize("failsafe", [["hil-no-such-nohup", "sh", "-c", "sleep 1"],
                                      ["sh", "-c", "exit 0"]])
def test_no_rule_goes_in_unless_the_failsafe_reports_itself_armed(bench, controller, monkeypatch,
                                                                  failsafe):
    monkeypatch.setattr(vanish, "failsafe_command", lambda deletes, seconds: failsafe)
    peer = bench.peer(controller)
    try:
        assert peer.start()["ev"] == vanish.CONNECTED
        with pytest.raises(vanish.VanishUnavailable, match="failsafe"):
            peer.vanish()
        assert bench.calls("-I") == [] and bench.rules() == []
    finally:
        peer.stop()


@pytest.mark.parametrize("end", ["eof", "sigterm", "sighup"])
def test_the_rules_go_when_the_channel_closes_or_the_client_is_told_to_stop(bench, controller,
                                                                            end):
    peer = _vanished(bench, controller)
    try:
        if end == "eof":
            peer._proc.stdin.close()
        else:
            peer._proc.send_signal(SIGNALS[end])
        assert wait_until(lambda: not bench.rules(), SETTLE_S, interval_s=POLL_S), bench.rules()
        assert wait_until(lambda: controller.ended, SETTLE_S, interval_s=POLL_S) == "reset"
    finally:
        peer.stop()


def test_the_hold_limit_removes_the_rules_and_resets_on_its_own(bench, controller):
    peer = _vanished(bench, controller, hold_s=HOLD_TEST_S)
    try:
        assert wait_until(lambda: not bench.rules(), HOLD_TEST_S + SETTLE_S,
                          interval_s=POLL_S), bench.rules()
        assert wait_until(lambda: controller.ended, SETTLE_S, interval_s=POLL_S) == "reset"
        assert peer.release() != vanish.CLEAN_RELEASE
    finally:
        peer.stop()
    assert {"ev": "expired"} in peer.seen


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


class World:
    def __init__(self, clients):
        self.clients = clients

    def diagnostics(self):
        return {"websocket": {"clients": self.clients, "inbox_overflow_total": 0}}


class StubPeer:
    def __init__(self, world, script):
        self.world, self.script = world, script
        self.sent_rules, self.seen = False, []

    def start(self):
        self.world.clients += 1 if self.script.get("counted", True) else 0
        return {"ev": vanish.CONNECTED}

    def vanish(self):
        self.sent_rules = True
        if self.script.get("vanish_fails"):
            raise vanish.VanishUnavailable("the WB did not black-hole the connection: None")
        self.world.clients += self.script.get("at_vanish", 0)
        return {"ev": vanish.VANISHED, "connected_to_rules_s": 0.5}

    def release(self):
        self.world.clients += self.script.get("at_release", 0)
        return self.script.get("released", vanish.CLEAN_RELEASE)

    def stop(self):
        pass


class Artifacts:
    def __init__(self):
        self.attached = {}

    def attach_json(self, name, doc):
        self.attached[name] = json.loads(json.dumps(doc))


class Hardware:
    def __init__(self, monkeypatch):
        self.monkeypatch = monkeypatch
        self.world, self.artifacts = World(BEFORE), Artifacts()
        self.sweeps, self.survived, self.window = [], [], (0.0, 1.0)
        monkeypatch.setattr(vanish, "window", lambda gap: self.window)

    def run(self, script, left=()):
        def sweep(cfg):
            self.sweeps.append(cfg.wb_ssh)
            return [], list(left)
        self.monkeypatch.setattr(vanish, "VanishingPeer", lambda cfg: StubPeer(self.world, script))
        self.monkeypatch.setattr(vanish, "sweep", sweep)
        test_ws.test_a_client_gone_from_the_network_is_reclaimed_by_keepalive(
            self.world, STUB_CONFIG, {"survived": lambda: self.survived.append(True)},
            self.artifacts)

    @property
    def report(self):
        return self.artifacts.attached["vanish"]


@pytest.fixture()
def hardware(monkeypatch):
    reached = []
    monkeypatch.setattr(vanish, "ssh_argv", _recording(reached, "ssh_argv"))
    monkeypatch.setattr(vanish.subprocess, "run", _recording(reached, "subprocess.run"))
    monkeypatch.setattr(vanish.subprocess, "Popen", _recording(reached, "Popen"))
    monkeypatch.setattr(test_ws.seriallog, "SerialLog",
                        lambda cfg: types.SimpleNamespace(monitor_alive=False))
    for name in ("ACK_BUDGET_S", "RELEASE_SETTLE_S", "VANISH_POLL_S", "COUNT_POLL_S"):
        monkeypatch.setattr(test_ws, name, QUICK_S)
    yield Hardware(monkeypatch)
    assert reached == []


def test_rules_left_on_the_wb_fail_the_run_even_when_the_vanish_step_gave_up(hardware):
    with pytest.raises(AssertionError, match=vanish.TAG_PREFIX):
        hardware.run({"vanish_fails": True}, left=[LEFT_RULE])
    assert hardware.sweeps == [STUB_CONFIG.wb_ssh]
    assert hardware.report["left"] == [LEFT_RULE]
    assert hardware.survived == [], "the run judged the controller before the WB was clean"


def test_a_vanish_step_that_gave_up_is_swept_reported_and_inconclusive(hardware):
    with pytest.raises(pytest.skip.Exception, match="INCONCLUSIVE"):
        hardware.run({"vanish_fails": True})
    assert hardware.sweeps == [STUB_CONFIG.wb_ssh]
    assert hardware.report["released"] == vanish.CLEAN_RELEASE


def test_a_slot_freed_in_the_window_and_still_free_after_a_clean_release_passes(hardware):
    hardware.run({"at_vanish": -1})
    assert hardware.report["verdict"] == vanish.IN_WINDOW
    assert hardware.report["settled"] == BEFORE and hardware.report["left"] == []
    assert hardware.survived == [True]


def test_a_drop_the_release_shows_was_another_client_leaving_is_inconclusive(hardware):
    with pytest.raises(pytest.skip.Exception, match="came or went"):
        hardware.run({"at_vanish": -1, "at_release": -1})
    assert hardware.report["verdict"] == vanish.NOISE
    assert hardware.report["settled"] == BEFORE - 1


def test_a_drop_followed_by_a_release_that_was_not_clean_is_inconclusive(hardware):
    with pytest.raises(pytest.skip.Exception, match="did not release cleanly"):
        hardware.run({"at_vanish": -1, "released": {"ev": vanish.GONE}})
    assert hardware.report["verdict"] == vanish.UNCLEAN


def test_a_client_the_count_never_showed_is_inconclusive_and_needs_no_sweep(hardware):
    with pytest.raises(pytest.skip.Exception, match="never settled on %d" % (BEFORE + 1)):
        hardware.run({"counted": False})
    assert hardware.report["verdict"] == vanish.NEVER_COUNTED
    assert hardware.sweeps == []


def test_a_slot_held_through_the_window_and_freed_by_the_release_is_red(hardware, monkeypatch):
    monkeypatch.setattr(vanish, "OVERRUN_S", 0.0)
    hardware.window = (0.0, QUICK_S * 4)
    with pytest.raises(AssertionError, match="keepalive did not reclaim it"):
        hardware.run({"at_release": -1})
    assert hardware.report["verdict"] == vanish.NEVER_FREED
    assert hardware.survived == [True]
