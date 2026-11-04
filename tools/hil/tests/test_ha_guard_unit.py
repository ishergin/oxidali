import subprocess

import pytest

import hil_test_guards

AVAILABILITY = "hiltest-dali/hiltest/availability"
FILTERS = ("hiltest/#", "hiltest-dali/#")
UNREADABLE = subprocess.TimeoutExpired("ssh", 1)


class _Ha:
    def __init__(self, log):
        self.log = log

    def patch(self, body):
        self.log.append(("patch", tuple(sorted(body))))


class _Api:
    def __init__(self, log):
        self.ha = _Ha(log)


class _Broker:
    def __init__(self, log, availability, sweep_fails=False):
        self.log = log
        self.availability = list(availability)
        self.sweep_fails = sweep_fails

    def retained(self, _cfg, topic):
        value = self.availability.pop(0) if len(self.availability) > 1 else self.availability[0]
        self.log.append(("read", topic, value))
        if isinstance(value, Exception):
            raise value
        return value

    def collect_retained(self, _cfg, filt):
        self.log.append(("collect", filt))
        if self.sweep_fails:
            raise UNREADABLE
        return [AVAILABILITY] if filt == "hiltest-dali/#" else []

    def clear_retained(self, _cfg, topic):
        self.log.append(("clear", topic))


@pytest.fixture()
def log():
    return []


def _leave(monkeypatch, log, availability, sweep_fails=False):
    broker = _Broker(log, availability, sweep_fails)
    for name in ("retained", "collect_retained", "clear_retained"):
        monkeypatch.setattr(hil_test_guards.mqtt_tap, name, getattr(broker, name))
    before = {"enabled": True, "controller_id": "dali-a1b2c3", "broker_password_set": True}
    hil_test_guards.leave_ha_namespace(_Api(log), None, before, AVAILABILITY, FILTERS)


def _first(log, kind):
    return next(i for i, call in enumerate(log) if call[0] == kind)


def test_a_live_test_session_is_swept_only_after_its_farewell_landed(monkeypatch, log):
    _leave(monkeypatch, log, ["online", "online", "offline"])

    assert log[_first(log, "patch")] == ("patch", ("controller_id", "enabled"))
    assert _first(log, "patch") < _first(log, "collect")
    assert log.index(("read", AVAILABILITY, "offline")) < _first(log, "collect"), (
        "the sweep ran before the bridge's farewell landed, so the farewell outlives it")
    assert ("clear", AVAILABILITY) in log


@pytest.mark.parametrize("availability", [None, "offline"])
def test_a_guard_never_entered_or_already_offline_sweeps_without_waiting(
        monkeypatch, log, availability):
    _leave(monkeypatch, log, [availability])

    assert [call[0] for call in log] == ["read", "patch", "collect", "collect", "clear"]


def test_an_unreadable_broker_is_waited_on_rather_than_taken_for_a_farewell(monkeypatch, log):
    _leave(monkeypatch, log, ["online", UNREADABLE, "offline"])

    assert log.index(("read", AVAILABILITY, "offline")) < _first(log, "collect")


def test_a_broker_unreadable_before_the_restore_never_holds_the_restore_back(monkeypatch, log):
    _leave(monkeypatch, log, [UNREADABLE, None])

    assert _first(log, "patch") == 1


def test_a_session_that_never_ends_is_still_swept_and_then_fails_the_teardown(
        monkeypatch, log):
    monkeypatch.setattr(hil_test_guards, "BRIDGE_LEAVE_S", 0.1)

    with pytest.raises(pytest.fail.Exception, match="still announces"):
        _leave(monkeypatch, log, ["online"])
    assert ("clear", AVAILABILITY) in log


def test_a_sweep_that_fails_is_named_after_the_settings_are_restored(monkeypatch, log):
    with pytest.raises(pytest.fail.Exception, match="could not sweep hiltest/#"):
        _leave(monkeypatch, log, [None], sweep_fails=True)
    assert _first(log, "patch") < _first(log, "collect")
