import os

import pytest

from hil_harness import track, validity_of

def _optical_unavailable(request, reason):
    if request.config.getoption("--no-camera") \
            or os.environ.get("HIL_NO_CAMERA"):
        pytest.skip(reason)
    pytest.fail(
        "optical channel unavailable: %s\n"
        "(pass --no-camera or set HIL_NO_CAMERA=1 to explicitly accept an "
        "optical-less run)" % reason, pytrace=False)


_OPTICAL_FIXTURES = frozenset(("camera", "calibration", "geometry", "oracle"))


def _run_needs_optics(session):
    return any(_OPTICAL_FIXTURES.intersection(item.fixturenames)
               for item in session.items)


@pytest.fixture(scope="session", autouse=True)
def optical_session(request):
    state = {"backend": None, "calibration": None, "error": None}
    opted_out = bool(request.config.getoption("--no-camera")
                     or os.environ.get("HIL_NO_CAMERA"))
    if opted_out or not _run_needs_optics(request.session):
        state["error"] = ("optical channel not requested for this run"
                          if not opted_out else
                          "optical channel opted out (--no-camera)")
        yield state
        return

    from hil.camera.backend import probe_and_select
    from hil.camera.calibrate import Calibrator
    from hil.camera.calibrate import load as load_cal
    hil_config = request.getfixturevalue("hil_config")
    try:
        state["backend"] = probe_and_select(hil_config)
        if not _skip_calibration(request):
            Calibrator(hil_config, request.getfixturevalue("api"),
                       state["backend"]).run()
        state["calibration"] = load_cal(hil_config)
    except Exception as exc:
        state["error"] = str(exc)
    yield state
    if state["backend"] is not None:
        state["backend"].close()


@pytest.fixture(scope="session")
def camera(optical_session, request):
    if optical_session["backend"] is None:
        _optical_unavailable(request, optical_session["error"] or "camera unavailable")
    return optical_session["backend"]


def _skip_calibration(request):
    return bool(request.config.getoption("--skip-calibration")
                or os.environ.get("HIL_SKIP_CALIBRATION"))


@pytest.fixture(scope="session")
def calibration(optical_session, camera, request):
    if optical_session["calibration"] is None:
        _optical_unavailable(
            request, optical_session["error"] or "calibration unavailable")
    return optical_session["calibration"]


@pytest.fixture(scope="session")
def geometry(hil_config, calibration):
    from hil.camera.masks import load_geometry
    return load_geometry(hil_config)


@pytest.fixture(scope="session")
def lamp_roster(optical_session, hil_config, request, api):
    if optical_session["calibration"] is not None:
        return optical_session["calibration"]["lamps"]
    if os.environ.get("HIL_LAMP_ROSTER") == "registry":
        bound = {(v.get("binding") or {}).get("physical_short_address"):
                 v["virtual_lamp_id"] for v in api.vlamps.list()["virtual_lamps"]}
        return [
            {"label": bound[d["short_address"]] + 1, "short_address": d["short_address"],
             "identity": {"gtin": d.get("gtin"),
                          "identification_number": d.get("identification_number")}}
            for d in api.devices()["physical_devices"]
            if d["short_address"] in bound and d.get("gtin") is not None
            and d.get("identification_number") is not None
        ]
    from hil.camera.calibrate import load as load_cal
    try:
        return load_cal(hil_config)["lamps"]
    except Exception as exc:
        _optical_unavailable(request, "lamp roster unavailable: %s" % exc)


@pytest.fixture(scope="session")
def lamps(api, lamp_roster, hil_config):
    from hil.identity import refresh_short_addresses
    mapping, missing = refresh_short_addresses(api, lamp_roster)
    allowed = hil_config.lamp_short_set()
    refused = {label: short for label, short in mapping.items() if short not in allowed}
    if refused:
        print("lamps: %s refused by HIL_LAMP_SHORTS=%s (owner's rule), not absent"
              % (sorted(refused.items()), hil_config.lamp_shorts))
    mapping = {label: short for label, short in mapping.items() if short in allowed}
    if not mapping:
        pytest.skip("none of the calibrated lamps are on the bus and permitted")

    class Lamps:
        by_label = mapping
        missing_labels = missing

        def short(self, label):
            if label not in mapping:
                pytest.skip("lamp %s (calibrated) not on the bus" % label)
            return mapping[label]

        def labels(self):
            return sorted(mapping)
    return Lamps()


@pytest.fixture(scope="session")
def _oracle_session(request, camera, calibration, geometry, hil_config):
    from hil.camera.calibrate import at_exposure_floor
    from hil.oracle import CameraOracle
    if at_exposure_floor(calibration):
        validity_of(request.config)["camera_exposure"] = (
            "calibration measured at the exposure FLOOR (%s x100us) with the "
            "glow ring still clipping at level 254 — thresholds are usable "
            "(tests run at 120-140) but the rig has no headroom: dim it or move "
            "the camera, then `hil calibrate`"
            % calibration.get("exposure_time_abs"))
    return track(request.config,
                  CameraOracle(camera, calibration, geometry, hil_config))


def _keep_calibration_fresh(oracle, hil_config, api, request):
    from hil.camera.calibrate import Calibrator, is_stale, age_seconds
    from hil.camera.calibrate import load as load_cal

    if _skip_calibration(request) or not is_stale(oracle.cal):
        return
    age_min = age_seconds(oracle.cal) / 60.0
    print("\ncalibration: profile %.0f min old — re-measuring before this test"
          % age_min)
    Calibrator(hil_config, api, oracle.backend).run()
    oracle.adopt_calibration(load_cal(hil_config))
    validity_of(request.config).setdefault("recalibrations", []).append(
        "%s (profile was %.0f min old)" % (request.node.name, age_min))


@pytest.fixture()
def camera_oracle(_oracle_session, test_artifacts, api, request, hil_config):
    from hil.camera.backend import CameraError
    _oracle_session.artifacts = test_artifacts
    _keep_calibration_fresh(_oracle_session, hil_config, api, request)
    try:
        _oracle_session.fresh_baseline(api)
    except CameraError as exc:
        _optical_unavailable(request, "baseline capture: %s" % exc)
    return _oracle_session
