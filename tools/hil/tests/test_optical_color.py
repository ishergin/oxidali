import time

import pytest

from hil.camera.calibrate import (COLOR_RAMP_S, RGB_PRIMARIES, cct_setpoint,
                                  pick_cct_anchor, rgb_setpoint)

pytestmark = [pytest.mark.smoke, pytest.mark.optical]


@pytest.mark.needs_capability("rgb")
def test_rgb_classification(api, lamps, capabilities, camera_oracle,
                            state_snapshot, needs_capability):
    short = capabilities.any_lamp_with("rgb")
    label = next((l for l, s in lamps.by_label.items() if s == short), None)
    if label is None:
        pytest.skip("rgb-capable lamp %s is not in the calibration" % short)
    for name, rgb in RGB_PRIMARIES:
        setpoint = rgb_setpoint(rgb)
        api.ts(short, setpoint)
        camera_oracle.assert_hue(label, name,
                                 resend=lambda sp=setpoint: api.ts(short, sp))
    api.off(short)


@pytest.mark.needs_capability("rgb")
def test_rgb_colour_without_level_activates(api, lamps, capabilities,
                                            camera_oracle, state_snapshot,
                                            needs_capability):
    short = capabilities.any_lamp_with("rgb")
    label = next((l for l, s in lamps.by_label.items() if s == short), None)
    if label is None:
        pytest.skip("rgb-capable lamp %s is not in the calibration" % short)
    api.ts(short, {"power": "on", "level": 200})
    time.sleep(COLOR_RAMP_S)
    for name, rgb in RGB_PRIMARIES[:2]:
        setpoint = {"power": "on", "color_mode": "rgb",
                    "rgb": {"r": rgb[0], "g": rgb[1], "b": rgb[2]}}
        api.ts(short, setpoint)
        camera_oracle.assert_hue(label, name,
                                 resend=lambda sp=setpoint: api.ts(short, sp))
    api.off(short)


@pytest.mark.needs_capability("cct")
def test_cct_ordering(api, lamps, camera_oracle, calibration, state_snapshot,
                      test_artifacts, needs_capability):
    label, spread = camera_oracle.cct_fingerprint_spread()
    if label is None:
        pytest.skip("no cct fingerprints in this calibration")
    if spread < 1.15:
        pytest.skip("cct fingerprint spread %.2f < 1.15 — optically degenerate "
                    "on this rig/lighting" % spread)
    short = lamps.short(label)
    if not needs_capability.ensure(short, "cct"):
        pytest.skip("cct capability absent on fingerprint lamp %s" % short)
    anchor_label = pick_cct_anchor(
        lamps.labels(), label,
        lambda l: needs_capability.ensure(lamps.short(l), "rgb"))
    if anchor_label is not None:
        api.ts(lamps.short(anchor_label), {"power": "on", "level": 120})
    fp = next(v for k, v in calibration["fingerprints"].items()
              if k.startswith("cct_addr_") and "2700" in v and "6500" in v)
    expected = {k: fp[str(k)]["rb_ratio"] for k in (2700, 6500)}
    ratios = {}
    for kelvin in (2700, 6500):
        setpoint = cct_setpoint(kelvin)
        api.ts(short, setpoint)
        time.sleep(COLOR_RAMP_S)
        m = camera_oracle.require_colour(
            camera_oracle.measure(label, name="cct_%d" % kelvin), label, "cct")
        other = 6500 if kelvin == 2700 else 2700
        if abs(m["rb_ratio"] - expected[kelvin]) > abs(m["rb_ratio"] - expected[other]):
            camera_oracle.count_retry("gear_colour_lag")
            api.ts(short, setpoint)
            time.sleep(COLOR_RAMP_S)
            m = camera_oracle.require_colour(
                camera_oracle.measure(label, name="cct_%d_lagretry" % kelvin),
                label, "cct")
        ratios[kelvin] = m["rb_ratio"]
    api.off(short)
    if anchor_label is not None:
        api.off(lamps.short(anchor_label))
    camera_oracle.judge_cct_order(ratios)
