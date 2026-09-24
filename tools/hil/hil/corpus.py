import datetime
import hashlib
import json
import os
import re
import subprocess
import zlib
from pathlib import Path

from hil.api import ApiError, Client
from hil.config import HilConfig

BENCH_FIXTURE_SHORTS = frozenset({0, 1, 2, 3})

ATTRIBUTE_SECTIONS = (
    "common_102", "dt6_led", "dt8_color", "extended", "groups",
    "memory_diagnostics", "memory_energy", "memory_bus_unit",
    "memory_identity", "memory_luminaire", "memory_profile", "scenes",
)

ATTRIBUTE_GROUPS = (
    "runtime_status", "common_102", "dt8_color", "dt6_led",
    "groups", "scenes", "extended", "scene_colours",
)
MEMORY_BANK_PRESET = "all"

SINGLETON_PATHS = (
    "health", "controller", "adapters", "time", "diagnostics", "stats",
    "firmware", "redundancy", "settings/redundancy", "settings/poller",
    "settings/dali", "settings/home-assistant", "policies", "hcl-schedules",
    "rules", "operations", "config/slices",
)

SECRET_SLICES = ("home_assistant_settings",)


def _utc_stamp() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S")


def _git_head(root: Path) -> dict:
    def _git(*args):
        out = subprocess.run(("git", "-C", str(root)) + args,
                             capture_output=True, text=True, timeout=20)
        return out.stdout.strip() if out.returncode == 0 else None

    commit = _git("rev-parse", "HEAD")
    status = _git("status", "--porcelain")
    return {"commit": commit,
            "dirty": None if status is None else bool(status.strip())}


def _write_bytes(path: Path, blob: bytes) -> dict:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(blob)
    return {"bytes": len(blob),
            "sha256": hashlib.sha256(blob).hexdigest(),
            "crc32": zlib.crc32(blob) & 0xFFFFFFFF}


def _write_json(path: Path, obj) -> dict:
    blob = json.dumps(obj, ensure_ascii=False, indent=1, sort_keys=True).encode()
    return _write_bytes(path, blob)


def _slug(text: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]", lambda m: "%%%02X" % ord(m.group()), text)


