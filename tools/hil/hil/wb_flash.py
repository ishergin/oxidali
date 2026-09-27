import contextlib
import hashlib
import importlib.util
import os
import subprocess
import time
from pathlib import Path

import requests

from hil import remote_serial

REMOTE_TOOLS = remote_serial.REMOTE_DIR + "/esptool"
REMOTE_IMAGES = remote_serial.REMOTE_DIR + "/images"
TOOL_PACKAGES = ("esptool", "intelhex")
TOOLS_DIGEST_FILE = ".digest"

WB_FLASH_BAUD = 921600
WRITE_TIMEOUT_S = 600
WRITE_POLL_S = 5
WRITE_START_TIMEOUT_S = 30
COPY_TIMEOUT_S = 180
WRITE_STEM = REMOTE_IMAGES + "/write-%d"
WRITE_LOG_TAIL = 15
BRIDGE_VERBS = "verbs="
RELEASE_VERB = "release"

OTA_HTTP_PORT = 8765
OTA_SERVER_START_TIMEOUT_S = 15
OTA_SERVER_POLL_S = 0.5


class WbFlashError(RuntimeError):
    pass


def _ssh(tgt, command, timeout=60, **kwargs):
    return subprocess.run(["ssh", *remote_serial.SSH_OPTS, tgt.ssh, command],
                          timeout=timeout, **kwargs)


def _site_packages() -> Path:
    found = importlib.util.find_spec("esptool")
    if found is None or found.origin is None:
        raise WbFlashError("esptool is not installed in the toolkit's venv")
    return Path(found.origin).resolve().parent.parent


def tools_digest(site: Path) -> str:
    digest = hashlib.sha256()
    for package in TOOL_PACKAGES:
        for path in sorted((site / package).rglob("*.py")):
            digest.update(str(path.relative_to(site)).encode())
            digest.update(path.read_bytes())
    return digest.hexdigest()


def ensure_tools(tgt) -> bool:
    site = _site_packages()
    want = tools_digest(site)
    have = _ssh(tgt, "cat %s/%s 2>/dev/null" % (REMOTE_TOOLS, TOOLS_DIGEST_FILE),
                capture_output=True, text=True).stdout.strip()
    if have == want:
        return False
    tar = subprocess.run(
        ["tar", "-C", str(site), "--exclude", "__pycache__", "-cf", "-", *TOOL_PACKAGES],
        capture_output=True, env=dict(os.environ, COPYFILE_DISABLE="1"), check=True)
    unpack = ("rm -rf {d} && mkdir -p {d} && tar -C {d} -xf - && echo {w} > {d}/{f}"
              .format(d=REMOTE_TOOLS, w=want, f=TOOLS_DIGEST_FILE))
    result = _ssh(tgt, unpack, timeout=COPY_TIMEOUT_S, input=tar.stdout,
                  capture_output=True)
    if result.returncode != 0:
        raise WbFlashError("copying esptool to %s failed: %s"
                           % (tgt.ssh, result.stderr.decode(errors="replace").strip()))
    return True


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stage(tgt, image: Path) -> str:
    remote = "%s/%s" % (REMOTE_IMAGES, image.name)
    _ssh(tgt, "mkdir -p %s" % REMOTE_IMAGES, check=True)
    copied = subprocess.run(["scp", *remote_serial.SSH_OPTS, "-q", str(image),
                             "%s:%s" % (tgt.ssh, remote)],
                            capture_output=True, text=True, timeout=COPY_TIMEOUT_S)
    if copied.returncode != 0:
        raise WbFlashError("copying %s to %s failed: %s"
                           % (image.name, tgt.ssh, copied.stderr.strip()))
    have = _ssh(tgt, "sha256sum %s | cut -d' ' -f1" % remote,
                capture_output=True, text=True).stdout.strip()
    if have != _sha256(image):
        unstage(tgt, remote)
        raise WbFlashError("%s arrived on %s with a different sha256" % (image.name, tgt.ssh))
    return remote


def unstage(tgt, remote: str):
    _ssh(tgt, "rm -f %s" % remote)


def write_command(device: str, remote_image: str, baud: int = WB_FLASH_BAUD) -> str:
    return ("PYTHONPATH={tools} python3 -m esptool --chip esp32p4 --port {port} "
            "--baud {baud} --before no_reset --after no_reset write_flash 0x0 {image}"
            .format(tools=REMOTE_TOOLS, port=device, baud=baud, image=remote_image))


