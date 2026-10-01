import re
import subprocess
import sys
import time
from pathlib import Path

import requests

from hil import benchenv, remote_serial, serialmon, serialport, wb_flash

READY_TIMEOUT_S = 90
OTA_TIMEOUT_S = 300
OTA_POLL_S = 2.0
HEALTH_TIMEOUT_S = 3
BOARD_QUIET_TIMEOUT_S = 15
REQUEST_TIMEOUT_S = 10
POLL_S = 1
BANNER_BOOT_S = 20
READY_ASK_INTERVAL_S = 10

REPO_ROOT = Path(__file__).resolve().parents[3]

BOARDS = {
    "esp32p4": {
        "target": "riscv32imafc-esp-espidf",
        "partition_table": "partitions-p4.csv",
        "mcu": "esp32p4",
        "build_env": {"ESP_IDF_SYS_ROOT_CRATE": "dali2rust-firmware"},
        "cargo_args": ["fw"],
        "flash_size": "32mb",
        "isr_iram_check": True,
    },
}

CONTROLLER, GEAR_SIM = "controller", "gear-sim"

IMAGES = {
    CONTROLLER: {"workspace": ".", "cargo_args": None, "elf": "dali2rust",
                 "bench": True},
    GEAR_SIM: {"workspace": "tools/dali-gear-sim", "cargo_args": ["build"],
               "elf": "dali-gear-sim", "bench": False},
}

VIA_RFC2217, VIA_WB, VIA_OTA = "rfc2217", "wb", "ota"
VIAS = (VIA_RFC2217, VIA_WB, VIA_OTA)

READY_LINE = re.compile(r"# ready build=(\S+) slot=(\S+) state=(\S+)")
BANNER_STATES = {VIA_OTA: ("pending_verify",), VIA_WB: ("valid",), VIA_RFC2217: ("valid",)}

CARGO_CONFIG = ".cargo/config.toml"

BOARD_ENV_KEYS = ("MCU", "ESP_IDF_SDKCONFIG_DEFAULTS")

ISR_IRAM_SCRIPT = "scripts/verify_dali_isr_iram.py"
ISR_IRAM_TOOLS_MISSING, ISR_IRAM_NOTHING_CHECKED = 2, 3
ISR_IRAM_UNCHECKED_EXITS = frozenset({ISR_IRAM_TOOLS_MISSING, ISR_IRAM_NOTHING_CHECKED})
ISR_IRAM_FAILED, ISR_IRAM_UNCHECKED = "failed", "unchecked"
ISR_IRAM_REFUSALS = {
    ISR_IRAM_FAILED: (
        "\nRefusing to flash: the DALI PHY interrupt reaches flash, and it runs\n"
        "with the flash cache off (CONFIG_GPTIMER_ISR_CACHE_SAFE). On the bench\n"
        "that is a Cache error panic during any persistence write, and the panic\n"
        "handler cannot print it — you would see resets with no message.\n"
        "Pass --allow-red-isr to flash it deliberately for diagnosis."),
    ISR_IRAM_UNCHECKED: (
        "\nRefusing to flash: the ISR-IRAM gate could not check the binary — its\n"
        "SKIP or UNCHECKED line above names why: a missing tool or ELF, or entry\n"
        "points or sections it could not find — so nothing proves that the DALI\n"
        "PHY interrupt stays out of flash.\n"
        "Pass --allow-red-isr to flash the unchecked image deliberately."),
}

UI_MIRROR_SCRIPT = "scripts/verify_web_mirror_fresh.sh"


class BoardError(RuntimeError):
    pass


def cargo_env(root: Path = None) -> dict:
    path = (root or REPO_ROOT) / CARGO_CONFIG
    values: dict = {}
    in_env = False
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if line.startswith("["):
            in_env = line == "[env]"
            continue
        if not in_env or not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        value = value.strip()
        if value.startswith("{"):
            inner = value.strip("{}").split(",")[0]
            value = inner.partition("=")[2]
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def board_env(spec, root: Path = None) -> dict:
    values = cargo_env(root)
    knobs = {k: values[k] for k in BOARD_ENV_KEYS if k in values}
    missing = [k for k in BOARD_ENV_KEYS if k not in knobs]
    if missing:
        raise BoardError(
            "%s has no %s in its [env] block — those decide which architecture "
            "is built, and without them a build inherits whatever the shell "
            "holds." % (CARGO_CONFIG, ", ".join(missing)))
    if knobs["MCU"] != spec["mcu"]:
        raise BoardError(
            "%s builds MCU=%s but board %r expects %s — one of the two moved "
            "without the other. Building for the wrong MCU silently produces an "
            "image for the wrong die."
            % (CARGO_CONFIG, knobs["MCU"], spec.get("name", "?"), spec["mcu"]))
    return knobs