class Capture:
    def __init__(self, client: Client, out: Path, label: str):
        self.client = client
        self.out = out
        self.label = label
        self.artifacts = {}
        self.failures = []

    def _get_raw(self, path: str):
        try:
            resp = self.client.raw_response("GET", path)
        except Exception as exc:
            return 0, ("%s: %s" % (type(exc).__name__, exc)).encode()
        return resp.status_code, resp.content

    def record(self, name: str, rel: Path, status: int, blob: bytes) -> bool:
        if status != 200:
            self.failures.append({"artifact": name, "status": status,
                                  "body": blob[:200].decode("utf-8", "replace")})
            return False
        self.artifacts[name] = dict(_write_bytes(self.out / rel, blob),
                                    path=str(rel))
        return True

    def get_json(self, path: str):
        status, blob = self._get_raw(path)
        if status != 200:
            self.failures.append({"artifact": "probe:" + path, "status": status,
                                  "body": blob[:200].decode("utf-8", "replace")})
            return None
        try:
            return json.loads(blob)
        except ValueError:
            return None

    def capture_get(self, name: str, path: str, rel: Path) -> bool:
        status, blob = self._get_raw(path)
        return self.record(name, rel, status, blob)

    def identity(self) -> dict:
        health = self.get_json("health") or {}
        controller = self.get_json("controller") or {}
        return {"label": self.label, "base": self.client.base,
                "version": health.get("version"), "role": health.get("role"),
                "uptime_seconds": health.get("uptime_seconds"),
                "controller_id": controller.get("controller_id"),
                "adapter_count": controller.get("adapter_count")}

    def slices(self, keep_secrets: bool = False) -> None:
        manifest = self.get_json("config/slices")
        if manifest is None:
            return
        self.artifacts["slices/manifest"] = dict(
            _write_json(self.out / "slices" / "manifest.json", manifest),
            path="slices/manifest.json")
        for row in manifest:
            name = row.get("name")
            if not name or row.get("bytes") is None:
                continue
            if name in SECRET_SLICES and not keep_secrets:
                self.failures.append({"artifact": "slice:" + name,
                                      "status": "withheld",
                                      "body": "carries broker credentials "
                                              "(ADR-018 A10); re-run with "
                                              "--keep-secrets into an "
                                              "untracked --out"})
                continue
            quoted = name.replace("/", "%2F")
            status, blob = self._get_raw("config/slices/" + quoted)
            self.record("slice:" + name, Path("slices") / (_slug(name) + ".bin"),
                        status, blob)

    def rest(self) -> None:
        for path in SINGLETON_PATHS:
            self.capture_get("rest:" + path, path,
                             Path("rest") / (_slug(path) + ".json"))
        self._rest_rules()
        for aid in self._adapter_ids():
            self._rest_adapter(aid)
        for sid in self._ids("hcl-schedules", "schedules", "schedule_id"):
            self.capture_get("rest:hcl-schedules/%s" % sid,
                             "hcl-schedules/%s" % sid,
                             Path("rest") / "hcl-schedules" / (_slug(str(sid)) + ".json"))

    def _rest_rules(self) -> None:
        doc = self.get_json("rules")
        source = (doc or {}).get("source")
        if not source:
            return
        status, blob = self._http_post("rules/parse", {"source": source})
        self.record("rest:rules/parse", Path("rest") / "rules-parse.json",
                    status, blob)

    def _http_post(self, path: str, body: dict):
        resp = self.client.raw_response("POST", path, body)
        return resp.status_code, resp.content

    def _adapter_ids(self) -> list:
        return self._ids("adapters", "adapters", "adapter_id") or [self.client.adapter]

    def _ids(self, path: str, envelope: str, key: str) -> list:
        doc = self.get_json(path)
        rows = (doc or {}).get(envelope) or []
        return [r[key] for r in rows if isinstance(r, dict) and key in r]

    def _rest_adapter(self, aid: int) -> None:
        base = "adapters/%d" % aid
        rel = Path("rest") / base
        for leaf in ("", "/physical-devices", "/groups", "/virtual-lamps",
                     "/scenes", "/input-devices", "/group-membership-matrix"):
            name = base + leaf
            self.capture_get("rest:" + name, name,
                             rel / ((_slug(leaf.strip("/")) or "adapter") + ".json"))
        self._rest_devices(aid, rel)
        self._rest_collections(aid, rel)
        self._rest_input_devices(aid, rel)

    def _rest_devices(self, aid: int, rel: Path) -> None:
        for short in self._ids("adapters/%d/physical-devices" % aid,
                               "physical_devices", "short_address"):
            dev = rel / "physical-devices" / str(short)
            for leaf in ("", "/attributes", "/memory-banks"):
                name = "adapters/%d/physical-devices/%d%s" % (aid, short, leaf)
                self.capture_get("rest:" + name, name,
                                 dev / ((_slug(leaf.strip("/")) or "device") + ".json"))
            for section in ATTRIBUTE_SECTIONS:
                name = ("adapters/%d/physical-devices/%d/attributes?sections=%s"
                        % (aid, short, section))
                self.capture_get("rest:" + name, name,
                                 dev / "sections" / (section + ".json"))

    def _rest_collections(self, aid: int, rel: Path) -> None:
        for gid in self._ids("adapters/%d/groups" % aid, "groups", "group_id"):
            name = "adapters/%d/groups/%d" % (aid, gid)
            self.capture_get("rest:" + name, name,
                             rel / "groups" / ("%d.json" % gid))
        for lid in self._ids("adapters/%d/virtual-lamps" % aid,
                             "virtual_lamps", "virtual_lamp_id"):
            name = "adapters/%d/virtual-lamps/%d" % (aid, lid)
            self.capture_get("rest:" + name, name,
                             rel / "virtual-lamps" / ("%d.json" % lid))
        for sid in self._ids("adapters/%d/scenes" % aid, "scenes", "scene_id"):
            for leaf in ("", "/matrix"):
                name = "adapters/%d/scenes/%d%s" % (aid, sid, leaf)
                self.capture_get("rest:" + name, name,
                                 rel / "scenes" /
                                 ("%d%s.json" % (sid, leaf.replace("/", "-"))))

    def _rest_input_devices(self, aid: int, rel: Path) -> None:
        doc = self.get_json("adapters/%d/input-devices" % aid) or {}
        for row in doc.get("input_devices") or []:
            short = row.get("short_address")
            if short is None:
                continue
            name = "adapters/%d/input-devices/%d" % (aid, short)
            self.capture_get("rest:" + name, name,
                             rel / "input-devices" / ("%d.json" % short))

    ENABLE_DEVICE_TYPE = 0xC100
    DTR0 = 0xA300
    QUERY_CONTENT_DTR0 = 0x98

    STANDARD_QUERIES = (tuple(range(0x90, 0xA9)) + (0xAA,)
                        + tuple(range(0xB0, 0xC0)) + tuple(range(0xC0, 0xC5)))

    DT6_QUERIES = tuple(range(0xED, 0x100))
    DT8_QUERIES = (0xF7, 0xF8, 0xF9, 0xFB, 0xFF)

    QUERY_NAMES = {
        0x90: "QUERY STATUS", 0x91: "QUERY CONTROL GEAR PRESENT",
        0x92: "QUERY LAMP FAILURE", 0x93: "QUERY LAMP POWER ON",
        0x94: "QUERY LIMIT ERROR", 0x95: "QUERY RESET STATE",
        0x96: "QUERY MISSING SHORT ADDRESS", 0x97: "QUERY VERSION NUMBER",
        0x98: "QUERY CONTENT DTR0", 0x99: "QUERY DEVICE TYPE",
        0x9A: "QUERY PHYSICAL MINIMUM", 0x9B: "QUERY POWER FAILURE",
        0x9C: "QUERY CONTENT DTR1", 0x9D: "QUERY CONTENT DTR2",
        0x9E: "QUERY OPERATING MODE", 0x9F: "QUERY LIGHT SOURCE TYPE",
        0xA0: "QUERY ACTUAL LEVEL", 0xA1: "QUERY MAX LEVEL",
        0xA2: "QUERY MIN LEVEL", 0xA3: "QUERY POWER ON LEVEL",
        0xA4: "QUERY SYSTEM FAILURE LEVEL", 0xA5: "QUERY FADE TIME/FADE RATE",
        0xA6: "QUERY MANUFACTURER SPECIFIC MODE",
        0xA7: "QUERY NEXT DEVICE TYPE", 0xA8: "QUERY EXTENDED FADE TIME",
        0xAA: "QUERY CONTROL GEAR FAILURE",
        0xC0: "QUERY GROUPS 0-7", 0xC1: "QUERY GROUPS 8-15",
        0xC2: "QUERY RANDOM ADDRESS (H)", 0xC3: "QUERY RANDOM ADDRESS (M)",
        0xC4: "QUERY RANDOM ADDRESS (L)",
    }
    DT6_NAMES = {
        0xED: "QUERY GEAR TYPE", 0xEE: "QUERY DIMMING CURVE",
        0xEF: "QUERY POSSIBLE OPERATING MODES", 0xF0: "QUERY FEATURES",
        0xF1: "QUERY FAILURE STATUS", 0xF2: "QUERY SHORT CIRCUIT",
        0xF3: "QUERY OPEN CIRCUIT", 0xF4: "QUERY LOAD DECREASE",
        0xF5: "QUERY LOAD INCREASE", 0xF6: "QUERY CURRENT PROTECTOR ACTIVE",
        0xF7: "QUERY THERMAL SHUT DOWN", 0xF8: "QUERY THERMAL OVERLOAD",
        0xF9: "QUERY REFERENCE RUNNING",
        0xFA: "QUERY REFERENCE MEASUREMENT FAILED",
        0xFB: "QUERY CURRENT PROTECTOR ENABLED", 0xFC: "QUERY OPERATING MODE",
        0xFD: "QUERY FAST FADE TIME", 0xFE: "QUERY MIN FAST FADE TIME",
        0xFF: "QUERY EXTENDED VERSION NUMBER",
    }
    DT8_NAMES = {
        0xF7: "QUERY GEAR FEATURES/STATUS", 0xF8: "QUERY COLOUR STATUS",
        0xF9: "QUERY COLOUR TYPE FEATURES", 0xFA: "QUERY COLOUR VALUE",
        0xFB: "QUERY RGBWAF CONTROL", 0xFF: "QUERY EXTENDED VERSION NUMBER",
    }

    def gear(self, shorts) -> None:
        allowed = sorted(set(shorts) & BENCH_FIXTURE_SHORTS)
        skipped = sorted(set(shorts) - BENCH_FIXTURE_SHORTS)
        if skipped:
            self.failures.append(
                {"artifact": "gear:skipped", "status": "refused",
                 "body": "short addresses %s are the owner's live luminaires; "
                         "queries included (permanent rule 2026-09-14)"
                         % ",".join(str(s) for s in skipped)})
        for short in allowed:
            self._gear_one(short)

    def _gear_one(self, short: int) -> None:
        sweep = {"short_address": short,
                 "standard": self._sweep(short, self.STANDARD_QUERIES, None),
                 "dt6": self._sweep(short, self.DT6_QUERIES, 6),
                 "dt8": self._sweep(short, self.DT8_QUERIES, 8),
                 "colour_values": self._sweep_colour_values(short)}
        self.artifacts["gear:%d" % short] = dict(
            _write_json(self.out / "gear" / ("short-%d.json" % short), sweep),
            path="gear/short-%d.json" % short)

    def _exchange(self, short: int, opcode: int, device_type=None) -> dict:
        if device_type is not None:
            self.client.raw(self.ENABLE_DEVICE_TYPE | device_type)
        frame = (((short << 1) | 1) << 8) | opcode
        body = self.client.raw(frame, expects_backward=True)
        answered = isinstance(body, dict) and body.get("success") is True
        names = {6: self.DT6_NAMES, 8: self.DT8_NAMES}.get(
            device_type, self.QUERY_NAMES)
        return {"opcode": opcode,
                "name": names.get(opcode),
                "answered": answered,
                "byte": body.get("backward_frame") if answered else None}

    def _sweep(self, short: int, opcodes, device_type) -> list:
        return [self._exchange(short, op, device_type) for op in opcodes]

    def _sweep_colour_values(self, short: int) -> list:
        out = []
        for selector in _defined_colour_values():
            self.client.raw(self.DTR0 | selector)
            entry = self._exchange(short, 0xFA, 8)
            entry["selector"] = selector
            entry["wide"] = _colour_value_is_wide(selector)
            if entry["answered"] and entry["wide"]:
                lsb = self._exchange(short, self.QUERY_CONTENT_DTR0, None)
                entry["lsb_answered"] = lsb["answered"]
                entry["lsb"] = lsb["byte"]
                entry["lsb_echoes_selector"] = lsb["byte"] == selector
            out.append(entry)
        return out


