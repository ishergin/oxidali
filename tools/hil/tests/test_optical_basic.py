import pytest

pytestmark = [pytest.mark.smoke, pytest.mark.optical]


def test_each_lamp_on_off(api, lamps, camera_oracle, state_snapshot):
    api.off_all()
    camera_oracle.invalidate_baseline()
    camera_oracle.fresh_baseline(api)
    for label in lamps.labels():
        short = lamps.short(label)
        on = {"power": "on", "level": 200}
        api.ts(short, on)
        camera_oracle.assert_on(label, name="lamp%s_on" % label,
                                resend=lambda s=short: api.ts(s, on))
        api.off(short)
        camera_oracle.assert_off(label, name="lamp%s_off" % label,
                                 resend=lambda s=short: api.off(s))


def test_brightness_strictly_monotonic(api, lamps, calibration, camera_oracle,
                                       state_snapshot, test_artifacts):
    label = calibration["ladder_label"]
    short = lamps.short(label)
    values = []
    for level in calibration["monotonic_levels"]:
        api.ts(short, {"power": "on", "level": level})
        m = camera_oracle.measure(label, name="level_%d" % level)
        values.append(camera_oracle.mono_metric(m))
    api.off(short)
    test_artifacts.attach_json("ladder", dict(zip(calibration["monotonic_levels"], values)))
    camera_oracle.assert_strict_order(values, context="brightness ladder")