def board_spec(cfg, image=CONTROLLER):
    spec = BOARDS.get(cfg.board)
    if spec is None:
        raise BoardError(
            "unknown HIL_BOARD %r — known boards: %s"
            % (cfg.board, ", ".join(sorted(BOARDS))))
    recipe = IMAGES.get(image)
    if recipe is None:
        raise BoardError("unknown image %r — known images: %s"
                         % (image, ", ".join(sorted(IMAGES))))
    target = spec["target"]
    spec = dict(spec, name=cfg.board)
    workspace = recipe["workspace"]
    prefix = "" if workspace == "." else workspace + "/"
    return dict(
        spec,
        image=image,
        workspace=workspace,
        bench=recipe["bench"],
        cargo_args=recipe["cargo_args"] or spec["cargo_args"],
        build_env=dict(spec["build_env"]) if recipe["bench"] else {},
        board_env=board_env(spec, REPO_ROOT / workspace),
        firmware_bin="%starget/%s/debug/%s" % (prefix, target, recipe["elf"]),
        bootloader="target/%s/debug/bootloader.bin" % target,
        merged_bin="target/%s/debug/%s-merged.bin" % (target, recipe["elf"]),
        app_bin="target/%s/debug/%s-app.bin" % (target, recipe["elf"]),
    )


def repo_root(cfg) -> Path:
    return Path(cfg.root).parent.parent


def build(cfg, allow_nonbench=False, spec=None):
    spec = spec or board_spec(cfg)
    if not spec["bench"]:
        print("cargo build of the %s image (%s)" % (spec["image"], spec["workspace"]),
              flush=True)
        return subprocess.run(["cargo", *spec["cargo_args"]],
                              cwd=repo_root(cfg) / spec["workspace"]).returncode
    env = benchenv.resolve(Path(cfg.root))
    env.update(spec["build_env"])
    problems = benchenv.validate(env)
    if problems:
        for problem in problems:
            print("bench env: %s" % problem, file=sys.stderr)
        if not allow_nonbench:
            print(
                "\nRefusing to build a non-bench firmware. Copy %s to %s and set the\n"
                "required knobs, or pass --allow-nonbench-build to proceed deliberately."
                % (benchenv.BENCH_ENV_EXAMPLE, benchenv.BENCH_ENV_FILE),
                file=sys.stderr,
            )
            return 2
        print("bench env: proceeding anyway (--allow-nonbench-build)", file=sys.stderr)

    knobs = benchenv.effective_knobs(env)
    print("cargo build for %s (%s) with %d bench knob(s):"
          % (spec["name"], spec["target"], len(knobs)), flush=True)
    for key in sorted(knobs):
        print("  %s=%s" % (key, knobs[key]), flush=True)
    for key in sorted(spec["board_env"]):
        print("  %s=%s" % (key, spec["board_env"][key]), flush=True)
    return subprocess.run(["cargo", *spec["cargo_args"]],
                          cwd=repo_root(cfg), env=env).returncode


def check_isr_iram(cfg, spec, allow_red=False):
    if not spec.get("isr_iram_check"):
        return "skipped", 0
    rc = subprocess.run(
        [sys.executable, ISR_IRAM_SCRIPT, "--require-tools", spec["firmware_bin"]],
        cwd=repo_root(cfg),
    ).returncode
    if rc == 0:
        return "ok", 0
    verdict = ISR_IRAM_UNCHECKED if rc in ISR_IRAM_UNCHECKED_EXITS else ISR_IRAM_FAILED
    if allow_red:
        print("isr-iram: %s, proceeding anyway (--allow-red-isr)" % verdict, file=sys.stderr)
        return verdict, 0
    print(ISR_IRAM_REFUSALS[verdict], file=sys.stderr)
    return verdict, 1