def _colour_value_is_wide(value_id: int) -> bool:
    return (0 <= value_id <= 8 or 64 <= value_id <= 81
            or 128 <= value_id <= 131 or 192 <= value_id <= 200
            or 224 <= value_id <= 232)


def _defined_colour_values() -> list:
    narrow = (list(range(9, 16)) + [82] + list(range(201, 209))
              + list(range(233, 241)))
    wide = [v for v in range(0, 241) if _colour_value_is_wide(v)]
    return sorted(set(narrow) | set(wide))


WIRE_CLASSES = (
    ("captures", re.compile(r"DALI sniff:.*capture=\[")),
    ("line_held", re.compile(r"DALI line held:")),
    ("sniff_timing", re.compile(r"DALI sniff timing:")),
    ("sniff_diag", re.compile(r"DALI sniff diag:")),
    ("isr_late", re.compile(r"DALI ISR late (tick|entries)")),
    ("flash_id", re.compile(r"flash: jedec=0x|spi_flash: detected chip")),
    ("phy_interrupt", re.compile(r"DALI PHY interrupt:")),
    ("persist_overlap", re.compile(r"DALI persist overlap:")),
    ("arbitration", re.compile(r"DALI arbitration:")),
    ("stack_census", re.compile(r"task stack hwm")),
    ("boot_heap", re.compile(r"boot heap \[")),
    ("boot", re.compile(r"boot:0x[0-9a-f]+|rst:0x|dali2rust .*version|"
                        r"ESP-ROM:|cpu_start:")),
)

