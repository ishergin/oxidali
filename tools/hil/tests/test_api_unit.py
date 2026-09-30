import dataclasses
import errno
import socket

import pytest
import requests

import hil.api
import test_target_state
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
    client = hil.api.Client(dataclasses.replace(load_config(), lamps_read_only=False))
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


def test_rule_toggles_are_read_from_the_compiled_projection():
    projection = {"rules": {"rules": [{"name": "a", "enabled": False}, {"name": "b"}]}}
    assert hil.api.rule_toggles_of(projection) == {"a": False, "b": True}
    assert hil.api.rule_toggles_of({"rules": None, "diagnostic": "x"}) == {}


def test_a_toggle_is_read_and_written_by_the_rule_name(monkeypatch):
    quoted = "rules/%D0%BA%D0%BD%D0%BE%D0%BF%D0%BA%D0%B0%203"
    client = _scripted(monkeypatch, {
        ("GET", "rules?format=json"): [(200, {"rules": {"rules": [
            {"name": "кнопка 3", "enabled": True}]}})],
        ("PATCH", quoted): [(200, {"name": "кнопка 3", "enabled": False})]})
    assert client.rules_toggles() == {"кнопка 3": True}
    assert client.rule_enable("кнопка 3", False)["enabled"] is False
    assert client.http.calls[-1] == ("PATCH", quoted)


def test_kelvin_turns_into_mirek_the_way_the_controller_rounds():
    assert [hil.api.kelvin_to_mirek(k) for k in (2700, 3000, 3500, 5000, 6000)] == [
        370, 333, 286, 200, 167]


def test_the_actual_level_is_the_answer_and_silence_or_a_violation_is_none(monkeypatch):
    client = _scripted(monkeypatch, {("POST", "dali/command"): [
        (200, {"success": True, "backward_frame": 150}),
        (200, {"success": False, "backward_frame": 0}),
        (200, {"success": True, "backward_frame": 0, "backward_violation": True})]})
    assert client.actual_levels([16, 17, 18]) == {16: 150, 17: None, 18: None}


def test_the_held_colour_temperature_comes_from_a_fresh_read(monkeypatch):
    client = _scripted(monkeypatch, {
        ("POST", "adapters/0/physical-devices/20/attribute-reads"): [ACCEPTED],
        ("GET", "operations/" + OPERATION): [COMMITTED],
        ("GET", "adapters/0/physical-devices/20/attributes?sections=dt8_color"): [
            (200, {"attributes": {"dt8_color": {"color_value_2": {"value": 333}}}})]})
    assert client.held_tc_mirek(20) == 333
    assert client.http.calls[0] == ("POST", "adapters/0/physical-devices/20/attribute-reads")



def test_fade_running_is_bit_4_of_a_clean_status_answer(monkeypatch):
    client = _scripted(monkeypatch, {("POST", "dali/command"): [
        (200, {"success": True, "backward_frame": 0x14}),
        (200, {"success": True, "backward_frame": 0x04}),
        (200, {"success": True, "backward_frame": 0xFF, "backward_violation": True}),
        (200, {"success": False, "backward_frame": 0})]})
    assert [test_target_state._fade_running(client, 20) for _ in range(4)] == [
        True, False, None, None]


def test_a_timed_series_reports_its_widest_gap_the_status_before_the_last_and_retries(
        monkeypatch):
    sent = []

    class _Fixture:
        retries = {}

        def cmd(self, short, opcode):
            sent.append(("status", short))
            return {"success": True, "backward_frame": 0x10 if len(sent) < 7 else 0x00}

        def ts(self, short, setpoint):
            sent.append(setpoint["color_temperature_kelvin"])

        def actual_level(self, short):
            return 120

        def held_tc_mirek(self, short):
            return 333

    monkeypatch.setattr(test_target_state.time, "sleep", lambda seconds: None)
    report = test_target_state._timed_series(_Fixture(), 20, 0.3)
    assert sent[:7] == [2700, 3500, 4200, 5000, 6000, ("status", 20), 3000]
    assert report["fade_running_before_last"] is True and report["settled"]
    assert report["retried"] == 0 and report["held_mirek"] == 333
    assert 0 <= report["widest_gap_s"] < 1.0