def merged_image(spec, root: Path) -> Path:
    if not (root / spec["bootloader"]).is_file():
        raise BoardError(
            "%s is missing: every image on this board is merged with the controller's "
            "bootloader, so build the controller once (`hil flash --build-only`)"
            % spec["bootloader"])
    out = root / spec["merged_bin"]
    rc = subprocess.run(
        ["espflash", "save-image", "--chip", spec["mcu"],
         "--flash-size", spec["flash_size"], "--merge", "--skip-padding",
         "--bootloader", spec["bootloader"],
         "--partition-table", spec["partition_table"],
         spec["firmware_bin"], str(out)],
        cwd=root).returncode
    if rc != 0:
        raise RuntimeError("espflash save-image failed (rc=%d)" % rc)
    return out


def app_image(spec, root: Path) -> Path:
    out = root / spec["app_bin"]
    rc = subprocess.run(
        ["espflash", "save-image", "--chip", spec["mcu"],
         "--flash-size", spec["flash_size"], spec["firmware_bin"], str(out)],
        cwd=root).returncode
    if rc != 0:
        raise RuntimeError("espflash save-image failed (rc=%d)" % rc)
    return out


def health(base):
    try:
        r = requests.get(base.rstrip("/") + "/api/v1/health", timeout=HEALTH_TIMEOUT_S)
        return r.json() if r.ok else None
    except (requests.RequestException, ValueError):
        return None


def board_proof(cfg):
    target_answered = bool(cfg.base) and health(cfg.base) is not None
    other = _other_board(cfg)
    other_before = health(other.base) if other is not None else None
    if not target_answered and other_before is None:
        raise BoardError("nothing can tell which board %s resets: %s answers no HTTP and the "
                         "other board cannot be reached" % (cfg.serial_remote, cfg.base or
                                                            "the target"))

    def check():
        if target_answered and not _goes_quiet(cfg.base):
            raise BoardError(
                "the bridge named by %s put a board into its bootloader, yet %s still "
                "answers: the port and the URL name different boards"
                % (cfg.serial_remote, cfg.base))
        if other_before is None:
            return
        other_after = health(other.base)
        if other_after is None or \
                other_after.get("uptime_seconds", 0) < other_before.get("uptime_seconds", 0):
            raise BoardError("%s stopped or restarted when %s was put into its bootloader: "
                             "the port names the wrong board" % (other.base, cfg.base))
    return check


def _other_board(cfg):
    try:
        other = cfg.peer()
    except Exception:
        return None
    return other if other.base else None


def _goes_quiet(base, timeout_s=BOARD_QUIET_TIMEOUT_S):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if health(base) is None:
            return True
        time.sleep(POLL_S)
    return False


def deliver_ota(cfg, spec, root: Path) -> int:
    image = app_image(spec, root)
    with wb_flash.serve(cfg, image) as url:
        r = requests.post(cfg.base.rstrip("/") + "/api/v1/firmware/updates",
                          json={"url": url}, timeout=REQUEST_TIMEOUT_S)
        if r.status_code != 202:
            print("the board refused the update: %d %s" % (r.status_code, r.text[:200]),
                  file=sys.stderr)
            return 1
        return await_update(cfg.base)


def await_update(base, timeout_s=OTA_TIMEOUT_S):
    deadline = time.monotonic() + timeout_s
    seen = None
    while time.monotonic() < deadline:
        try:
            update = requests.get(base.rstrip("/") + "/api/v1/firmware",
                                  timeout=HEALTH_TIMEOUT_S).json().get("update") or {}
        except (requests.RequestException, ValueError):
            if seen in ("finishing", "ready_to_reboot"):
                return 0
            update = {}
        state = update.get("state")
        if state == "failed":
            print("the update failed: %s" % update.get("error"), file=sys.stderr)
            return 1
        if state == "ready_to_reboot":
            return 0
        seen = state or seen
        time.sleep(OTA_POLL_S)
    print("the update did not finish within %ds" % timeout_s, file=sys.stderr)
    return 1


def deliver(cfg, spec, root: Path, via, serial_port) -> int:
    if via == VIA_OTA:
        return deliver_ota(cfg, spec, root)
    if via == VIA_WB:
        return wb_flash.write(cfg, merged_image(spec, root), before_write=board_proof(cfg))
    if remote_serial.enabled(cfg):
        return write_remote(cfg, spec, root)
    return write_local(spec, root, serial_port, cfg.flash_baud)