WIRE_KEEP_DEFAULT = 2000

WIRE_KEEP_BY_CLASS = {
    "captures": 8000,
    "stack_census": 600,
    "boot_heap": 600,
    "arbitration": 600,
    "sniff_diag": 1000,
    "sniff_timing": 1000,
}

WIRE_SCAN_BYTES_DEFAULT = 256 * 1024 * 1024


def capture_wire(log: Path, out: Path, keep: int = WIRE_KEEP_DEFAULT,
                 scan_bytes: int = WIRE_SCAN_BYTES_DEFAULT) -> dict:
    import collections as _c
    if not log.exists():
        return {"log": str(log), "present": False}
    size = log.stat().st_size
    start = max(0, size - scan_bytes)
    kept = {name: _c.deque(maxlen=_keep_for(name, keep))
            for name, _ in WIRE_CLASSES}
    counts = _c.Counter()
    with log.open("r", errors="replace") as fh:
        if start:
            fh.seek(start)
            fh.readline()
        for line in fh:
            for name, pattern in WIRE_CLASSES:
                if pattern.search(line):
                    counts[name] += 1
                    kept[name].append(line.rstrip("\n"))
                    break
    written = {}
    for name, lines in kept.items():
        if not lines:
            continue
        blob = ("\n".join(lines) + "\n").encode()
        written[name] = dict(_write_bytes(out / "wire" / (name + ".log"), blob),
                             lines=len(lines), seen_in_window=counts[name],
                             path="wire/%s.log" % name)
    return {"log": str(log), "present": True, "file_bytes": size,
            "scanned_bytes": size - start, "truncated": start > 0,
            "keep_per_class": keep, "classes": written}


