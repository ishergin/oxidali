import socket
import struct
import threading
import time

import pytest

from hil import provoke, restload, seriallog, tripwire, wsclient
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


@pytest.mark.hil_id("HIL-WS-08")
@pytest.mark.smoke
@pytest.mark.serial
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


SOAK_CLIENTS = 2
SOAK_QUIET_S = 300
SOAK_PROVOKED_S = 300
SOAK_PATCH_EVERY_S = 3.0
SOAK_POLL_S = 2.0
DIAGNOSTICS_PERIOD_S = 2.0
SNIFFER_BATCH = "SnifferBatch"
DIAGNOSTICS_SNAPSHOT = "DiagnosticsSnapshot"
WS_CLIENT_TASK = "ws-client"
WS_CLIENT_LIMIT = 4
MIN_DIAGNOSTICS = (SOAK_QUIET_S + SOAK_PROVOKED_S) / DIAGNOSTICS_PERIOD_S / 2
HEARTBEATS_NEEDED = 9
LATE_REPORT_INTERVAL_MS = 5000
LATE_REPORT_SLACK_MS = 1000
LATE_REPORT_WAIT_S = 30.0
LATE_REPORT_POLL_S = 1.0
PHY_DRIVER_LEVEL = 3
BELOW_THE_CRITICAL_SECTION = (
    "INCONCLUSIVE on this image: %s. ws-client can delay the PHY interrupt only at level "
    "%d, which a critical section masks; above it the late-entries line stays clean "
    "whatever the send path does, so the check needs an image built with "
    "DALI2RUST_PHY_ISR_LEVEL=%d")


def require_the_driver_level(serial_log):
    level = serial_log.boot_phy_level()
    if level == PHY_DRIVER_LEVEL:
        return
    found = ("the serial log holds no `DALI PHY interrupt` line of the running boot"
             if level is None else "the running boot put the PHY interrupt at level %d" % level)
    pytest.skip(BELOW_THE_CRITICAL_SECTION % (found, PHY_DRIVER_LEVEL, PHY_DRIVER_LEVEL))


def _soak(load, seconds, rewrite=None):
    end = time.monotonic() + seconds
    while time.monotonic() < end and not (load.errors or load.closes):
        if rewrite is not None:
            rewrite()
        time.sleep(SOAK_PATCH_EVERY_S if rewrite is not None else SOAK_POLL_S)


def _idle_subscribers(kinds):
    return [index for index, seen in enumerate(kinds)
            if not seen.get(SNIFFER_BATCH) or seen.get(DIAGNOSTICS_SNAPSHOT, 0) < MIN_DIAGNOSTICS]


def _await_next_late_report(api, serial):
    closed_ms = seriallog.newest_ms(serial.lines())
    if closed_ms is None:
        return False
    past = closed_ms + LATE_REPORT_INTERVAL_MS + LATE_REPORT_SLACK_MS
    return bool(wait_until(lambda: api.health() and seriallog.logged_past(serial.lines(), past),
                           LATE_REPORT_WAIT_S, interval_s=LATE_REPORT_POLL_S))


def _soaked(api, hil_config, serial_log, rewrite):
    with serial_log.window() as serial:
        load = wsclient.Subscribers(hil_config.base, SOAK_CLIENTS)
        try:
            _soak(load, SOAK_QUIET_S)
            _soak(load, SOAK_PROVOKED_S, rewrite)
        finally:
            load.close()
        reported = _await_next_late_report(api, serial)
        return load, reported, serial.lines()