def wait_banner(cfg, since: int, timeout_s=READY_TIMEOUT_S):
    deadline = time.monotonic() + timeout_s
    found = _poll_banner(cfg, since, BANNER_BOOT_S)
    while found is None and remote_serial.enabled(cfg) and time.monotonic() < deadline:
        try:
            remote_serial.control(cfg, "write ready")
        except (OSError, remote_serial.RemoteError) as exc:
            print("asking the emulator for its ready line failed: %s" % exc, file=sys.stderr)
            return None
        found = _poll_banner(cfg, since, min(READY_ASK_INTERVAL_S, deadline - time.monotonic()))
    return found


def _poll_banner(cfg, since, timeout_s):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        found = ready_line(cfg, since)
        if found:
            return found
        time.sleep(POLL_S)
    return None


def ready_line(cfg, since: int):
    log = serialmon.log_path(cfg)
    if not log or not log.exists():
        return None
    found = None
    with open(log, "r", errors="replace") as fh:
        fh.seek(since)
        for line in fh:
            match = READY_LINE.search(line)
            if match:
                found = match.groups()
    return found


def verify_banner(cfg, since, via, root) -> int:
    found = wait_banner(cfg, since)
    if found is None:
        print("the emulator printed no ready line within %ds — check %s"
              % (READY_TIMEOUT_S, serialmon.log_path(cfg)), file=sys.stderr)
        return 1
    build_id, slot, state = found
    head = benchenv.head_commit(root)
    if head and not head.startswith(build_id.partition(".")[0]):
        print("the emulator reports build %s, not the commit just built (%s)"
              % (build_id, head[:8]), file=sys.stderr)
        return 1
    want = BANNER_STATES[via]
    if state not in want:
        print("the emulator runs %s in state %r; a %s delivery must give %s%s"
              % (slot, state, via, " or ".join(repr(s) for s in want),
                 " — the bootloader kept the image: `hil --peer role controller --via wb` "
                 "writes the controller back" if state == "new" else ""), file=sys.stderr)
        return 1
    print("emulator ready: build %s in %s, state %s" % (build_id, slot, state))
    return 0


def write_remote(cfg, spec, root: Path) -> int:
    image = merged_image(spec, root)
    try:
        remote_serial.control(cfg, "bootloader")
        return subprocess.run(
            [sys.executable, "-m", "esptool",
             "--chip", spec["mcu"], "--port", remote_serial.data_url(cfg),
             "--before", "no_reset", "--after", "no_reset",
             "--baud", str(cfg.flash_baud),
             "write_flash", "0x0", str(image)],
            cwd=root).returncode
    finally:
        remote_serial.control(cfg, "run")


def write_local(spec, root: Path, serial_port: str, baud: int) -> int:
    return subprocess.run(
        ["espflash", "flash", "--port", serial_port, "--baud", str(baud),
         "--bootloader", spec["bootloader"],
         "--partition-table", spec["partition_table"],
         spec["firmware_bin"]],
        cwd=root).returncode


def learn_base(cfg, since: int, timeout_s=READY_TIMEOUT_S):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        address = serialmon.announced_address(cfg, since=since)
        if address:
            return "http://%s" % address
        time.sleep(1)
    return None


def record_base(cfg, base: str):
    path = cfg.base_pin_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(base + "\n")
    print("base recorded: %s -> %s" % (base, path), flush=True)


def wait_ready(cfg, timeout_s=READY_TIMEOUT_S, base=None):
    base = base or cfg.base
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            r = requests.get(base + "/api/v1/health", timeout=2)
            if r.ok:
                health = r.json()
                print("DUT ready: %s" % health)
                return health
        except requests.RequestException:
            pass
        time.sleep(1)
    return None


