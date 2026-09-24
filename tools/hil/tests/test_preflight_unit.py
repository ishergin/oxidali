import json
import types

import hil.api
import hil.serialmon
from hil import preflight
from hil.camera.calibrate import MIN_EXPOSURE, SCHEMA_VERSION, at_exposure_floor
from hil.config import load as load_config


class _FakeClient:
    def __init__(self, cfg, answer=True):
        self._answer = answer

    def health(self):
        if not self._answer:
            raise RuntimeError("DUT unreachable")
        return {"status": "ok"}


def _patch(monkeypatch, tmp_log, *, alive=True, dut_answers=True, grows=False):
    monkeypatch.setattr(hil.serialmon, "alive", lambda cfg: alive)
    monkeypatch.setattr(hil.serialmon, "log_path", lambda cfg: tmp_log)

    def client(cfg):
        if grows:
            with tmp_log.open("a") as handle:
                handle.write("HTTP access: GET /api/v1/health -> 200\n")
        return _FakeClient(cfg, answer=dut_answers)

    monkeypatch.setattr(hil.api, "Client", client)


def test_silent_channel_fails_and_names_the_replug(monkeypatch, tmp_path):
    log = tmp_path / "serial.log"
    log.write_text("existing\n")
    _patch(monkeypatch, log, grows=False)

    status, name, detail = preflight._check_monitor(load_config())
    assert status == preflight.FAIL
    assert name == "serial monitor"
    assert "SILENT" in detail and "re-plug" in detail


def test_live_channel_passes(monkeypatch, tmp_path):
    log = tmp_path / "serial.log"
    log.write_text("existing\n")
    _patch(monkeypatch, log, grows=True)

    status, _, _ = preflight._check_monitor(load_config())
    assert status == preflight.OK


def test_dead_monitor_process_still_reported_first(monkeypatch, tmp_path):
    log = tmp_path / "serial.log"
    _patch(monkeypatch, log, alive=False)

    status, _, detail = preflight._check_monitor(load_config())
    assert status == preflight.FAIL
    assert "not running" in detail


def test_unreachable_dut_warns_instead_of_blaming_the_channel(monkeypatch, tmp_path):
    log = tmp_path / "serial.log"
    log.write_text("existing\n")
    _patch(monkeypatch, log, dut_answers=False, grows=False)

    status, _, detail = preflight._check_monitor(load_config())
    assert status == preflight.WARN
    assert "not exercised" in detail



def test_exposure_floor_detected_by_flag_and_by_stored_exposure():
    assert at_exposure_floor({"exposure_floor_hit": True, "exposure_time_abs": 313})
    assert at_exposure_floor({"exposure_time_abs": MIN_EXPOSURE})
    assert not at_exposure_floor({"exposure_time_abs": 313})
    assert not at_exposure_floor({}), "an absent exposure is unknown, not a floor"


def _calibration_doc(tmp_path, **overrides):
    doc = {"version": SCHEMA_VERSION, "profile": "night",
           "exposure_time_abs": 313}
    doc.update(overrides)
    (tmp_path / "calibration.json").write_text(json.dumps(doc))
    return types.SimpleNamespace(state_dir=str(tmp_path))


def test_floor_hit_calibration_warns_without_claiming_the_profile_is_unusable(tmp_path):
    cfg = _calibration_doc(tmp_path, exposure_time_abs=MIN_EXPOSURE)
    status, name, detail = preflight._check_calibration(cfg)
    assert (status, name) == (preflight.WARN, "calibration")
    assert "FLOOR" in detail and "usable" in detail
    assert "FAIL" not in detail, "the floor does not fail the optical tier"


def test_healthy_calibration_passes(tmp_path):
    status, _, _ = preflight._check_calibration(_calibration_doc(tmp_path))
    assert status == preflight.OK


def test_calibration_age_comes_from_the_stamp_not_the_file():
    from hil.camera.calibrate import age_seconds, is_stale, CALIBRATION_TTL_S
    import time as _t

    now = _t.time()
    fresh = {"created": _t.strftime("%Y-%m-%dT%H:%M:%S", _t.localtime(now - 60))}
    stale = {"created": _t.strftime("%Y-%m-%dT%H:%M:%S", _t.localtime(now - CALIBRATION_TTL_S - 60))}
    assert age_seconds(fresh, now) < 120
    assert not is_stale(fresh, now=now)
    assert is_stale(stale, now=now)
    assert is_stale({}, now=now)
    assert age_seconds({"created": "not-a-date"}, now) == float("inf")