def _keep_for(name: str, keep: int) -> int:
    override = WIRE_KEEP_BY_CLASS.get(name)
    if override is None:
        return keep
    return max(1, int(override * keep / WIRE_KEEP_DEFAULT))


def _board_capture(cfg: HilConfig, label: str, out: Path, which: set,
                   keep_secrets: bool, wire_keep: int, wire_scan: int) -> dict:
    client = Client(cfg)
    cap = Capture(client, out, label)
    before = cap.identity()
    reachable = before.get("version") is not None
    if not reachable:
        cap.failures.append(
            {"artifact": "board:unreachable", "status": "skipped",
             "body": "%s did not answer; slices/rest/gear skipped, local "
                     "artefacts still captured" % cfg.base})
    if reachable and "slices" in which:
        cap.slices(keep_secrets=keep_secrets)
    if reachable and "rest" in which:
        cap.rest()
    wire = {}
    if reachable and "gear" in which:
        if before.get("role") == "active":
            cap.gear(cfg.gear_short_set() or BENCH_FIXTURE_SHORTS)
        else:
            cap.failures.append(
                {"artifact": "gear:role", "status": "skipped",
                 "body": "role %r: a passive controller refuses everything "
                         "that touches the wire (409 controller_standby)"
                         % before.get("role")})
    if "wire" in which:
        wire = capture_wire(cfg.persist_serial_log, out,
                            keep=wire_keep, scan_bytes=wire_scan)
    if "bench" in which:
        capture_bench_identity(cfg, cap)
    after = cap.identity()
    return {"identity_before": before, "identity_after": after,
            "rebooted_during_capture":
                _rebooted(before.get("uptime_seconds"), after.get("uptime_seconds")),
            "artifacts": cap.artifacts, "failures": cap.failures, "wire": wire}


