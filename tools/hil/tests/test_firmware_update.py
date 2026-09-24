import socket
import subprocess
from functools import partial
import threading
import time
from http.server import HTTPServer, SimpleHTTPRequestHandler
from pathlib import Path

import pytest

from hil import pair
from hil.wait import wait_until

UPDATE_TIMEOUT_S = 240.0
REBOOT_TIMEOUT_S = 120.0
VERIFY_TIMEOUT_S = 120.0
POLLER_PROOF_S = 12.0
POLLER_INTERVAL_MS = 1000


def _repo_root(hil_config) -> Path:
    return Path(hil_config.root).parent.parent


def _build_app_image(hil_config, out_dir: Path) -> Path:
    root = _repo_root(hil_config)
    elf = root / "target/riscv32imafc-esp-espidf/debug/dali2rust"
    if not elf.is_file():
        pytest.skip("no firmware ELF built — run `hil flash` or `cargo fw` first")
    image = out_dir / "dali2rust.bin"
    result = subprocess.run(
        ["espflash", "save-image", "--chip", "esp32p4", "--flash-size", "32mb",
         "--partition-table", "partitions-p4.csv", str(elf), str(image)],
        cwd=root, capture_output=True, text=True)
    if result.returncode != 0:
        pytest.skip("espflash save-image failed: %s" % result.stderr.strip()[:200])
    return image


def _address_the_dut_can_reach(hil_config) -> str:
    host = hil_config.base.split("//", 1)[-1].split(":")[0].split("/")[0]
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.connect((host, 80))
        return probe.getsockname()[0]


class _QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


class _ImageServer:
    def __init__(self, directory: Path):
        handler = partial(_QuietHandler, directory=str(directory))
        self._httpd = HTTPServer(("0.0.0.0", 0), handler)
        self.port = self._httpd.server_address[1]

    def __enter__(self):
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, *_exc):
        self._httpd.shutdown()
        self._httpd.server_close()


def _poller_reads(api) -> int:
    return _poller_reads_and_uptime(api)[0]


def _poller_reads_and_uptime(api):
    diag = api.diagnostics()
    poller = diag.get("poller", {})
    reads = int(poller.get("reads_published", 0)) + int(poller.get("health_probes_published", 0))
    return reads, int(diag.get("uptime_ms", 0))


@pytest.mark.hil_id("HIL-OTA-01")
@pytest.mark.destructive
@pytest.mark.slow
def test_network_update_boots_the_other_slot_and_holds_background_work(
        api, hil_config, poller_guard, test_artifacts, tmp_path, peer_api_if_any):
    before = api.firmware.get()
    if not before["ota_capable"]:
        pytest.skip("this build has no second app slot (ota_capable=false)")
    assert before["update"]["state"] in ("idle", "failed"), \
        "an update is already in flight; the bench is not in a state to test one"

    image = _build_app_image(hil_config, tmp_path)
    lamps_before = len(api.vlamps.list()["virtual_lamps"])

    poller_guard(enabled=True, interval_ms=POLLER_INTERVAL_MS)
    moving_from = _poller_reads(api)
    assert wait_until(lambda: _poller_reads(api) > moving_from, POLLER_PROOF_S,
                      interval_s=1.0, desc="the poller to publish on the wire"), \
        "the poller published nothing — the hold assertion would prove nothing"

    with _ImageServer(image.parent) as server, api.expect_reboot():
        url = "http://%s:%d/%s" % (
            _address_the_dut_can_reach(hil_config), server.port, image.name)
        accepted = api.firmware.update(url)
        assert accepted["status"] == "accepted"

        held_from, up_from = _poller_reads_and_uptime(api)
        wait_until(lambda: api.firmware.get()["update"]["state"] != "downloading",
                   UPDATE_TIMEOUT_S, interval_s=2.0, desc="the download to finish")
        held_to, up_to = _poller_reads_and_uptime(api)
    assert up_to >= up_from, (
        "the hold was sampled across the reboot (uptime %d -> %d ms): the closing "
        "read landed in the new boot, so it cannot judge D7 — re-run" % (up_from, up_to))
    assert held_to == held_from, (
        "background work reached the wire during the write: poller publishes "
        "%d -> %d (ADR-024 D7)" % (held_from, held_to))

    with api.expect_reboot():
        came_back = wait_until(
            lambda: _slot_or_none(api) not in (None, before["running_slot"]),
            REBOOT_TIMEOUT_S, interval_s=3.0,
            desc="the controller to come back on the other slot")
    assert came_back, "the controller did not boot the image it just wrote"
    pair.hand_back(api, peer_api_if_any)

    after = api.firmware.get()
    assert after["running_slot"] != before["running_slot"]
    assert wait_until(lambda: api.firmware.get()["pending_verify"] is False,
                      VERIFY_TIMEOUT_S, interval_s=5.0,
                      desc="the rollback window to close"), \
        "the image never proved itself — the next reset rolls it back"
    assert len(api.vlamps.list()["virtual_lamps"]) == lamps_before, \
        "the registry did not survive the update"


def _slot_or_none(api):
    try:
        return api.firmware.get()["running_slot"]
    except Exception:
        return None
