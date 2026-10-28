import dataclasses
import json
import os
import sys
import types

import pytest

from hil import foreign
from hil.config import load as load_config
from hil.foreign import (PEER_LOG_STALE_S, ForeignMaster, ForeignMasterSilent, PeerLog,
                         frame_hex, frames_on_wire, interleaving, monitor_verdict, peer_note,
                         sniffed)

TOPIC = "/wb-dali/wb-dali_19_bus_1/bus_monitor"
DTR0_114 = {"bits": 16, "bytes": [0xA3, 0x72]}


def test_frame_hex_is_the_monitors_spelling():
    assert frame_hex(DTR0_114) == "a372"
    assert frame_hex({"bits": 24, "bytes": [0xFF, 0xFE, 0x3D]}) == "fffe3d"


def test_a_refused_batch_is_not_delivered_whatever_else_crossed_the_wire():
    lines = [
        TOPIC + " >>   a372 FF16 DTR0(114) (from lunatone)",
        TOPIC + " <<no response from gateway",
        TOPIC + " <<   0999 FF16 QueryDeviceType(<address (control gear) 4>) (fc: 1)",
        TOPIC + " << fffe3d FF24 QueryApplicationControlEnabled(<broadcast>) (fc: 2)",
    ]
    seen, errors = frames_on_wire(lines, [DTR0_114])
    assert seen == 0
    assert errors == ["no response from gateway"]


def test_a_frame_on_the_wire_counts_and_the_request_line_does_not():
    lines = [
        TOPIC + " >>   a372 FF16 DTR0(114) (from lunatone)",
        TOPIC + " <<   a372 FF16 DTR0(114) (fc: 29786)",
    ]
    assert frames_on_wire(lines, [DTR0_114]) == (1, [])


def test_a_truncated_batch_is_not_delivered():
    dtr1 = {"bits": 16, "bytes": [0xC3, 0x01]}
    activate = {"bits": 16, "bytes": [0x03, 0xE2]}
    lines = [
        TOPIC + " >>   a372 FF16 DTR0(114) (from lunatone)",
        TOPIC + " <<   a372 FF16 DTR0(114) (fc: 1)",
        TOPIC + " >>   c301 FF16 DTR1(1) (from lunatone)",
        TOPIC + " <<   c301 FF16 DTR1(1) (fc: 2)",
        TOPIC + " >>   03e2 FF16 Activate(1) (from lunatone)",
        TOPIC + " <<no response from gateway",
    ]
    seen, errors = frames_on_wire(lines, [DTR0_114, dtr1, activate])
    assert (seen, errors) == (2, ["no response from gateway"])


def test_someone_elses_identical_frame_is_not_ours():
    lines = [
        TOPIC + " >>   a372 FF16 DTR0(114) (from lunatone)",
        TOPIC + " <<no response from gateway",
        TOPIC + " <<   a372 FF16 DTR0(114) (fc: 7)",
    ]
    assert frames_on_wire(lines, [DTR0_114]) == (0, ["no response from gateway"])


QUERY_STATUS_A5 = {"bits": 16, "bytes": [0x0B, 0x90]}
DAPC_A1 = {"bits": 16, "bytes": [0x02, 0x4D]}
STAMPED_REQUEST = TOPIC + " 08:08:25.882 >> 0b90 QueryStatus(A5) (from lunatone) - "
PEER_LINE = ("2026-10-26T07:00:11.531Z I (711362) dali2rust_adapters::dali::transport::esp_idf: "
             "DALI sniff: foreign forward16=0x%s (dapc, short=1, addr=0x02, data=0x4d)\n")
QUICK_S = 0.2
POLL_S = 0.01


def test_a_stamped_request_the_gateway_refused_is_an_error():
    lines = [STAMPED_REQUEST + "no response from gateway"]
    assert frames_on_wire(lines, [QUERY_STATUS_A5]) == (0, ["no response from gateway"])


def test_a_stamped_echo_is_another_masters_frame():
    lines = [STAMPED_REQUEST + "00 0", TOPIC + " 08:08:25.901 << 0b90 QueryStatus(A5) (fc: 4447)"]
    assert monitor_verdict(lines, [QUERY_STATUS_A5]) == (0, [], 1)


def test_a_stamped_request_without_an_echo_is_left_awaiting_its_witness():
    lines = [STAMPED_REQUEST + "00 0",
             TOPIC + " 08:08:26.003 << fffe3d FF24.QueryApplicationControlEnabled (fc: 4204)"]
    assert monitor_verdict(lines, [QUERY_STATUS_A5]) == (0, [], 1)
    assert monitor_verdict([STAMPED_REQUEST + "transmission error"], [QUERY_STATUS_A5]) == (
        0, ["transmission error"], 0)


def test_the_peer_witness_counts_each_frame_once_and_by_width():
    assert sniffed(PEER_LINE % "024d", [DAPC_A1])
    assert not sniffed(PEER_LINE % "024d", [DAPC_A1, DAPC_A1])
    assert sniffed(PEER_LINE % "024d" * 2, [DAPC_A1, DAPC_A1])
    assert not sniffed(PEER_LINE % "024e", [DAPC_A1])
    assert not sniffed((PEER_LINE % "024d").replace("forward16", "forward24"), [DAPC_A1])