@pytest.mark.hil_id("HIL-WS-07")
@pytest.mark.slow
@pytest.mark.serial
def test_two_sniffer_subscribers_keep_ws_client_out_of_late_isr_entries(
        api, hil_config, serial_log, ws_baseline, test_artifacts):
    require_the_driver_level(serial_log)
    connected = _clients(api)
    if connected + SOAK_CLIENTS > WS_CLIENT_LIMIT:
        pytest.skip("%d WebSocket clients are connected already (a web UI tab counts), and "
                    "the controller serves %d: two more would be refused"
                    % (connected, WS_CLIENT_LIMIT))
    short = provoke.first_registered(api, hil_config.lamp_short_set())
    rewrite = provoke.NameRewrite(api, short) if short is not None else None
    losses = tripwire.log_losses(api.stats())
    try:
        load, reported, lines = _soaked(api, hil_config, serial_log, rewrite)
    finally:
        restored = rewrite.restore() if rewrite is not None else True
    lost = tripwire.lost_lines(losses, tripwire.log_losses(api.stats()))
    late, heartbeats = seriallog.late_entries(lines), seriallog.heartbeat_run(lines)
    kinds = [dict(seen) for seen in load.kinds]
    test_artifacts.attach_json("soak", {
        "frames_by_subscriber": kinds, "load_errors": load.errors, "closes": load.closes,
        "late_entries": late.tasks, "late_unparsed": late.unparsed,
        "late_truncated": late.truncated, "heartbeat_run": heartbeats,
        "next_report_seen": reported, "provoked_on": short,
        "name_writes": rewrite.writes if rewrite else 0, "lost_log_lines": lost})

    offending = late.mentioning(WS_CLIENT_TASK)
    assert not offending, (
        "ws-client delayed the PHY interrupt while two subscribers were served: %s"
        % offending)
    assert restored, "SA%s's name is not back to %r: restore it by hand" % (
        short, rewrite.original)
    assert not load.refused(), (
        "INCONCLUSIVE, not a product failure: the controller refused subscriber(s) %s for "
        "capacity (1013) — another client took a slot during the soak" % load.refused())
    assert not load.errors and not load.closes, (
        "the WebSocket load did not hold: %s %s" % (load.errors, load.closes))
    assert serial_log.monitor_alive and heartbeats >= HEARTBEATS_NEEDED, (
        "INCONCLUSIVE, not a product failure: the serial log holds a run of %d consecutive "
        "heartbeats, %d needed, and the monitor is %s — the ten minutes were not all read"
        % (heartbeats, HEARTBEATS_NEEDED, "alive" if serial_log.monitor_alive else "down"))
    assert reported, (
        "INCONCLUSIVE, not a product failure: the log never reached the report that follows "
        "the load, so the last late entries were not read")
    assert not lost, (
        "INCONCLUSIVE, not a product failure: the DUT dropped log lines %s, and a "
        "late-entries line may be among them" % lost)
    assert not late.unparsed and not late.truncated, (
        "INCONCLUSIVE, not a product failure: %d late-entry item(s) did not parse and %d "
        "line(s) reached the firmware's %d-byte report limit"
        % (late.unparsed, late.truncated, seriallog.LATE_REPORT_BYTES))
    assert not _idle_subscribers(kinds), (
        "INCONCLUSIVE, not a product failure: subscriber(s) %s got no sniffer batch or "
        "fewer than %d diagnostics snapshots, so the send path was not exercised: %s"
        % (_idle_subscribers(kinds), MIN_DIAGNOSTICS, kinds))
    ws_baseline["survived"]()

LOAD_SECONDS = 30
LOAD_THREADS = 3
READ_BURST = 2
BURST_JOIN_S = 25.0
FRAMES_FLOOR = 100
REST_CALLS_FLOOR = 400
LOUD = "debug"
LOG_CHANNEL_ONLY = ["logs"]
ATTR_READ_GROUPS = ["runtime_status", "common_102", "dt8_color", "groups", "scenes"]
ACCEPTED_READ = (200, 202)
WAITED_OUT = "waited out its queued send"
SOCKETS_FULL = "sockets open"
DROP_NOTICE = "DropNotice"


def _read_the_gear(api, short, tally, lock):
    try:
        status, body = api.raw_request(
            "POST", "adapters/%d/physical-devices/%d/attribute-reads" % (api.adapter, short),
            {"attribute_groups": ATTR_READ_GROUPS, "memory_banks": "none"})
    except Exception as exc:
        with lock:
            tally["reads_refused"].append((short, type(exc).__name__))
        return
    with lock:
        if status in ACCEPTED_READ:
            tally["reads_accepted"] += 1
        else:
            tally["reads_refused"].append((short, status, str(body)[:80]))


def _burst(api, shorts, tally, lock):
    readers = [threading.Thread(target=_read_the_gear, args=(api, short, tally, lock))
               for short in shorts]
    for reader in readers:
        reader.start()
    for reader in readers:
        reader.join(timeout=BURST_JOIN_S)


def _under_load(api, hil_config, shorts, load, tally):
    lock = threading.Lock()
    hammer = restload.BenchLanLoad(hil_config, LOAD_SECONDS, LOAD_THREADS)
    try:
        end = time.monotonic() + LOAD_SECONDS
        while time.monotonic() < end and not (load.errors or load.closes):
            for first in range(0, len(shorts), READ_BURST):
                if time.monotonic() >= end:
                    break
                _burst(api, shorts[first:first + READ_BURST], tally, lock)
        return hammer.report()
    finally:
        hammer.stop()


