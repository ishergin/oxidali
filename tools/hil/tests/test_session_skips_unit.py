import types

import pytest

import hil_session
import hil_session_guards
import test_groups
import test_input_devices
import test_policies
import test_redundancy
import test_target_state
import test_virtual_gear
import test_ws
from hil.config import HilConfig
from hil.lamp_guard import LampNotAllowed

UART = "rfc2217://127.0.0.1:4444"


class _Item:
    def __init__(self, markers, fixturenames=()):
        self.markers = set(markers)
        self.fixturenames = list(fixturenames)
        self.added = []

    def get_closest_marker(self, name):
        return name if name in self.markers else None

    def add_marker(self, marker):
        self.added.append(marker)


def _skip_reasons(item):
    return [m.kwargs["reason"] for m in item.added if m.name == "skip"]


def test_a_serial_test_is_skipped_while_the_console_is_unreachable():
    uart, plain = _Item({"smoke", "serial"}), _Item({"smoke"})
    hil_session._mark_skips([uart, plain], hil_session.NO_SERIAL, UART)
    assert _skip_reasons(uart) == ["serial port %s unreachable — %s" % (UART, hil_session.NO_SERIAL)]
    assert _skip_reasons(plain) == []


def test_a_serial_test_runs_while_the_console_answers():
    uart = _Item({"smoke", "serial"})
    hil_session._mark_skips([uart], None, UART)
    assert _skip_reasons(uart) == []


def test_the_log_channel_test_asks_for_the_serial_console():
    marks = {mark.name for mark in test_ws.test_the_log_channel_replays_and_keeps_uart_alive.pytestmark}
    assert {"smoke", "serial"} <= marks, (
        "HIL-WS-08 asserts on the UART; without the serial marker it fails, instead of "
        "skipping, on a run with no serial console"
    )



def test_a_light_test_is_selected_only_when_the_run_may_drive_lamps():
    light, plain = _Item({"light"}), _Item({"smoke"})
    assert hil_session.ungated(light, drives_lamps=False, commits_rules=True)
    assert not hil_session.ungated(light, drives_lamps=True, commits_rules=False)
    assert not hil_session.ungated(plain, drives_lamps=False, commits_rules=False)


def test_a_rule_committing_test_is_selected_only_with_its_flag():
    rules = _Item(set(), fixturenames=["api", "rules_guard"])
    assert hil_session.ungated(rules, drives_lamps=True, commits_rules=False)
    assert not hil_session.ungated(rules, drives_lamps=False, commits_rules=True)


class _Hook:
    def __init__(self):
        self.deselected = []

    def pytest_deselected(self, items):
        self.deselected.extend(items)


class _Config:
    def __init__(self):
        self.hook = _Hook()


def test_the_default_run_deselects_light_and_rule_committing_tests(monkeypatch):
    monkeypatch.delenv(hil_session.RULE_COMMITS_ENV, raising=False)
    light, rules, plain = (_Item({"light"}), _Item(set(), ["rules_guard"]), _Item({"smoke"}))
    items, config = [light, rules, plain], _Config()
    hil_session._deselect_ungated(config, items, HilConfig(lamp_shorts="", serial_remote=""))
    assert items == [plain] and config.hook.deselected == [light, rules]
    items = [light, rules, plain]
    monkeypatch.setenv(hil_session.RULE_COMMITS_ENV, "1")
    hil_session._deselect_ungated(_Config(), items, HilConfig(lamp_shorts="2",
                                                              lamps_read_only=True,
                                                              serial_remote=""))
    assert items == [rules, plain]


def test_no_lamp_is_drivable_unless_the_run_names_one(monkeypatch):
    monkeypatch.delenv("HIL_LAMP_SHORTS", raising=False)
    assert HilConfig(serial_remote="").lamp_short_set() == frozenset()


def test_the_new_light_tests_carry_the_light_marker():
    for test in (test_target_state.test_a_colour_temperature_series_during_a_fade_leaves_the_fixture_at_the_last_value,
                 test_groups.test_a_stop_fade_rule_leaves_real_members_where_it_caught_them):
        assert _light(test), test.__name__


def _light(test):
    marks = {mark.name for mark in getattr(test, "pytestmark", [])}
    return "light" in marks | {mark.name for mark in _module_marks(test)}


def _module_marks(test):
    marks = __import__(test.__module__).__dict__.get("pytestmark", [])
    return marks if isinstance(marks, list) else [marks]


def test_the_policy_test_is_destructive_and_the_emulated_ones_are_not_light():
    assert "destructive" in {m.name for m in _module_marks(
        test_policies.test_a_discovery_run_writes_the_armed_policy_without_a_manual_apply)}
    assert "light" not in {m.name for m in _module_marks(
        test_virtual_gear.test_a_stop_fade_rule_sends_one_dapc_mask_to_its_group_and_no_level)}


