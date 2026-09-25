import re
import subprocess
import sys
import time
from pathlib import Path

import requests

from hil import benchenv, remote_serial, serialmon, serialport

READY_TIMEOUT_S = 90

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

CARGO_CONFIG = ".cargo/config.toml"

BOARD_ENV_KEYS = ("MCU", "ESP_IDF_SDKCONFIG_DEFAULTS")

ISR_IRAM_SCRIPT = "scripts/verify_dali_isr_iram.py"

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


def board_spec(cfg):
    spec = BOARDS.get(cfg.board)
    if spec is None:
        raise BoardError(
            "unknown HIL_BOARD %r — known boards: %s"
            % (cfg.board, ", ".join(sorted(BOARDS))))
    target = spec["target"]
    spec = dict(spec, name=cfg.board)
    return dict(
        spec,
        build_env=dict(spec["build_env"]),
        board_env=board_env(spec),
        firmware_bin="target/%s/debug/dali2rust" % target,
        bootloader="target/%s/debug/bootloader.bin" % target,
        merged_bin="target/%s/debug/dali2rust-merged.bin" % target,
    )


def repo_root(cfg) -> Path:
    return Path(cfg.root).parent.parent


def build(cfg, allow_nonbench=False, spec=None):
    spec = spec or board_spec(cfg)
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
    if allow_red:
        print("isr-iram: proceeding anyway (--allow-red-isr)", file=sys.stderr)
        return "failed", 0
    print(
        "\nRefusing to flash: the DALI PHY interrupt reaches flash, and it runs\n"
        "with the flash cache off (CONFIG_GPTIMER_ISR_CACHE_SAFE). On the bench\n"
        "that is a Cache error panic during any persistence write, and the panic\n"
        "handler cannot print it — you would see resets with no message.\n"
        "Pass --allow-red-isr to flash it deliberately for diagnosis.",
        file=sys.stderr,
    )
    return "failed", 1


def merged_image(spec, root: Path) -> Path:
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
        allow_stale_ui=False):
    if check_ui_mirror(cfg, allow_stale=allow_stale_ui) != 0:
        return 1
    spec = board_spec(cfg)
    serial_port = None
    if not build_only:
        serial_port = serial_port_for_flash(cfg)
        if serial_port is None:
            return 2
        if not cfg.base and not serialmon.alive(cfg):
            print("no base URL for this board and no serial monitor to learn "
                  "one from: start `hil %smonitor start` first, or set "
                  "HIL_PEER_BASE" % ("--peer " if cfg.is_peer else ""),
                  file=sys.stderr)
            return 2
    rc = build(cfg, allow_nonbench=allow_nonbench, spec=spec)
    if rc != 0 or build_only:
        return rc
    isr_iram, rc = check_isr_iram(cfg, spec, allow_red=allow_red_isr)
    if rc != 0:
        return rc
    root = repo_root(cfg)
    env = benchenv.resolve(Path(cfg.root))
    manifest = benchenv.write_manifest(
        cfg.new_run_dir(), root, env, Path(spec["firmware_bin"]), spec["target"],
        bench_valid=not benchenv.validate(env),
        board_env=spec["board_env"],
        checks={"isr_iram": isr_iram},
        transport=("wb-bridge %s" % remote_serial.target(cfg)
                   if remote_serial.enabled(cfg) else "local %s" % serial_port),
    )
    print("run manifest: %s" % manifest, flush=True)
    monitor_was_alive = serialmon.alive(cfg)
    log_mark = serialmon.log_size(cfg)
    if monitor_was_alive:
        serialmon.stop(cfg)
        time.sleep(1)
    sys.stdout.flush()
    try:
        rc = (write_remote(cfg, spec, root) if remote_serial.enabled(cfg)
              else write_local(spec, root, serial_port, cfg.flash_baud))
        if rc != 0:
            return rc
    finally:
        if monitor_was_alive:
            serialmon.start(cfg)
    base = cfg.base
    if not base:
        base = learn_base(cfg, since=log_mark)
        if base is None:
            print("the board announced no lease within %ds — check the serial "
                  "log (%s)" % (READY_TIMEOUT_S, serialmon.log_path(cfg)),
                  file=sys.stderr)
            return 1
        record_base(cfg, base)
    health = wait_ready(cfg, base=base)
    if health is None:
        print("DUT did not come back within %ds — check serial log"
              % READY_TIMEOUT_S, file=sys.stderr)
        return 1
    return verify_running_version(health, root, manifest,
                                  image=root / spec["merged_bin"])


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
              "HEAD comparison (see ISSUE-102)" % image)
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