def write(cfg, image: Path, before_write=None, log=print) -> int:
    tgt = remote_serial.target(cfg)
    remote_serial.resolve_device(tgt)
    require_release(cfg)
    if ensure_tools(tgt):
        log("wb flash: esptool copied to %s:%s" % (tgt.ssh, REMOTE_TOOLS))
    remote = stage(tgt, image)
    try:
        enter_loader(cfg, before_write)
        try:
            log("wb flash: writing %s on %s through %s" % (image.name, tgt.ssh, tgt.device))
            return run_detached(tgt, write_command(tgt.device, remote), log)
        finally:
            remote_serial.control(cfg, "reacquire")
    finally:
        unstage(tgt, remote)


def require_release(cfg):
    reply = remote_serial.control(cfg, "status")
    if "released" in reply.split():
        raise WbFlashError("the bridge's port is released, so a write did not finish: "
                           "`hil %sremote reacquire` opens it again (and resets the board)"
                           % _peer_flag(cfg))
    verbs = reply.partition(BRIDGE_VERBS)[2].split(" ")[0].split(",")
    if RELEASE_VERB not in verbs:
        raise WbFlashError("the bridge on the WB cannot release its port (%s): `hil "
                           "%sremote start --restart` runs the current bridge, which resets "
                           "the board" % (reply, _peer_flag(cfg)))


def enter_loader(cfg, before_write=None):
    remote_serial.control(cfg, "bootloader")
    try:
        if before_write is not None:
            before_write()
        remote_serial.control(cfg, "release")
    except Exception:
        remote_serial.control(cfg, "run")
        raise


def run_detached(tgt, command, log=print, timeout_s=WRITE_TIMEOUT_S) -> int:
    stem = WRITE_STEM % time.time_ns()
    rc_file, log_file = stem + ".rc", stem + ".log"
    start = ("nohup setsid sh -c '{cmd} > {log} 2>&1; echo $? > {rc}' "
             "< /dev/null > /dev/null 2>&1 &").format(cmd=command, log=log_file, rc=rc_file)
    try:
        started = _ssh(tgt, start, timeout=WRITE_START_TIMEOUT_S)
    except subprocess.TimeoutExpired:
        log("wb flash: the ssh that started esptool hung; polling for its result")
    else:
        if started.returncode != 0:
            raise WbFlashError("ssh did not start esptool on %s (exit %d)"
                               % (tgt.ssh, started.returncode))
    rc = _await_rc(tgt, rc_file, timeout_s)
    if rc != 0:
        log(_ssh(tgt, "tail -%d %s" % (WRITE_LOG_TAIL, log_file),
                 capture_output=True, text=True).stdout)
    _ssh(tgt, "rm -f %s %s" % (rc_file, log_file))
    return rc


def _await_rc(tgt, rc_file, timeout_s):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        time.sleep(WRITE_POLL_S)
        rc = _remote_rc(tgt, rc_file)
        if rc is not None:
            return rc
    raise WbFlashError("esptool did not finish within %ds on %s: see %s there"
                       % (timeout_s, tgt.ssh, rc_file.replace(".rc", ".log")))


def _remote_rc(tgt, rc_file):
    try:
        out = _ssh(tgt, "cat %s 2>/dev/null" % rc_file, capture_output=True, text=True)
    except subprocess.TimeoutExpired:
        return None
    text = out.stdout.strip()
    return int(text) if text.lstrip("-").isdigit() else None


def _peer_flag(cfg) -> str:
    return "--peer " if cfg.is_peer else ""


def _await_served(url, timeout_s=OTA_SERVER_START_TIMEOUT_S):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            if requests.head(url, timeout=OTA_SERVER_POLL_S * 4).ok:
                return
        except requests.RequestException:
            pass
        time.sleep(OTA_SERVER_POLL_S)
    raise WbFlashError("%s is not served within %ds" % (url, timeout_s))


def wb_host(cfg) -> str:
    return cfg.wb_ssh.rpartition("@")[2]


@contextlib.contextmanager
def serve(cfg, image: Path):
    tgt = remote_serial.Target("%s:/dev/null" % cfg.wb_ssh)
    remote = stage(tgt, image)
    host = wb_host(cfg)
    server = subprocess.Popen(
        ["ssh", *remote_serial.SSH_OPTS, tgt.ssh,
         "cd %s && exec python3 -m http.server %d --bind %s"
         % (REMOTE_IMAGES, OTA_HTTP_PORT, host)],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=True)
    url = "http://%s:%d/%s" % (host, OTA_HTTP_PORT, image.name)
    try:
        _await_served(url)
        yield url
    finally:
        server.terminate()
        _ssh(tgt, "fuser -k -n tcp %d 2>/dev/null; true" % OTA_HTTP_PORT)
        unstage(tgt, remote)
