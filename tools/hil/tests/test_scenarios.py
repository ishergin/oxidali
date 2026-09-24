import time

import pytest

from hil.camera.calibrate import RGB_PRIMARIES, rgb_setpoint


def _rgb_lamp(lamps, capabilities):
    for label in lamps.labels():
        if capabilities.ensure(lamps.short(label), "rgb"):
            return label
    return None


@pytest.mark.optical
@pytest.mark.sniffer
def test_target_state_level_and_color(api, lamps, capabilities, camera_oracle,
                                      sniffer, paced, state_snapshot):
    label = lamps.labels()[0]
    short = lamps.short(label)
    with sniffer.window() as win:
        paced(1.0)
        api.ts(short, {"power": "on", "level": 200})
        win.expect_frame("DAPC short %d -> level 200" % short)
        camera_oracle.assert_on(label, window=win)

    rgb_label = _rgb_lamp(lamps, capabilities)
    if rgb_label is not None:
        rgb_short = lamps.short(rgb_label)
        _, red = RGB_PRIMARIES[0]
        setpoint = rgb_setpoint(red)
        with sniffer.window() as win:
            paced(1.0)
            api.ts(rgb_short, setpoint)
            win.expect_frame("DT8 SET TEMP RGB DIM LEVEL")
            camera_oracle.assert_hue(rgb_label, "red", window=win,
                                     resend=lambda: api.ts(rgb_short, setpoint))
    api.off_all()


def _group_members(api, gid):
    members = []
    for d in api.devices()["physical_devices"]:
        bitmask = d.get("groups_membership")
        if bitmask is not None and bitmask & (1 << gid):
            members.append(d["short_address"])
    return members


@pytest.mark.optical
@pytest.mark.sniffer
def test_group_target_state_reaches_members(api, lamps, camera_oracle, sniffer,
                                            paced, state_snapshot, capabilities,
                                            test_artifacts):
    for short in [d["short_address"] for d in api.devices()["physical_devices"]]:
        capabilities.ensure(short, "groups")
    gid, members = None, []
    for candidate in range(16):
        m = _group_members(api, candidate)
        if len(m) >= 2:
            gid, members = candidate, m
            break
    if gid is None:
        pytest.skip("no DALI group with >= 2 member lamps on this rig")
    test_artifacts.attach_json("group", {"gid": gid, "members": members})

    with sniffer.window() as win:
        paced(1.0)
        api.group_ts(gid, {"power": "on", "level": 254})
        win.expect_frame("DAPC group %d -> level 254" % gid)
        for witness in (l for l, s in lamps.by_label.items() if s in members):
            camera_oracle.assert_on(witness, window=win,
                                    name="member%s_on" % witness)
    time.sleep(1.0)
    test_artifacts.attach_json("member_states_after", {
        str(s): (api.state(s).get("state") or {}).get("level") for s in members})
    api.off_all()


@pytest.mark.sniffer
def test_discovery_scan_is_readonly(api, sniffer, test_artifacts):
    before = {d["short_address"]: d.get("random_address")
              for d in api.devices()["physical_devices"]}
    with sniffer.window() as win:
        op = api.wait_op(api.discovery("scan_known_short_addresses"), timeout_s=90)
        test_artifacts.attach_json("scan_op", op)
        time.sleep(2.0)
        frames = win.frames()
        test_artifacts.attach_json("scan_frames", [f["decoded"] for f in frames])
        readdress = [f for f in frames
                     if "PROGRAM SHORT ADDRESS" in f["decoded"]
                     or ("RANDOMISE" in f["decoded"]
                         and f["decoded"].endswith("0x00"))]
        suspicious = [f for f in frames
                      if "RANDOMISE" in f["decoded"]
                      and not f["decoded"].endswith("0x00")]
        if suspicious:
            test_artifacts.attach_json("suspected_corrupted_frames",
                                       [f["decoded"] for f in suspicious])
        assert not readdress, "re-addressing frames during read-only scan: %s" % (
            [f["decoded"] for f in readdress])
    after = {d["short_address"]: d.get("random_address")
             for d in api.devices()["physical_devices"]}
    assert sorted(after) == sorted(before), \
        "short-address set changed: %s -> %s" % (sorted(before), sorted(after))
    random_diffs = {s: (before[s], after[s]) for s in before
                    if s in after and before[s] != after[s]}
    if random_diffs:
        test_artifacts.attach_json("random_address_instability", random_diffs)
