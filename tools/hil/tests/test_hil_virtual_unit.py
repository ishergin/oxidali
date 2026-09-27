import pytest

import hil_virtual
from hil import api as api_mod
from hil import prod_state, tripwire, virtual_gear
from hil.lamp_guard import VirtualFence
from hil.seriallog import LogWindow

PARK = [16, 17]


def _tx(frame):
    return "I (1) dali: DALI PHY TX: forward16 0x%04x (ISR-owned bitbang)\n" % frame


class _Admin:
    def __init__(self, log, answers=True, fails=False):
        self.log, self.answers, self.fails, self.sent = log, answers, fails, []

    def cmd(self, short, opcode):
        self.sent.append((short, opcode))
        if self.fails:
            raise api_mod.ApiError(503, "busy", "dali/command")
        if self.answers:
            with open(self.log, "a") as fh:
                fh.write(_tx((((short << 1) | 1) << 8) | opcode))
        return {"success": True}

    def stats(self):
        return {"dali": {}}


@pytest.fixture()
def quick(monkeypatch):
    monkeypatch.setattr(hil_virtual, "QUIET_S", 0.05)
    monkeypatch.setattr(hil_virtual, "SETTLE_MAX_S", 0.5)
    monkeypatch.setattr(hil_virtual, "BARRIER_TIMEOUT_S", 0.3)
    monkeypatch.setattr(hil_virtual, "POLL_S", 0.01)


def _window(tmp_path, *frames):
    log = tmp_path / "serial.log"
    log.write_text("")
    window = LogWindow(log).__enter__()
    with open(log, "a") as fh:
        fh.writelines(_tx(f) for f in frames)
    return log, window


def test_the_barrier_proves_the_log_is_live(tmp_path, quick):
    log, window = _window(tmp_path, 0x2190)
    admin = _Admin(log)
    assert hil_virtual._flush(window, admin, 16)
    assert admin.sent == [(16, tripwire.QUERY_STATUS)]


def test_a_test_frame_that_looks_like_the_barrier_does_not_count(tmp_path, quick):
    log, window = _window(tmp_path, 0x2190)
    assert not hil_virtual._flush(window, _Admin(log, answers=False), 16)


def test_a_barrier_the_dut_refuses_means_a_blind_tripwire(tmp_path, quick):
    log, window = _window(tmp_path)
    assert not hil_virtual._flush(window, _Admin(log, fails=True), 16)


class _Config:
    def __init__(self):
        self._hil_validity = {}


class _Request:
    def __init__(self):
        self.config = _Config()
        self.node = type("Node", (), {"nodeid": "tests/test_virtual_gear.py::test_x"})()


class _Cfg:
    def peer(self):
        return self


def _judge(monkeypatch, flushed=True, found=(), lost=None, peer=None):
    monkeypatch.setattr(hil_virtual.tripwire, "lost_lines", lambda before, after: lost or {})
    monkeypatch.setattr(hil_virtual.role, "controller_health", lambda cfg: peer)
    monkeypatch.setattr(hil_virtual.serialmon, "alive", lambda cfg: False)
    request = _Request()
    hil_virtual._judge(request, _Cfg(), _Admin(None), {}, flushed, list(found))
    return request.config._hil_validity


@pytest.mark.parametrize("kwargs,code", [
    ({"found": ["DAPC to SA6 went out"]}, hil_virtual.EXIT_SAFETY),
    ({"flushed": False}, hil_virtual.EXIT_BLIND),
    ({"peer": {"role": "standby"}}, hil_virtual.EXIT_PEER_RETURNED),
])
def test_each_stop_has_its_own_exit_code(monkeypatch, kwargs, code):
    with pytest.raises(pytest.exit.Exception) as stop:
        _judge(monkeypatch, **kwargs)
    assert stop.value.returncode == code


def test_lost_lines_leave_the_test_inconclusive(monkeypatch):
    state = _judge(monkeypatch, lost={"console_log_dropped_total": 2})
    assert "cannot vouch" in state["virtual_gear_inconclusive"][0]


def test_the_session_sweep_judges_frames_between_tests(tmp_path, quick):
    log, whole = _window(tmp_path, 0x2105, 0x0d05)
    state = {"virtual_gear_safety": []}
    opened = {"park": PARK, "groups": [4], "vl_of_short": {"16": 60, "17": 61}}
    hil_virtual._sweep_with(whole, _Admin(log), opened, state)
    assert len(state["virtual_gear_safety"]) == 1 and "SA6" in state["virtual_gear_safety"][0]


def test_the_sweep_does_not_report_a_frame_twice(tmp_path, quick):
    log, whole = _window(tmp_path, 0x0d05)
    fence = VirtualFence(PARK, [4], [60, 61])
    first = tripwire.violations(whole.lines(), fence)[0]
    state = {"virtual_gear_safety": ["tests/test_virtual_gear.py::test_x: %s" % first]}
    opened = {"park": PARK, "groups": [4], "vl_of_short": {"16": 60, "17": 61}}
    hil_virtual._sweep_with(whole, _Admin(log), opened, state)
    assert len(state["virtual_gear_safety"]) == 1


class _Lamps:
    def __init__(self):
        self.vlamps = self
        self.unbound = []

    def list(self):
        return {"virtual_lamps": [{"virtual_lamp_id": 60, "binding": {"physical_short_address": 16}}]}

    def list_unfiltered(self):
        return {"virtual_lamps": [
            {"virtual_lamp_id": 1, "binding": {"physical_short_address": 1}},
            {"virtual_lamp_id": 60, "binding": {"physical_short_address": 16}}]}

    def unbind(self, lamp_id):
        self.unbound.append(lamp_id)


def test_the_session_counts_every_vl_the_controller_holds():
    assert virtual_gear.vl_ids(_Lamps()) == [1, 60]


def test_a_restore_keeps_an_owner_binding_a_narrowed_roster_hides():
    api = _Lamps()
    snap = {"vl": api.list_unfiltered()}
    prod_state._restore_vl(api, snap, log=lambda line: None)
    assert api.unbound == []