@pytest.mark.hil_id("HIL-WS-09")
@pytest.mark.serial
def test_a_subscriber_on_every_channel_loses_nothing_under_rest_load(
        api, hil_config, serial_log, ws_baseline, test_artifacts):
    shorts = api.addrs()
    if not shorts:
        pytest.skip("the filtered inventory holds no gear whose attributes a read can stream")
    before = api.diagnostics()["websocket"]
    tally = {"reads_accepted": 0, "reads_refused": []}
    with serial_log.window() as serial:
        ws = wsclient.connect(hil_config.base)
        hello = ws.wait_for_op("hello", deadline_s=ACK_BUDGET_S)
        assert hello is not None, "no hello frame after the upgrade"
        channels = hello.get("channels") or []
        assert channels, hello
        load = wsclient.Subscribers(hil_config.base, 1, channels=channels,
                                    opener=lambda _base: ws)
        ws.subscribe(LOG_CHANNEL_ONLY, log_level=LOUD)
        try:
            rest = _under_load(api, hil_config, shorts, load, tally)
        except restload.RestLoadUnavailable as exc:
            pytest.skip("no REST load from the bench LAN (%s): a client a wide-area hop away "
                        "cannot hold the one httpd task busy, so a clean counter here would "
                        "prove nothing" % exc)
        finally:
            load.close()
        lines = serial.lines()
    after = api.diagnostics()["websocket"]
    kinds = dict(load.kinds[0])
    frames = sum(kinds.values())
    grew = {key: after[key] - before[key] for key in before if isinstance(before[key], int)}
    waited_out = [line for line in lines if WAITED_OUT in line]
    test_artifacts.attach_json("load", {
        "gear_read": shorts, "seconds": LOAD_SECONDS, "load_threads": LOAD_THREADS,
        "rest_from_the_lan": rest, "tally": tally, "frames_by_kind": kinds, "grew": grew,
        "load_errors": load.errors, "closes": load.closes, "websocket_before": before,
        "websocket_after": after, "clients_before": ws_baseline["clients"],
        "waited_out": waited_out,
        "sockets_full": [line for line in lines if SOCKETS_FULL in line]})


    assert tally["reads_accepted"] >= len(shorts), (
        "the event source never ran: %d attribute reads of %d gear were accepted, refused %r"
        % (tally["reads_accepted"], len(shorts), tally["reads_refused"][:5]))
    assert rest.get("served", 0) >= REST_CALLS_FLOOR, (
        "the load was too thin to occupy the httpd task: %r in %d s" % (rest, LOAD_SECONDS))
    assert frames >= FRAMES_FLOOR and grew["events_sent_total"] >= FRAMES_FLOOR, (
        "nothing was streamed, so a clean counter proves nothing: %d frames reached the "
        "client, the hub sent %d" % (frames, grew["events_sent_total"]))
    assert grew["events_coalesced_total"] > 0, (
        "no frame ever waited in a client queue (nothing coalesced onto one), so the load "
        "never pressed the send path and a clean drop counter proves nothing: %r" % grew)

    pool_full = [line for line in lines if SOCKETS_FULL in line]
    if (load.errors or load.closes) and pool_full:
        pytest.skip("INCONCLUSIVE: the httpd socket pool ran out %d time(s) and the purge "
                    "closes the least recently used socket, so this subscriber's close says "
                    "nothing about the send path — thin the load or the other clients: %r"
                    % (len(pool_full), load.errors or load.closes))
    assert not load.errors and not load.closes, (
        "the subscriber was dropped during the load: %r %r" % (load.errors, load.closes))
    assert not kinds.get(DROP_NOTICE), (
        "the client was told events were dropped: %r" % kinds)
    assert grew["events_dropped_total"] == 0, (
        "websocket.events_dropped_total grew by %d under the load (hub-wide: %d client(s) "
        "were connected before this one)"
        % (grew["events_dropped_total"], ws_baseline["clients"]))
    assert grew["upgrades_rejected_total"] == 0, (
        "websocket.upgrades_rejected_total grew by %d: a connection was refused, which an "
        "empty sender-slot pool looks like" % grew["upgrades_rejected_total"])
    assert not waited_out, (
        "the sender waited out its queued send, so ESP-IDF lost a work item: %r" % waited_out)

    ws_baseline["survived"]()
