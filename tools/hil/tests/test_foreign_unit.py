import dataclasses

import pytest

from hil import foreign
from hil.config import load as load_config
from hil.foreign import (ForeignMaster, ForeignMasterSilent, PeerLog, frame_hex,
                         frames_on_wire, sniffed)

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


def test_a_stamped_echo_counts_as_on_the_wire():
    lines = [STAMPED_REQUEST + "00 0", TOPIC + " 08:08:25.901 << 0b90 QueryStatus(A5) (fc: 4447)"]
    assert frames_on_wire(lines, [QUERY_STATUS_A5]) == (1, [])


def test_a_stamped_request_without_an_echo_is_neither_seen_nor_refused():
    lines = [STAMPED_REQUEST + "00 0",
             TOPIC + " 08:08:26.003 << fffe3d FF24.QueryApplicationControlEnabled (fc: 4204)"]
    assert frames_on_wire(lines, [QUERY_STATUS_A5]) == (0, [])


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
        if peer_sees:
            with open(log, "a") as fh:
                fh.write(PEER_LINE % "024d")
        return []

    monkeypatch.setattr(foreign, "_peer_log_path", lambda cfg: log)
    monkeypatch.setattr(master, "_wait_quiet", lambda timeout_s: {16: 0, 24: 0})
    monkeypatch.setattr(master, "_arm_monitor", lambda: None)
    monkeypatch.setattr(master, "_monitor_verdict", lambda monitor, frames: verdict)
    monkeypatch.setattr(master, "_send_once", send_once)
    master.WITNESS_TIMEOUT_S, master.POLL_S = QUICK_S, POLL_S
    return master, sends


def test_a_frame_only_the_peer_saw_is_delivered_once(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, (0, []), peer_sees=True)
    master.dapc(1, 0x4D)
    assert len(sends) == 1 and not master.retries


def test_a_frame_no_witness_saw_is_resent_then_reported_silent(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, (0, []), peer_sees=False)
    with pytest.raises(ForeignMasterSilent, match="the peer's sniffer did not log them"):
        master.dapc(1, 0x4D)
    assert len(sends) == ForeignMaster.MAX_SENDS
    assert master.retries["foreign_master_silent"] == ForeignMaster.MAX_SENDS - 1


def test_a_gateway_refusal_outranks_the_peer(monkeypatch, tmp_path):
    master, sends = _master(monkeypatch, tmp_path, (0, ["no response from gateway"]),
                            peer_sees=True)
    with pytest.raises(ForeignMasterSilent, match="no response from gateway"):
        master.dapc(1, 0x4D)
    assert len(sends) == ForeignMaster.MAX_SENDS