def capture_bench_identity(cfg: HilConfig, cap: Capture) -> None:
    for name in BENCH_IDENTITY_FILES:
        src = cfg.state_dir / name
        if not src.exists():
            cap.failures.append({"artifact": "bench:" + name, "status": "absent",
                                 "body": str(src)})
            continue
        cap.artifacts["bench:" + name] = dict(
            _write_bytes(cap.out / "bench" / name, src.read_bytes()),
            path="bench/" + name)
    masks = cfg.state_dir / "masks"
    if masks.is_dir():
        for mask in sorted(masks.iterdir()):
            if mask.is_file():
                cap.artifacts["bench:masks/" + mask.name] = dict(
                    _write_bytes(cap.out / "bench" / "masks" / mask.name,
                                 mask.read_bytes()),
                    path="bench/masks/" + mask.name)


def _rebooted(before, after) -> bool:
    if before is None or after is None:
        return False
    return after < before


BENCH_IDENTITY_FILES = (
    "camera_bench.json", "camera_identity.json", "uvc-util.commit",
    "calibration.json", "calibration-day.json", "calibration-night.json",
    "baseline_meta.json", "baseline.npz", "baseline.png",
    "rois.png", "masks_overlay.png",
    "wb_master_controls.json", "serial_port.pin", "stack_previous.json",
)

PARTS = ("slices", "rest", "gear", "wire", "bench")

CORPUS_ROOT = Path(__file__).resolve().parent.parent / "corpus"


def run(parts=None, out_root: Path = None, boards: str = "both",
        keep_secrets: bool = False, wire_keep: int = WIRE_KEEP_DEFAULT,
        wire_scan: int = WIRE_SCAN_BYTES_DEFAULT) -> dict:
    parts = set(parts or PARTS)
    if "all" in parts:
        parts = set(PARTS)
    stamp = _utc_stamp()
    root = Path(out_root or CORPUS_ROOT) / stamp
    root.mkdir(parents=True, exist_ok=True)
    primary = HilConfig()
    primary.ensure_dirs()
    index = {"captured_at_utc": stamp, "parts": sorted(parts),
             "repo": _git_head(Path(__file__).resolve().parents[3]),
             "tool": "hil corpus", "boards": {}}
    for label, cfg in _boards(primary, boards):
        print("hil corpus: %s (%s)" % (label, cfg.base))
        index["boards"][label] = _board_capture(
            cfg, label, root / label, parts, keep_secrets, wire_keep, wire_scan)
    index["full_serial_logs"] = _serial_log_note(primary, boards)
    _write_json(root / "index.json", index)
    _write_readme(root, index)
    _report(root, index)
    return index


def _boards(primary: HilConfig, boards: str):
    out = []
    if boards in ("both", "primary"):
        out.append(("primary", primary))
    if boards in ("both", "peer"):
        if primary.has_peer:
            out.append(("peer", primary.peer()))
        else:
            print("hil corpus: no peer configured (HIL_PEER_BASE) — skipping")
    return out


def _serial_log_note(primary: HilConfig, boards: str) -> dict:
    note = {"warning": "full serial logs are NOT captured here — back them up",
            "logs": []}
    for label, cfg in _boards(primary, boards):
        log = cfg.persist_serial_log
        note["logs"].append({
            "board": label, "path": str(log),
            "bytes": log.stat().st_size if log.exists() else None})
    return note


def _report(root: Path, index: dict) -> None:
    print("\nhil corpus -> %s" % root)
    for label, board in index["boards"].items():
        ident = board["identity_before"]
        print("  %-8s %s role=%s version=%s artifacts=%d failures=%d"
              % (label, ident.get("base"), ident.get("role"),
                 ident.get("version"), len(board["artifacts"]),
                 len(board["failures"])))
        if board["rebooted_during_capture"]:
            print("    !! uptime went BACKWARDS during capture — the board "
                  "rebooted; everything after that point is a different boot")
        classes = board.get("wire", {}).get("classes") or {}
        if classes:
            wire = board["wire"]
            print("    wire: %s%s"
                  % (", ".join("%s=%d/%d" % (k, v["lines"], v["seen_in_window"])
                               for k, v in sorted(classes.items())),
                     " [TRUNCATED window %d of %d bytes]"
                     % (wire["scanned_bytes"], wire["file_bytes"])
                     if wire.get("truncated") else ""))
        for failure in board["failures"][:8]:
            print("    - %s: %s" % (failure["artifact"], failure["status"]))
        extra = len(board["failures"]) - 8
        if extra > 0:
            print("    - (%d more in index.json)" % extra)
    for log in index.get("full_serial_logs", {}).get("logs", []):
        print("  serial %-6s %s (%s bytes) — NOT in the corpus, back it up"
              % (log["board"], log["path"], log["bytes"]))