def serial_port_for_flash(cfg):
    if remote_serial.enabled(cfg):
        remote_serial.ensure(cfg)
        port = remote_serial.data_url(cfg)
        print("serial port: %s (WB bridge at %s)"
              % (port, remote_serial.target(cfg)), flush=True)
        return port
    port, pinned = serialmon.effective_port(cfg)
    if pinned:
        if cfg.serial_port_pinned:
            serialmon.record_pin(cfg, port)
            why = "pinned via HIL_SERIAL_PORT"
        else:
            why = "pinned by %s" % serialmon.pin_path(cfg)
    else:
        ports = serialport.candidates()
        if len(ports) > 1:
            print(
                "Refusing to guess a serial port: %d board ports are attached "
                "(%s) and none is pinned. Set HIL_SERIAL_PORT to the DUT's "
                "port — one pinned run records it in %s and every later flash "
                "and monitor restart inherits it."
                % (len(ports), ", ".join(ports), serialmon.pin_path(cfg)),
                file=sys.stderr)
            return None
        why = "auto-discovered"
    print("serial port: %s (%s)" % (port, why), flush=True)
    return port


def check_ui_mirror(cfg, allow_stale=False):
    rc = subprocess.run(["bash", UI_MIRROR_SCRIPT], cwd=repo_root(cfg)).returncode
    if rc == 0:
        return 0
    if allow_stale:
        print("ui-mirror: building with the previous UI bundle (--allow-stale-ui)",
              file=sys.stderr)
        return 0
    print(
        "\nRefusing to build: the embedded web UI is older than web/app, so the\n"
        "image would carry the previous UI. Run `bash scripts/build_web_ui.sh` and\n"
        "land the rebuilt bundle through a pull request first. Pass --allow-stale-ui\n"
        "to build with the previous UI deliberately.",
        file=sys.stderr,
    )
    return 1


def run(cfg, build_only=False, allow_nonbench=False, allow_red_isr=False,
        allow_stale_ui=False, image=CONTROLLER, via=VIA_RFC2217, on_delivered=None):
    spec, serial_port, rc = _prepare(cfg, image, via, build_only, allow_stale_ui)
    if rc is not None:
        return rc
    rc = build(cfg, allow_nonbench=allow_nonbench, spec=spec)
    if rc != 0 or build_only:
        return rc
    isr_iram, rc = check_isr_iram(cfg, spec, allow_red=allow_red_isr)
    if rc != 0:
        return rc
    root = repo_root(cfg)
    manifest = _write_manifest(cfg, spec, root, via, serial_port, isr_iram)
    log_mark = serialmon.log_size(cfg)
    rc = _deliver_around_monitor(cfg, spec, root, via, serial_port)
    if rc != 0:
        return rc
    if on_delivered is not None:
        on_delivered()
    if not spec["bench"]:
        return verify_banner(cfg, log_mark, via, root)
    return _verify_controller(cfg, spec, root, via, manifest, log_mark)


def require_standby_peer(cfg):
    if not cfg.is_peer:
        raise BoardError("the gear emulator goes only onto the pair's other board: "
                         "`hil --peer role gear-sim`")
    found = health(cfg.base) if cfg.base else None
    if not found or found.get("role") != "standby":
        raise BoardError("%s is not a standby controller (%r): the emulator never replaces "
                         "the controller that holds the wire" % (cfg.base, found))


def _prepare(cfg, image, via, build_only, allow_stale_ui):
    if via not in VIAS:
        raise BoardError("unknown delivery %r — known: %s" % (via, ", ".join(VIAS)))
    if image == GEAR_SIM:
        require_standby_peer(cfg)
    if via == VIA_WB and not remote_serial.enabled(cfg):
        raise BoardError("--via wb writes through the board's bridge on the WB, and none "
                         "is named (HIL_SERIAL_REMOTE, HIL_PEER_SERIAL_REMOTE)")
    if via == VIA_OTA and not cfg.base:
        raise BoardError("--via ota posts to the board's HTTP address, and none is known")
    if image == CONTROLLER and check_ui_mirror(cfg, allow_stale=allow_stale_ui) != 0:
        return None, None, 1
    spec = board_spec(cfg, image)
    if build_only or via == VIA_OTA:
        return spec, None, None
    serial_port = serial_port_for_flash(cfg)
    if serial_port is None:
        return spec, None, 2
    if not cfg.base and not serialmon.alive(cfg):
        print("no base URL for this board and no serial monitor to learn one from: start "
              "`hil %smonitor start` first, or set HIL_PEER_BASE"
              % ("--peer " if cfg.is_peer else ""), file=sys.stderr)
        return spec, None, 2
    return spec, serial_port, None


