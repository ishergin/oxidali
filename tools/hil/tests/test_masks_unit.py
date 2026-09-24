import numpy as np
import pytest

from hil.camera import masks as M
from hil.camera import metrics as MX

H, W = 360, 640


def synth_frame(ambient=30, lamps=()):
    frame = np.full((H, W, 3), float(ambient), np.float32)
    yy, xx = np.mgrid[0:H, 0:W]
    for cx, cy, r, bright, bgr in lamps:
        d2 = (xx - cx) ** 2 + (yy - cy) ** 2
        core = d2 <= r ** 2
        glow = (d2 <= (3 * r) ** 2) & ~core
        for c in range(3):
            frame[..., c][core] = np.minimum(bright * bgr[c] / 255.0, 255)
            frame[..., c][glow] += bright * bgr[c] / 255.0 * 0.25
    return np.clip(frame, 0, 255)


BASE = synth_frame()
LAMP_A = (160, 180, 22, 250, (255, 255, 255))
LAMP_B = (480, 180, 22, 250, (255, 255, 255))


def geometry(lamp):
    frame = synth_frame(lamps=[lamp])
    diff = M.gray_absdiff(frame, BASE)
    mask = M.detect_mask(diff)
    assert mask is not None
    return frame, mask, M.core_from(mask, diff), M.ring_from(mask)


def test_detect_mask_finds_the_lamp_not_the_room():
    _, mask, core, _ = geometry(LAMP_A)
    ys, xs = np.where(mask)
    assert abs(xs.mean() - 160) < 30 and abs(ys.mean() - 180) < 30
    assert mask.sum() < 0.3 * H * W
    assert core.sum() >= 30 and core.sum() <= mask.sum()


def test_focal_metric_rejects_neighbour_spill():
    frame_a, mask_a, core_a, ring_a = geometry(LAMP_A)
    frame_b = synth_frame(lamps=[LAMP_B])
    own = MX.lamp_metrics(frame_a, mask_a, core_a, ring_a, BASE)["core_delta"]
    spill = MX.lamp_metrics(frame_b, mask_a, core_a, ring_a, BASE)["core_delta"]
    assert own > 50, own
    assert spill < 8, spill


def test_metrics_detect_colour():
    red = (160, 180, 22, 250, (0, 0, 255))
    frame = synth_frame(lamps=[red])
    diff = M.gray_absdiff(frame, BASE)
    mask = M.detect_mask(diff)
    m = MX.lamp_metrics(frame, mask, baseline_bgr=BASE)
    assert m["colour_blind"] is None, m
    assert m["hue"] <= 20 or m["hue"] >= 330, m
    assert m["rb_ratio"] > 3, m


def test_colour_of_a_clipped_emitter_comes_from_the_unclipped_glow():
    blue = (160, 180, 26, 255, (255, 0, 0))
    frame = synth_frame(lamps=[blue])
    mask = M.detect_mask(M.gray_absdiff(frame, BASE))
    m = MX.lamp_metrics(frame, mask, ring=M.ring_from(mask), baseline_bgr=BASE)
    assert m["colour_blind"] is None, m
    assert 190 <= m["hue"] <= 260, m
    assert m["signal_pixels"] >= MX.MIN_SIGNAL_PIXELS, m


def test_fully_clipped_region_reports_blind_not_a_neutral():
    frame = np.full((H, W, 3), 255.0, np.float32)
    mask = np.zeros((H, W), bool)
    mask[150:210, 130:190] = True
    m = MX.lamp_metrics(frame, mask, baseline_bgr=BASE)
    assert m["colour_blind"] == "colour region fully clipped", m
    assert m["rb_ratio"] is None and m["hue"] is None and m["mean_rgb"] is None, m
    assert m["colour_clip_fraction"] == 1.0, m


def test_unlit_lamp_does_not_report_the_rooms_colour():
    warm_room = np.zeros((H, W, 3), np.float32)
    warm_room[..., 0], warm_room[..., 1], warm_room[..., 2] = 12.0, 22.0, 40.0
    mask = np.zeros((H, W), bool)
    mask[150:210, 130:190] = True
    m = MX.lamp_metrics(warm_room, mask, baseline_bgr=warm_room)
    assert m["colour_blind"] is not None, m
    assert m["hue"] is None and m["rb_ratio"] is None, m


def test_a_handful_of_lit_pixels_is_blind_not_a_measurement():
    frame = BASE.copy()
    frame[180, 158:161] = (40.0, 60.0, 220.0)
    mask = np.zeros((H, W), bool)
    mask[150:210, 130:190] = True
    m = MX.lamp_metrics(frame, mask, baseline_bgr=BASE)
    assert m["colour_blind"] == "only 3 lit unclipped pixels", m
    assert m["signal_pixels"] == 3, m
    assert m["hue"] is None, m


def test_brightness_ordering_is_monotonic():
    values = []
    for bright in (80, 150, 250):
        lamp = (160, 180, 22, bright, (255, 255, 255))
        frame = synth_frame(lamps=[lamp])
        mask = M.detect_mask(M.gray_absdiff(frame, BASE))
        values.append(MX.lamp_metrics(frame, mask, baseline_bgr=BASE)["diff_sum_lum"])
    assert values[0] < values[1] < values[2], values


def test_labelmap_roundtrip(tmp_path):
    _, mask_a, _, _ = geometry(LAMP_A)
    _, mask_b, _, _ = geometry(LAMP_B)
    path = tmp_path / "labelmap.png"
    M.save_labelmap(path, {1: mask_a, 3: mask_b})
    loaded = M.load_labelmap(path)
    assert set(loaded) == {1, 3}
    assert (loaded[1] == mask_a).all() and (loaded[3] == mask_b).all()


def test_colour_acceptance_rejects_blind_and_lagged_samples():
    from hil.camera import calibrate as C

    white = {"colour_blind": None, "hue": 224, "sat": 0.45}
    lagged_blue = {"colour_blind": None, "hue": 224, "sat": 0.99}
    blind = {"colour_blind": "colour region fully clipped", "hue": None, "sat": None}
    assert C.colour_ok("white", white)
    assert not C.colour_ok("white", lagged_blue)
    assert not C.colour_ok("white", blind)
    assert not C.colour_ok("red", blind)


def test_calibration_loader_rejects_v1(tmp_path, monkeypatch):
    from hil.camera import calibrate as C

    class Cfg:
        state_dir = tmp_path
    (tmp_path / "calibration.json").write_text('{"rois": {}}')
    with pytest.raises(C.CalibrationError, match="hil calibrate"):
        C.load(Cfg)


def test_calibration_loader_rejects_the_previous_schema(tmp_path):
    from hil.camera import calibrate as C

    class Cfg:
        state_dir = tmp_path
    (tmp_path / "calibration.json").write_text('{"version": 2, "lamps": []}')
    with pytest.raises(C.CalibrationError, match="hil calibrate"):
        C.load(Cfg)