class _Schedules:
    def __init__(self):
        self.enabled, self.patches = {"owner-evening": True}, []

    def list(self):
        return [{"schedule_id": k, "enabled": v} for k, v in self.enabled.items()]

    def patch(self, schedule_id, body):
        self.patches.append((schedule_id, body))
        self.enabled[schedule_id] = body["enabled"]


class _HclApi:
    def __init__(self):
        self.hcl = _Schedules()


def test_a_run_that_drives_no_lamp_leaves_the_owner_schedules_alone():
    api = _HclApi()
    findings, suspended = hil_session_guards._neutralize_schedules(api, suspend=False)
    assert api.hcl.patches == [] and suspended == []
    assert "left 1 HCL schedule(s)" in findings[0]
    findings, suspended = hil_session_guards._neutralize_schedules(api, suspend=True)
    assert api.hcl.patches == [("owner-evening", {"enabled": False})]
    assert suspended == ["owner-evening"]


def test_only_a_named_lamp_without_read_only_makes_a_run_drive_lamps():
    assert not HilConfig(lamp_shorts="", lamps_read_only=False, serial_remote="").drives_lamps()
    assert not HilConfig(lamp_shorts="2", lamps_read_only=True, serial_remote="").drives_lamps()
    assert HilConfig(lamp_shorts="2", lamps_read_only=False, serial_remote="").drives_lamps()


def test_the_hcl_guard_refuses_a_run_that_drives_no_lamp():
    api = _HclApi()
    api.cfg = HilConfig(lamp_shorts="", serial_remote="")
    with pytest.raises(pytest.skip.Exception, match=r"leaves the owner's HCL schedules alone"):
        hil_session_guards.refuse_schedule_suspension(api)
    assert api.hcl.patches == []


class _Excinfo:
    def __init__(self, exc):
        self.value = exc

    def errisinstance(self, cls):
        return isinstance(self.value, cls)


def _failed(when):
    return types.SimpleNamespace(when=when, outcome="failed", longrepr="boom",
                                 location=("tests/test_x.py", 7, "test_x"))


def test_a_guard_refusal_during_a_test_is_a_skip_that_names_its_cause():
    refusal = LampNotAllowed("target-state of SA1 refused: SA1 is outside HIL_LAMP_SHORTS")
    for when in ("setup", "call"):
        report = _failed(when)
        hil_session.skip_guard_refusal(report, types.SimpleNamespace(excinfo=_Excinfo(refusal)))
        assert report.outcome == "skipped" and "SA1 is outside" in report.longrepr[2]
    teardown = _failed("teardown")
    hil_session.skip_guard_refusal(teardown, types.SimpleNamespace(excinfo=_Excinfo(refusal)))
    assert teardown.outcome == "failed"
    other = _failed("call")
    hil_session.skip_guard_refusal(other, types.SimpleNamespace(
        excinfo=_Excinfo(AssertionError("a product failure"))))
    assert other.outcome == "failed"


def test_the_default_run_is_read_only(monkeypatch):
    monkeypatch.delenv("HIL_LAMPS_READ_ONLY", raising=False)
    assert HilConfig(serial_remote="").lamps_read_only
    for value in ("", " ", "false", "no", "1"):
        monkeypatch.setenv("HIL_LAMPS_READ_ONLY", value)
        assert HilConfig(serial_remote="").lamps_read_only, value
    monkeypatch.setenv("HIL_LAMPS_READ_ONLY", "0")
    assert not HilConfig(serial_remote="").lamps_read_only


class _ZoneApi:
    def __init__(self, zone):
        self.zone, self.sets = zone, []

    def time_get(self):
        return {"timezone": self.zone}

    def time_set(self, timezone):
        self.sets.append(timezone)
        self.zone = timezone


def test_a_run_that_drives_no_lamp_reports_a_wrong_zone_and_never_moves_the_clock():
    api = _ZoneApi("UTC0")
    findings = hil_session_guards._neutralize_timezone(api, drives_lamps=False)
    assert api.sets == [] and "left as it is" in findings[0]
    findings = hil_session_guards._neutralize_timezone(api, drives_lamps=True)
    assert api.sets == [hil_session_guards.BENCH_BASELINE_TZ] and "restored" in findings[0]


def test_every_test_that_forges_an_input_event_is_a_light_test():
    for name in ("test_an_injected_event_frame_is_received_and_decoded",
                 "test_an_injected_event_activates_a_rule_and_reaches_the_gear",
                 "test_a_scheme_2_event_is_retyped_by_the_registry",
                 "test_the_button_vocabulary_reaches_a_rule_from_a_real_panel",
                 "test_a_power_notification_is_decoded_as_a_lifecycle_fact",
                 "test_a_scheme_0_event_is_counted_as_unattributable",
                 "test_a_press_reaches_home_assistant"):
        assert _light(getattr(test_input_devices, name)), name


def test_every_test_that_hands_the_bus_to_the_peer_is_a_light_test():
    for name in ("test_a_silent_primary_hands_the_bus_over_and_takes_it_back",
                 "test_a_planned_switchover_moves_the_bus_and_not_the_light"):
        assert _light(getattr(test_redundancy, name)), name

