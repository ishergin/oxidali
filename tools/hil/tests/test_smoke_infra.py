import shutil
import subprocess

import pytest

import hil
from hil.sniffer import ssh_argv

pytestmark = pytest.mark.smoke


def test_preflight(hil_config, test_artifacts):
    report = {
        "hil_version": hil.__version__,
        "ffmpeg": bool(shutil.which("ffmpeg")),
        "uvc_util": hil_config.uvc_util.exists(),
        "state_dir": str(hil_config.state_dir),
    }
    test_artifacts.attach_json("preflight", report)
    assert report["ffmpeg"], "ffmpeg missing (brew install ffmpeg)"


def test_api_alive(api, device_inventory, test_artifacts):
    health = api.health()
    test_artifacts.attach_json("health", health)
    test_artifacts.attach_json("devices", device_inventory)
    assert health.get("status") == "ok"
    addrs = sorted(d["short_address"] for d in device_inventory)
    assert len(addrs) >= 1, "no physical devices discovered"


@pytest.mark.serial
def test_serial_console_alive(api, serial_log):
    with serial_log.window() as win:
        api.health()
        line = win.expect(r"HTTP access: GET /api/v1/health", timeout_s=10)
    assert "-> 200" in line


@pytest.mark.sniffer
def test_sniffer_catches_paced_commands(api, sniffer, paced, state_snapshot,
                                        test_artifacts):
    addr = state_snapshot[0]["short_address"]
    with sniffer.window() as win:
        for i in range(6):
            paced(1.0)
            api.dapc(addr, 80 + i)
        paced(1.0)
        api.cmd(addr, 0xA0)
        win.expect_frame("DAPC short %d" % addr, timeout_s=8)
        try:
            win.expect_frame("backward", timeout_s=8)
        except AssertionError:
            win.expect_monitor("BF8", direction="rx", timeout_s=4)
        stats = win.stats()
    test_artifacts.attach_json("sniffer_stats", stats)
    assert stats["frames_live"] >= 6, stats
    assert stats["counter_missed"] == 0, "ring dropped frames at 1 Hz pacing: %s" % stats


def test_wb_ssh_reachable(hil_config):
    probe = subprocess.run(ssh_argv(hil_config, "true"),
                           capture_output=True, timeout=15)
    assert probe.returncode == 0, probe.stderr.decode()[:200]


def test_camera_lock_state(hil_config, test_artifacts):
    lock_json = hil_config.state_dir / "camera_lock.json"
    if not lock_json.exists():
        pytest.skip("camera never locked on this host (run hil calibrate first)")
    import json
    lock = json.loads(lock_json.read_text())
    test_artifacts.attach_json("camera_lock", lock)
    assert "targets" in lock and "exposure-time-abs" in lock["targets"]
