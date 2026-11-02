import pytest

import hil_test_guards

FAREWELL = "hiltest-dali/hiltest/availability"
FILTERS = ("hiltest/#", "hiltest-dali/#")


class _Ha:
    def __init__(self, log):
        self.log = log

    def patch(self, body):
        self.log.append(("patch", tuple(sorted(body))))


class _Api:
    def __init__(self, log, sessions):
        self.log = log
        self.ha = _Ha(log)
        self._sessions = list(sessions)

    def diagnostics(self):
        connected, connects = self._sessions.pop(0) if len(self._sessions) > 1 else self._sessions[0]
        self.log.append(("diag", connected, connects))
        return {"mqtt": {"connected": connected, "connects_total": connects}}


@pytest.fixture()
def log(monkeypatch):
    calls = []
    monkeypatch.setattr(hil_test_guards.mqtt_tap, "collect_retained",
                        lambda _cfg, filt: calls.append(("collect", filt))
                        or ([FAREWELL] if filt == "hiltest-dali/#" else []))
    monkeypatch.setattr(hil_test_guards.mqtt_tap, "clear_retained",
                        lambda _cfg, topic: calls.append(("clear", topic)))
    return calls


def _leave(log, sessions):
    before = {"enabled": True, "controller_id": "dali-a1b2c3", "broker_password_set": True}
    hil_test_guards.leave_ha_namespace(_Api(log, sessions), None, before, FILTERS)


def _first(log, kind):
    return next(i for i, call in enumerate(log) if call[0] == kind)


def test_the_retained_topics_are_cleared_only_after_the_hil_session_ended(log):
    _leave(log, [(True, 5), (True, 5), (True, 6)])

    assert log[_first(log, "patch")] == ("patch", ("controller_id", "enabled"))
    assert _first(log, "patch") < _first(log, "collect")
    assert log.index(("diag", True, 6)) < _first(log, "collect"), (
        "the bridge's farewell lands after a clear that did not wait for the session to end")
    assert ("clear", FAREWELL) in log


def test_a_bridge_without_a_session_at_restore_is_not_waited_for(log):
    _leave(log, [(False, 5)])

    assert _first(log, "patch") < _first(log, "collect")
    assert ("clear", FAREWELL) in log


def test_a_session_that_never_ends_is_still_cleaned_and_then_fails_the_teardown(log, monkeypatch):
    monkeypatch.setattr(hil_test_guards, "BRIDGE_LEAVE_S", 0.1)

    with pytest.raises(AssertionError, match="kept its HIL-namespace session"):
        _leave(log, [(True, 5)])
    assert ("clear", FAREWELL) in log
