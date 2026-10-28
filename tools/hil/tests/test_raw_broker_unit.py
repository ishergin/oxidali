import dataclasses
import socket

import pytest

from hil import raw_broker
from hil.config import load as load_config

FIELD = 2


def _local(cfg, remote):
    return ["sh", "-c", remote]


def _free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def _field(text):
    return len(text).to_bytes(FIELD, "big") + text


def _packet(kind, body):
    return bytes([kind, len(body)]) + body


def _recv(sock, size):
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            break
        data += chunk
    return data


@pytest.fixture()
def broker(monkeypatch):
    monkeypatch.setattr(raw_broker, "ssh_argv", _local)
    cfg = dataclasses.replace(load_config(), wb_ssh="root@127.0.0.1",
                              mqtt_broker_host="127.0.0.1")
    started = raw_broker.RawBroker(cfg, port=_free_port()).start()
    try:
        yield started
    finally:
        started.stop()


@pytest.fixture()
def client(broker):
    sock = socket.create_connection((broker.host, broker.port), timeout=5)
    sock.sendall(_packet(0x10, _field(b"MQTT") + b"\x04\x02\x00\x3c" + _field(b"bridge")))
    assert _recv(sock, 4) == b"\x20\x02\x00\x00"
    try:
        yield sock
    finally:
        sock.close()


def test_the_broker_answers_what_the_bridge_sends(broker, client):
    client.sendall(_packet(0x82, b"\x00\x07" + _field(b"hiltest/+/vl/+/set") + b"\x01"))
    assert _recv(client, 5) == b"\x90\x03\x00\x07\x01"
    client.sendall(_packet(0x32, _field(b"hiltest/a0/vl/1/state") + b"\x00\x09" + b"{}"))
    assert _recv(client, 4) == b"\x40\x02\x00\x09"
    client.sendall(b"\xc0\x00")
    assert _recv(client, 2) == b"\xd0\x00"

    status = broker.status()
    assert status["connects"] == 1 and status["connected"] and status["client_id"] == "bridge"
    assert status["filters"] == ["hiltest/+/vl/+/set"]
    assert status["publishes"] == 1 and status["topics_tail"] == ["hiltest/a0/vl/1/state"]


def test_a_topic_that_is_not_utf8_reaches_the_client_byte_for_byte(broker, client):
    topic = b"hiltest/\xc3\x28/vl/1/set"
    broker.publish(topic, b'{"state":"OFF"}')
    body = _field(topic) + b'{"state":"OFF"}'
    assert _recv(client, 2 + len(body)) == bytes([0x30, len(body)]) + body


def test_a_drop_closes_the_client_and_a_redial_is_counted(broker, client):
    assert broker.drop()["had_client"]
    assert _recv(client, 1) == b""
    assert broker.wait_event(lambda e: e.get("ev") == "closed", 5.0), broker.events
    again = socket.create_connection((broker.host, broker.port), timeout=5)
    try:
        again.sendall(_packet(0x10, _field(b"MQTT") + b"\x04\x02\x00\x3c" + _field(b"bridge")))
        assert _recv(again, 4) == b"\x20\x02\x00\x00"
        assert broker.wait_event(lambda e: e.get("ev") == "connect" and e["n"] == 2, 5.0)
    finally:
        again.close()


def test_an_unsubscribe_takes_its_filter_away(broker, client):
    client.sendall(_packet(0x82, b"\x00\x07" + _field(b"a/+") + b"\x01"
                           + _field(b"b/+") + b"\x01"))
    assert _recv(client, 6) == b"\x90\x04\x00\x07\x01\x01"
    client.sendall(_packet(0xA2, b"\x00\x08" + _field(b"a/+")))
    assert _recv(client, 4) == b"\xb0\x02\x00\x08"
    assert broker.status()["filters"] == ["b/+"]


def test_a_qos2_release_is_completed(broker, client):
    client.sendall(_packet(0x62, b"\x00\x05"))
    assert _recv(client, 4) == b"\x70\x02\x00\x05"