def _write_manifest(cfg, spec, root, via, serial_port, isr_iram):
    env = benchenv.resolve(Path(cfg.root))
    manifest = benchenv.write_manifest(
        cfg.new_run_dir(), root, env, Path(spec["firmware_bin"]), spec["target"],
        bench_valid=not benchenv.validate(env) if spec["bench"] else True,
        board_env=spec["board_env"], checks={"isr_iram": isr_iram},
        transport=_transport(cfg, via, serial_port), image=spec["image"])
    print("run manifest: %s" % manifest, flush=True)
    return manifest


def _verify_controller(cfg, spec, root, via, manifest, log_mark):
    base = cfg.base or learn_base(cfg, since=log_mark)
    if base is None:
        print("the board announced no lease within %ds — check the serial log (%s)"
              % (READY_TIMEOUT_S, serialmon.log_path(cfg)), file=sys.stderr)
        return 1
    if not cfg.base:
        record_base(cfg, base)
    if via == VIA_OTA:
        _goes_quiet(base)
    found = wait_ready(cfg, base=base)
    if found is None:
        print("DUT did not come back within %ds — check serial log" % READY_TIMEOUT_S,
              file=sys.stderr)
        return 1
    built = spec["app_bin"] if via == VIA_OTA else spec["merged_bin"]
    return verify_running_version(found, root, manifest, image=root / built)


def _transport(cfg, via, serial_port):
    if via == VIA_OTA:
        return "ota %s served from %s" % (cfg.base, cfg.wb_ssh)
    if via == VIA_WB:
        return "wb-local esptool %s" % remote_serial.target(cfg)
    if remote_serial.enabled(cfg):
        return "wb-bridge %s" % remote_serial.target(cfg)
    return "local %s" % serial_port


def _deliver_around_monitor(cfg, spec, root, via, serial_port):
    monitor_was_alive = serialmon.alive(cfg)
    holds_port = via != VIA_OTA and monitor_was_alive
    if holds_port:
        serialmon.stop(cfg)
        time.sleep(1)
    sys.stdout.flush()
    try:
        return deliver(cfg, spec, root, via, serial_port)
    finally:
        if holds_port:
            serialmon.start(cfg)


IMAGE_VERSION = re.compile(
    rb"[0-9]+\.[0-9]+\.[0-9]+\+(?:[0-9a-f]{6,}(?:\.dirty(?:\.[0-9]{4}T[0-9]{4})?)?|unknown)"
)


def image_version(path):
    try:
        blob = Path(path).read_bytes()
    except OSError:
        return None
    found = {m.decode() for m in IMAGE_VERSION.findall(blob)}
    return found.pop() if len(found) == 1 else None


def verify_running_version(health, root, manifest, image=None):
    reported = str(health.get("version", ""))
    benchenv.record_boot_version(manifest, reported)
    built = image_version(image) if image else None
    if built:
        if reported == built:
            print("running version: %s (matches the image just written)" % reported)
            return 0
        print("DUT reports %r, but the image just written carries %r. The write "
              "did not take, the board booted its other slot, or this is not the "
              "board that was written." % (reported, built), file=sys.stderr)
        return 1
    if image:
        print("could not read a version out of %s — falling back to the weaker "
              "HEAD comparison, which passes any image built from this commit "
              "(tools/hil/README.md §Flashing)" % image)
    identity = reported.partition("+")[2].partition(".")[0]
    head = benchenv.head_commit(root)
    if identity == "unknown":
        print("firmware version %r carries no commit — not verified" % reported)
        return 0
    if not identity:
        print("DUT reports %r, which names no commit at all — an image built "
              "before the version was derived from git, so it is not the commit "
              "just built (%s). The write did not take, the board booted its "
              "other slot, or this is not the board that was written."
              % (reported, head[:8] or "unknown"), file=sys.stderr)
        return 1
    if not head:
        print("git could not name HEAD — running version %r not verified" % reported)
        return 0
    if not head.startswith(identity):
        print("DUT reports %r, which is not the commit just built (%s). The write "
              "did not take, the board booted its other slot, or this is not the "
              "board that was written." % (reported, head[:8]), file=sys.stderr)
        return 1
    print("running version: %s" % reported)
    return 0
