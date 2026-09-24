import errno
import socket

import requests

import hil.api
from hil.config import load as load_config


class _FakeResponse:
    status_code = 200
    content = b"{}"

    @staticmethod
    def json():
        return {}


class _FakeSession:
    def __init__(self):
        self.closes = 0
        self.requests = 0

    def request(self, method, url, json=None, timeout=None):
        self.requests += 1
        return _FakeResponse()

    def close(self):
        self.closes += 1


def _client_with_fake_session():
    client = hil.api.Client(load_config())
    client.http = _FakeSession()
    return client


def test_a_reboot_seen_by_one_client_drops_every_clients_pool():
    a = _client_with_fake_session()
    b = _client_with_fake_session()
    a._http("GET", "health")
    b._http("GET", "health")
    assert (a.http.closes, b.http.closes) == (0, 0), "no reboot, no reason to drop a pool"

    with a.expect_reboot():
        a._http("GET", "health")
    a._http("GET", "health")
    b._http("GET", "health")
    b._http("GET", "health")
    assert (a.http.closes, b.http.closes) == (1, 1)


def test_a_transport_retry_names_the_socket_level_cause():
    reset = ConnectionResetError(errno.ECONNRESET, "Connection reset by peer")
    exc = requests.exceptions.ConnectionError("wrapped")
    exc.__cause__ = reset
    described = hil.api._cause_of(exc)
    assert described.startswith("ConnectionError"), described
    assert "ECONNRESET" in described, described


def test_a_refused_connect_is_named_differently_from_a_reset():
    refused = socket.error(errno.ECONNREFUSED, "Connection refused")
    exc = requests.exceptions.ConnectionError("wrapped")
    exc.__cause__ = refused
    described = hil.api._cause_of(exc)
    assert "ECONNREFUSED" in described, described
    assert "ECONNRESET" not in described


def test_a_cause_chain_with_no_inner_error_still_names_the_class():
    assert hil.api._cause_of(requests.exceptions.ReadTimeout("slow")) == "ReadTimeout"


def test_every_diagnostic_door_books_the_lamps_it_can_move():
    client = hil.api.Client(load_config())
    client._note_diagnostic_write("POST", "dali/raw", {"frame": (5 << 9) | 120})
    client._note_diagnostic_write("POST", "dali/level", {"wire_address": 3 << 1, "level": 9})
    client._note_diagnostic_write("POST", "dali/command",
                                  {"wire_address": (7 << 1) | 1, "command": 0x10})
    client._note_diagnostic_write("POST", "dali/command",
                                  {"wire_address": (8 << 1) | 1, "command": 0xA0})
    client._note_diagnostic_write("POST", "dali/raw", {"frame": 0xA372})
    assert client.raw_touched == {5, 3, 7}
    client._note_diagnostic_write("POST", "dali/raw", {"frame": 0xFF00})
    assert client.TOUCHED_ALL in client.raw_touched