README = """# Bench corpus `{stamp}`

Captured by `hil corpus` from the live bench. **This is evidence, not a
snapshot to be tidied up:** every file is what a real controller, a real
driver or a real interrupt answered at that instant, and nothing in it can be
re-derived on a host once the hardware is gone.

Repo at capture time: `{commit}`{dirty}

| Board | URL | Role | Firmware | Artifacts | Failures |
| --- | --- | --- | --- | --- | --- |
{rows}

## What each part is, and how to replay it

- **`slices/`** — the installation's own configuration, byte for byte, as
  `GET /api/v1/config/slices/{{name}}` served it. The host slice store
  (`slice_store_files.rs`) uses the same names, so dropping these into a host
  dev server's store makes it hydrate as THIS bench: 13 devices, their
  bindings, groups, 16 scenes, HCL schedules, rules, policies. Import order is
  hydration order — physical devices before virtual lamps, or every binding is
  orphaned at once (measured, 30 of 30).
- **`rest/`** — every read-only resource, verbatim bytes. Per-device
  `attributes` are captured whole AND per `?sections=` token, because that
  filter is a frozen response shape and a test pinned only on the whole tree
  would not see one section break.
- **`gear/short-N.json`** — the wire. Every standard query, every DT6 and DT8
  query, and `QUERY COLOUR VALUE` across every defined Table 11 identifier,
  per bench fixture, each recorded as `answered` plus `byte` — `null`, never
  `0`, when nothing answered. This is what `dali2rust-gear-model` can be
  pinned on so the host fake stops modelling the standard's ideal driver and
  starts modelling the ones that were on this wire, deviations included.
  Capped at short addresses 0..3 in code: 4 and up are the owner's live
  luminaires and queries count as touching them.
- **`wire/`** — the instrument-level evidence, classified out of the serial
  log: undecodable PHY captures (plus one readable one per 256 as a length
  calibration), line holds, the sniffer's timing line, late-tick reports, the
  per-task stack census, the boot heap ladder, arbitration traffic.
  `captures.log` replays directly: `capture_from_hex` in
  `dali2rust-dali-codec::rx_decode` turns a `capture=[0f 0f 1e ...]` dump back
  into an `RxCompletedEvent` and decodes it on the host.

## What is deliberately NOT here

- **The full serial logs.** {serial_note}
- **Anything that changes state.** The capture is GETs plus DALI *queries*; it
  runs against a live controller mid-schedule without disturbing it.
- **Broker credentials.** The `home_assistant_settings` slice carries the
  password (owner's call, `ADR-018` A10, so a standby announces the same
  installation). A corpus lives in the repository, which is a different
  audience: the slice is withheld unless `--keep-secrets` is passed, and the
  withholding is recorded in `index.json` rather than left silent.

## Reading `index.json`

`identity_before` / `identity_after` bracket the capture, and
`rebooted_during_capture` is the only thing that can tell you the second half
came from a different boot. `failures` is the honest half: a missing artifact
and an artifact that came back empty are different facts, and after the bench
is gone nobody can tell them apart by retrying.
"""


def _write_readme(root: Path, index: dict) -> None:
    rows = []
    for label, board in index["boards"].items():
        ident = board["identity_before"]
        rows.append("| `%s` | %s | %s | `%s` | %d | %d |"
                    % (label, ident.get("base"), ident.get("role"),
                       ident.get("version"), len(board["artifacts"]),
                       len(board["failures"])))
    logs = index.get("full_serial_logs", {}).get("logs", [])
    serial_note = "; ".join(
        "%s: `%s` (%s bytes)" % (l["board"], l["path"], l["bytes"]) for l in logs)
    repo = index.get("repo") or {}
    (root / "README.md").write_text(README.format(
        stamp=index["captured_at_utc"],
        commit=repo.get("commit") or "unknown",
        dirty=" (dirty tree)" if repo.get("dirty") else "",
        rows="\n".join(rows),
        serial_note=(serial_note or "none found")
        + " — gitignored, and the raw evidence behind every measurement in "
          "`STRATEGY.md` §7. Back them up outside this repository before the "
          "bench goes, or this classified extract becomes the only copy."),
        encoding="utf-8")


