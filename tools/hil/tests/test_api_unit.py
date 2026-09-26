import errno
import socket

import pytest
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


SOURCE = "rule dusk when time 18:00 then off(VL1)"
BASE = 7
OPERATION = "cfg-rules-1"
LOST = requests.exceptions.ConnectionError("connection reset by peer")
ACCEPTED = (202, {"operation_id": OPERATION, "status": "accepted"})
COMMITTED = (200, {"operation_id": OPERATION, "status": "succeeded"})
PUT = ("PUT", "rules")


class _Answer:
    content = b"{}"

    def __init__(self, status, payload):
        self.status_code, self._payload = status, payload

    def json(self):
        return self._payload


class _ScriptedSession:
    def __init__(self, script):
        self.script, self.calls = script, []

    def request(self, method, url, json=None, timeout=None):
        key = (method, url.split("/api/v1/", 1)[1])
        self.calls.append(key)
        queue = self.script[key]
        answer = queue.pop(0) if len(queue) > 1 else queue[0]
        if isinstance(answer, Exception):
            raise answer
        return _Answer(*answer)

    def close(self):
        pass


def _scripted(monkeypatch, script):
    client = hil.api.Client(load_config())
    client.http = _ScriptedSession(script)
    monkeypatch.setattr(hil.api.time, "sleep", lambda seconds: None)
    monkeypatch.setattr(hil.api, "RULES_RECONCILE_S", 0)
    return client


def _document(revision, source):
    return [(200, {"revision": revision, "source": source})]


def test_a_rules_write_that_answers_commits_through_its_operation(monkeypatch):
    client = _scripted(monkeypatch, {PUT: [ACCEPTED],
                                     ("GET", "operations/" + OPERATION): [COMMITTED]})
    assert client.rules_replace(SOURCE, BASE)["status"] == "succeeded"
    assert not client.retries


def test_a_rules_write_whose_answer_was_lost_is_read_back_not_repeated(monkeypatch):
    client = _scripted(monkeypatch, {PUT: [LOST], ("GET", "rules"): _document(BASE + 1, SOURCE)})
    view = client.rules_replace(SOURCE, BASE)
    assert view["status"] == "succeeded" and view["reconciled"] == hil.api.RULES_LANDED
    assert client.http.calls.count(PUT) == 1, "a repeat would meet its own revision: a false 409"
    assert client.retries == {"rules_put_reconciled": 1}
    assert "base_revision=%d" % BASE in client.retry_events[0]


def test_a_lost_rules_write_another_writer_overtook_is_reported_as_a_conflict(monkeypatch):
    client = _scripted(monkeypatch, {PUT: [LOST],
                                     ("GET", "rules"): _document(BASE + 1, "someone else's")})
    view = client.rules_replace(SOURCE, BASE)
    assert view["status"] == "failed" and view["reconciled"] == hil.api.RULES_OVERTAKEN
    assert view["error"]["message"] == "rule_set_conflict"
    assert client.http.calls.count(PUT) == 1


def test_a_lost_rules_write_the_document_never_took_is_sent_once_more(monkeypatch):
    client = _scripted(monkeypatch, {PUT: [LOST, ACCEPTED],
                                     ("GET", "rules"): _document(BASE, "the old one"),
                                     ("GET", "operations/" + OPERATION): [COMMITTED]})
    assert client.rules_replace(SOURCE, BASE)["status"] == "succeeded"
    assert client.http.calls.count(PUT) == 2
    assert client.retries == {"rules_put_reconciled": 1}


def test_a_504_on_the_rules_write_is_read_back_not_retried(monkeypatch):
    client = _scripted(monkeypatch, {PUT: [(504, {"error": "confirmation_timeout"})],
                                     ("GET", "rules"): _document(BASE + 1, SOURCE)})
    assert client.rules_replace(SOURCE, BASE)["status"] == "succeeded"
    assert client.http.calls.count(PUT) == 1
    assert client.retries == {"rules_put_reconciled": 1}


def test_a_conflict_the_controller_answers_is_raised_not_reconciled(monkeypatch):
    client = _scripted(monkeypatch, {PUT: [(409, {"error": "rule_set_conflict"})]})
    with pytest.raises(hil.api.ApiError) as refused:
        client.rules_replace(SOURCE, BASE)
    assert refused.value.status == 409
    assert client.http.calls == [PUT] and not client.retries