def test_the_peer_log_reads_only_what_was_written_after_the_mark(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text(PEER_LINE % "024d")
    mark = PeerLog(log)
    assert not sniffed(mark.since(), [DAPC_A1])
    with open(log, "a") as fh:
        fh.write(PEER_LINE % "024d")
    assert sniffed(mark.since(), [DAPC_A1])
    assert PeerLog(tmp_path / "absent.log").offset is None
    assert PeerLog(None).since() == ""


def test_a_peer_log_nobody_writes_is_no_witness(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text(PEER_LINE % "024d")
    old = log.stat().st_mtime - PEER_LOG_STALE_S - 1
    os.utime(log, (old, old))
    peer = PeerLog(log)
    assert peer.offset is None
    assert peer_note(peer) == "no live peer serial log could witness them"
    assert foreign._peer_log_path(types.SimpleNamespace(has_peer=False)) is None


def test_a_peer_that_sniffs_nothing_is_named_no_witness(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text("")
    peer = PeerLog(log)
    with open(log, "a") as fh:
        fh.write("2026-10-26T08:12:35.418Z I (5193278) gear_sim: answered 0xff\n")
    assert "no sniffed frame at all" in peer_note(peer)
    with open(log, "a") as fh:
        fh.write(PEER_LINE % "024e")
    assert peer_note(peer) == "the peer's sniffer did not log them"


class _Api:
    adapter = 0

    def segment_shorts(self):
        return []


def _master(monkeypatch, tmp_path, verdict, peer_sees):
    master = ForeignMaster(dataclasses.replace(load_config(), lamp_shorts="1",
                                               lamps_read_only=False), api=_Api())
    log = tmp_path / "peer.log"
    log.write_text("")
    sends = []

    def send_once(frames):
        sends.append(frames)
        with open(log, "a") as fh:
            fh.write(PEER_LINE % ("024d" if peer_sees else "024e"))
        return []

    monkeypatch.setattr(foreign, "_peer_log_path", lambda cfg: log)
    monkeypatch.setattr(master, "_wait_quiet", lambda timeout_s: {16: 0, 24: 0})
    monkeypatch.setattr(master, "_arm_monitor", lambda: None)
    monkeypatch.setattr(master, "_monitor_verdict", lambda monitor, frames: verdict)
    monkeypatch.setattr(master, "_witness", lambda before, width: 1)
    monkeypatch.setattr(master, "_send_once", send_once)
    master.WITNESS_TIMEOUT_S, master.POLL_S = QUICK_S, POLL_S
    return master, sends


def test_a_frame_only_the_peer_saw_is_delivered_once(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, (0, [], 1), peer_sees=True)
    master.dapc(1, 0x4D)
    assert len(sends) == 1 and not master.retries


def test_a_frame_no_witness_saw_is_resent_then_reported_silent(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, (0, [], 1), peer_sees=False)
    with pytest.raises(ForeignMasterSilent, match="the peer's sniffer did not log them"):
        master.dapc(1, 0x4D)
    assert len(sends) == ForeignMaster.MAX_SENDS
    assert master.retries["foreign_master_silent"] == ForeignMaster.MAX_SENDS - 1


def test_a_gateway_refusal_outranks_the_peer(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, (0, ["no response from gateway"], 0),
                            peer_sees=True)
    with pytest.raises(ForeignMasterSilent, match="no response from gateway"):
        master.dapc(1, 0x4D)
    assert len(sends) == ForeignMaster.MAX_SENDS


def test_a_frame_the_monitor_never_saw_requested_is_not_handed_to_the_peer(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, (0, [], 0), peer_sees=True)
    with pytest.raises(ForeignMasterSilent):
        master.dapc(1, 0x4D)
    assert len(sends) == ForeignMaster.MAX_SENDS


def test_without_a_monitor_the_controller_counter_decides(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, None, peer_sees=False)
    master.dapc(1, 0x4D)
    assert len(sends) == 1 and not master.retries


def _batches(monkeypatch):
    master = ForeignMaster(dataclasses.replace(load_config(), lamp_shorts="1",
                                               lamps_read_only=False))
    batches = []
    monkeypatch.setattr(master, "send_frames", lambda frames: batches.append(
        [(f["addr"], f["data"]) for f in frames]) or [])
    return master, batches


def test_a_dt8_command_goes_out_in_one_batch_with_its_dtrs_and_its_enable(monkeypatch):
    master, batches = _batches(monkeypatch)
    master.set_cct(1, 2700)
    assert batches == [[(0xA3, 0x72), (0xC3, 0x01), (0xC1, 8), (0x03, 0xE7)],
                       [(0xC1, 8), (0x03, 0xE2)]]
    batches.clear()
    master.set_rgb(1, 10, 20, 30)
    assert [batch[-2:] for batch in batches] == [[(0xC1, 8), (0x03, op)]
                                                 for op in (0xED, 0xEB, 0xEC, 0xE2)]
    assert batches[1][:3] == [(0xA3, 10), (0xC3, 20), (0xC5, 30)]


STAMP = TOPIC + " 09:09:20.%03d "


def test_another_forward_frame_inside_a_unit_breaks_it():
    lines = [STAMP % 1 + ">> a372 DTR0(114) (from lunatone)",
             STAMP % 2 + ">> 0390 QueryStatus(A1) - 00 0",
             STAMP % 3 + "<< fffe3d FF24.QueryApplicationControlEnabled (fc: 1)",
             STAMP % 4 + "<< 00ff BF8 (fc: 2)",
             STAMP % 5 + ">> c108 EnableDeviceType(8) (from lunatone)",
             STAMP % 6 + "<< 0b90 QueryStatus(A5) (fc: 3)"]
    assert interleaving(lines) == ["interleaved by 0390"]
    assert interleaving(lines[:1] + lines[2:]) == []
    assert monitor_verdict(lines, [DTR0_114, {"bits": 16, "bytes": [0xC1, 8]}])[1] == [
        "interleaved by 0390"]


class _ScriptedSocket:
    def __init__(self, incoming):
        self.incoming, self.consumed, self.sent_after, self.sent = list(incoming), 0, [], []

    async def recv(self):
        self.consumed += 1
        return json.dumps(self.incoming.pop(0))

    async def send(self, text):
        self.sent_after.append(self.consumed)
        self.sent.append(json.loads(text))

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        return False


def test_the_remote_client_sends_a_frame_only_after_the_previous_one_is_settled(monkeypatch):
    monitor = {"type": "daliMonitor", "data": {"bits": 24, "data": [255, 254, 61]}}
    socket = _ScriptedSocket([{"type": "info"}, monitor, {"type": "daliFrame"}, monitor,
                              {"type": "daliFrame"}, {"type": "daliAnswer"}])
    monkeypatch.setitem(sys.modules, "websockets",
                        types.SimpleNamespace(connect=lambda url: socket))
    frames = [{"bits": 16, "bytes": [0xC1, 8]},
              {"bits": 16, "bytes": [0x03, 0x90], "wait_answer": True}]
    monkeypatch.setattr(sys, "argv", ["remote", json.dumps(frames)])
    exec(compile(foreign.remote_client_script(0), "remote", "exec"), {"__name__": "__main__"})
    assert socket.sent_after == [1, 3]
    assert socket.incoming == []


def test_a_send_twice_frame_leaves_as_two_frames_before_either_is_settled(monkeypatch):
    socket = _ScriptedSocket([{"type": "info"}, {"type": "daliFrame"}, {"type": "daliFrame"}])
    monkeypatch.setitem(sys.modules, "websockets",
                        types.SimpleNamespace(connect=lambda url: socket))
    frames = [{"bits": 16, "bytes": [0x03, 0xF3], "send_twice": True}]
    monkeypatch.setattr(sys, "argv", ["remote", json.dumps(frames)])
    exec(compile(foreign.remote_client_script(0), "remote", "exec"), {"__name__": "__main__"})
    assert socket.sent_after == [1, 1]
    assert [m["data"]["daliData"] for m in socket.sent] == [[0x03, 0xF3], [0x03, 0xF3]]
    assert not any(m["data"]["mode"]["sendTwice"] for m in socket.sent)
    assert socket.incoming == []


def test_the_remote_client_stops_waiting_once_the_emulator_refused_a_frame(monkeypatch):
    refused = {"type": "daliFrame", "data": {"line": 0, "result": 6}}
    socket = _ScriptedSocket([{"type": "info"}, refused, {"type": "daliFrame"}])
    monkeypatch.setitem(sys.modules, "websockets",
                        types.SimpleNamespace(connect=lambda url: socket))
    frames = [{"bits": 16, "bytes": [0x03, 0x90], "wait_answer": True},
              {"bits": 16, "bytes": [0x02, 0x4D]}]
    monkeypatch.setattr(sys, "argv", ["remote", json.dumps(frames)])
    exec(compile(foreign.remote_client_script(0), "remote", "exec"), {"__name__": "__main__"})
    assert socket.sent_after == [1, 2]


def test_a_split_batch_is_counted_apart_from_a_silent_one(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, (0, ["interleaved by 0390"], 0),
                            peer_sees=True)
    with pytest.raises(ForeignMasterSilent, match="the bench bus is busy"):
        master.dapc(1, 0x4D)
    assert master.retries == {"foreign_unit_interleaved": ForeignMaster.MAX_SENDS - 1}


def test_a_single_frame_is_never_reported_split():
    lines = [STAMP % 1 + ">> a372 DTR0(114) (from lunatone)",
             STAMP % 2 + ">> 0390 QueryStatus(A1) - 00 0",
             STAMP % 3 + ">> a372 DTR0(114) (from lunatone)"]
    assert monitor_verdict(lines, [DTR0_114])[1] == []