PANEL_ROOT = CORPUS_ROOT / "panel"


def capture_panel(cfg: HilConfig, seconds: int = 300, out_root: Path = None,
                  label: str = "panel") -> dict:
    from hil import wsclient
    stamp = _utc_stamp()
    out = Path(out_root or (CORPUS_ROOT / label)) / stamp
    out.mkdir(parents=True, exist_ok=True)
    client = Client(cfg)
    cap = Capture(client, out, "panel")
    cap.capture_get("panel:devices", "adapters/%d/input-devices" % cfg.adapter,
                    Path("input-devices.json"))
    for short in _panel_shorts(cap, cfg.adapter):
        cap.capture_get("panel:device-%d" % short,
                        "adapters/%d/input-devices/%d" % (cfg.adapter, short),
                        Path("device-%d.json" % short))
    session = _panel_listen(cfg, out, seconds, wsclient)
    session["captured_at_utc"] = stamp
    session["artifacts"] = cap.artifacts
    _write_json(out / "index.json", session)
    return session


def _panel_shorts(cap: Capture, adapter: int) -> list:
    doc = cap.get_json("adapters/%d/input-devices" % adapter) or {}
    return [r["short_address"] for r in doc.get("input_devices") or []
            if r.get("short_address") is not None]


def _panel_listen(cfg: HilConfig, out: Path, seconds: int, wsclient) -> dict:
    import time
    frames, events, others = [], [], 0
    deadline = time.time() + seconds
    print("\nЗаписываю %d с. Жмите кнопки — каждый кадр печатается сюда.\n"
          % seconds)
    with wsclient.connect(cfg.base) as ws:
        ws.subscribe(["sniffer", "input"])
        while time.time() < deadline:
            try:
                msg = ws.recv_json(timeout=min(2.0, max(0.1, deadline - time.time())))
            except Exception:
                continue
            if not isinstance(msg, dict):
                continue
            others += _panel_record(msg, frames, events)
    _write_jsonl(out / "frames.jsonl", frames)
    _write_jsonl(out / "input-events.jsonl", events)
    wide = [f for f in frames if f.get("width") == "forward24"]
    probes = sum(1 for f in wide if f.get("hex") == ARBITRATION_PROBE_HEX)
    print("\nЗаписано: %d кадров 24 бита, из них %d зондов арбитража (в файле, "
          "не на экране); %d входных событий; прочего %d"
          % (len(wide), probes, len(events), others))
    return {"frames": len(frames), "input_events": len(events),
            "forward24": len(wide), "arbitration_probes": probes,
            "panel_frames": len(wide) - probes,
            "seconds": seconds, "base": cfg.base}


ARBITRATION_PROBE_HEX = "FF FE 3D"


def _panel_record(msg: dict, frames: list, events: list) -> int:
    channel = msg.get("channel")
    payload = msg.get("payload") or {}
    if channel == "sniffer":
        for frame in payload.get("frames") or []:
            frames.append(dict(frame, ts_ms=msg.get("ts_ms")))
            if (frame.get("width") == "forward24"
                    and frame.get("hex") != ARBITRATION_PROBE_HEX):
                print("  кадр %-3d %s  %s%s"
                      % (len(frames), frame.get("hex"), frame.get("name") or "?",
                         " — " + frame["detail"] if frame.get("detail") else ""),
                      flush=True)
        return 0
    if channel == "input":
        events.append(dict(payload, batch_ts_ms=msg.get("ts_ms")))
        print("  событие  %s" % json.dumps(payload, ensure_ascii=False)[:160])
        return 0
    return 1


def _write_jsonl(path: Path, rows: list) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps(row, ensure_ascii=False) + "\n")
