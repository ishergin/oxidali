from __future__ import annotations

import argparse
import json
import sys
import threading
import time
from pathlib import Path

DESCRIPTION = """ISSUE-86 acceptance: a busy worker must not break the liveness lease.

Holds the wire with a full attribute read while a probe schedule on empty groups
(level_mode none, so no light moves) ticks; the owner's schedules are disabled
for the run and restored afterwards. Passes when hcl.command_timeouts grows by at
least 2 within one tick while worker_stale stays 0, no stale-worker line names
hcl, and the peer records no peer_silent."""

sys.path.insert(0, str(Path(__file__).resolve().parent))

from hil import config as config_mod
from hil.api import ATTR_GROUPS_DEFAULT, Client

SCHEDULE_ID = "issue86-probe"
ALLOWED_SHORTS = (0, 2, 3)
TICK_PERIOD_S = 60
TIMEOUTS_PER_TICK_REQUIRED = 2
SAMPLE_EVERY_S = 1.0


def _now() -> float:
    return time.monotonic()


class Sampler(threading.Thread):
    def __init__(self, api: Client, peer: Client | None, out: Path):
        super().__init__(daemon=True)
        self.api = api
        self.peer = peer
        self.out = out
        self.stop_flag = threading.Event()
        self.samples: list[dict] = []

    def run(self) -> None:
        with self.out.open("w", encoding="utf-8") as fh:
            while not self.stop_flag.is_set():
                sample = self._sample()
                if sample is not None:
                    self.samples.append(sample)
                    fh.write(json.dumps(sample, ensure_ascii=False) + "\n")
                    fh.flush()
                self.stop_flag.wait(SAMPLE_EVERY_S)

    def _sample(self) -> dict | None:
        try:
            diag = self.api.diagnostics()
        except Exception as exc:
            return {"t": _now(), "error": repr(exc)}
        hcl = diag.get("hcl", {})
        red = diag.get("redundancy", {})
        sample = {
            "t": _now(),
            "ticks": hcl.get("ticks"),
            "command_timeouts": hcl.get("command_timeouts"),
            "commands_published": hcl.get("commands_published"),
            "command_failures": hcl.get("command_failures"),
            "worker_stale": red.get("worker_stale"),
            "answered": red.get("answered"),
            "suppressed": red.get("suppressed"),
            "window_closed": red.get("window_closed"),
            "late": red.get("late"),
        }
        if self.peer is not None:
            try:
                peer_red = self.peer._req("GET", "redundancy")
                sample["peer_owned"] = peer_red["probes"]["owned"]
                sample["peer_unowned"] = peer_red["probes"]["unowned"]
                sample["peer_transitions"] = len(peer_red.get("transitions", []))
                sample["peer_takeovers"] = peer_red.get("takeovers")
            except Exception as exc:
                sample["peer_error"] = repr(exc)
        return sample


def _points(local_minutes: int) -> list[dict]:
    points = []
    for step in range(-2, 5):
        minute = (local_minutes + step * 10) % (24 * 60)
        points.append(
            {
                "time_ref": "absolute",
                "offset_minutes": minute,
                "level_mode": "none",
                "level": None,
                "color_temperature_kelvin": 2000 if step % 2 else 6500,
            }
        )
    points.sort(key=lambda p: p["offset_minutes"])
    return points


def _schedule_body(local_minutes: int, groups: list[int]) -> dict:
    return {
        "schedule_id": SCHEDULE_ID,
        "enabled": True,
        "algorithm": "interpolated",
        "active_days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
        "location": None,
        "targets": [{"adapter_id": 0, "scope": "group", "group_ids": groups}],
        "points": _points(local_minutes),
    }


def _assert_groups_empty(api: Client, groups: list[int]) -> None:
    listing = api._req("GET", "adapters/%d/groups" % api.adapter)["groups"]
    by_id = {g["group_id"]: g for g in listing}
    for gid in groups:
        g = by_id.get(gid)
        if g is None:
            raise SystemExit("group %d does not exist on this adapter" % gid)
        if g["member_count_desired"] or g["member_count_applied"]:
            raise SystemExit(
                "group %d has members (%d/%d) — pick empty groups, a setpoint "
                "on a populated group moves the owner's light"
                % (gid, g["member_count_desired"], g["member_count_applied"])
            )


def _tick_windows(samples: list[dict]) -> list[dict]:
    windows = []
    start = None
    for sample in samples:
        if sample.get("ticks") is None:
            continue
        if start is None:
            start = sample
            continue
        if sample["ticks"] != start["ticks"]:
            windows.append(
                {
                    "ticks_from": start["ticks"],
                    "ticks_to": sample["ticks"],
                    "d_ticks": sample["ticks"] - start["ticks"],
                    "d_timeouts": sample["command_timeouts"] - start["command_timeouts"],
                    "d_published": sample["commands_published"] - start["commands_published"],
                    "d_failures": sample["command_failures"] - start["command_failures"],
                    "d_worker_stale": sample["worker_stale"] - start["worker_stale"],
                    "seconds": round(sample["t"] - start["t"], 1),
                }
            )
            start = sample
    return windows


def _serial_window(path: Path, from_bytes: int) -> list[str]:
    if not path.exists():
        return []
    with path.open("rb") as fh:
        fh.seek(from_bytes)
        blob = fh.read()
    text = blob.decode("utf-8", errors="replace")
    keys = ("last turned", "has not turned", "peer_silent", "stale")
    return [line for line in text.splitlines() if any(k in line for k in keys)]


def main() -> int:
    parser = argparse.ArgumentParser(description=DESCRIPTION)
    parser.add_argument("--minutes", type=float, default=12.0)
    parser.add_argument("--groups", default="4,5,6")
    parser.add_argument("--keep", action="store_true",
                        help="leave the probe schedule in place (debugging only)")
    args = parser.parse_args()

    groups = [int(x) for x in args.groups.split(",") if x.strip()]
    if len(groups) < TIMEOUTS_PER_TICK_REQUIRED:
        raise SystemExit("need at least %d target groups: one timeout per "
                         "target per tick" % TIMEOUTS_PER_TICK_REQUIRED)

    cfg = config_mod.load()
    api = Client(cfg)
    peer = None
    if cfg.has_peer:
        try:
            peer = Client(cfg.peer())
            peer.health()
        except Exception as exc:
            print("peer unreachable, continuing without it: %r" % exc)
            peer = None

    health = api.health()
    if health.get("role") != "active":
        raise SystemExit("this controller is %r — run the provocation on the "
                         "ACTIVE unit, a standby publishes no setpoints"
                         % health.get("role"))
    _assert_groups_empty(api, groups)

    serial_log = Path(cfg.state_dir) / "persist" / "serial.log"
    serial_from = serial_log.stat().st_size if serial_log.exists() else 0

    out_dir = Path(cfg.state_dir) / "issue86"
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    jsonl = out_dir / ("samples-%s.jsonl" % stamp)

    devices = api._req("GET", "adapters/%d/physical-devices"
                       % api.adapter)["physical_devices"]
    read_shorts = [d["short_address"] for d in devices
                   if d["short_address"] in ALLOWED_SHORTS]
    if not read_shorts:
        raise SystemExit("none of the bench fixtures %s is in the registry — the "
                         "provocateur reads those and nothing else" % (ALLOWED_SHORTS,))

    print("primary %s %s, peer %s" % (cfg.base, health.get("version"),
                                      "yes" if peer else "no"))
    print("provocateur: attribute reads (banks=all) on %s" % read_shorts)
    print("schedule %s on empty groups %s, %.0f min" % (SCHEDULE_ID, groups, args.minutes))

    suspended: list[str] = []
    created = False
    sampler = Sampler(Client(cfg), Client(cfg.peer()) if peer is not None else None, jsonl)
    try:
        for s in api.hcl.list():
            if not s.get("enabled"):
                continue
            suspended.append(s["schedule_id"])
            api.hcl.patch(s["schedule_id"], {"enabled": False})
        print("suspended pre-existing schedules: %s" % (suspended or "none"))

        local_minutes = api.time_get()["local_minutes"]
        api.hcl.create(_schedule_body(local_minutes, groups))
        created = True

        sampler.start()
        deadline = _now() + args.minutes * 60
        reads = 0
        while _now() < deadline:
            for short in read_shorts:
                if _now() >= deadline:
                    break
                try:
                    api.wait_op(api.attr_read(short, groups=ATTR_GROUPS_DEFAULT, banks="all"))
                    reads += 1
                except Exception as exc:
                    print("read on SA%02d: %r" % (short, exc))
        sampler.stop_flag.set()
        sampler.join(timeout=5)
        print("attribute reads completed: %d" % reads)
    finally:
        sampler.stop_flag.set()
        if created and not args.keep:
            try:
                api.hcl.delete(SCHEDULE_ID)
            except Exception as exc:
                print("WARNING: probe schedule %s not deleted: %r" % (SCHEDULE_ID, exc))
        for schedule_id in suspended:
            try:
                api.hcl.patch(schedule_id, {"enabled": True})
            except Exception as exc:
                print("WARNING: schedule %s left disabled: %r" % (schedule_id, exc))

    windows = _tick_windows(sampler.samples)
    serial_lines = _serial_window(serial_log, serial_from)
    qualifying = [w for w in windows
                  if w["d_ticks"] == 1 and w["d_timeouts"] >= TIMEOUTS_PER_TICK_REQUIRED]
    stale_moved = any(w["d_worker_stale"] for w in windows)
    hcl_stale_lines = [line for line in serial_lines if "hcl" in line]
    peer_transitions = 0
    if sampler.samples:
        firsts = [s for s in sampler.samples if "peer_transitions" in s]
        if len(firsts) >= 2:
            peer_transitions = firsts[-1]["peer_transitions"] - firsts[0]["peer_transitions"]

    print("\n--- ISSUE-86 acceptance ---")
    print("ticks observed:            %d" % len(windows))
    print("ticks with >= %d timeouts:  %d" % (TIMEOUTS_PER_TICK_REQUIRED, len(qualifying)))
    print("worker_stale moved:        %s" % ("YES" if stale_moved else "no"))
    print("serial lines naming hcl:   %d" % len(hcl_stale_lines))
    print("peer transitions added:    %d" % peer_transitions)
    for w in windows:
        print("  tick %s→%s  %5.1fs  published %+d  timeouts %+d  failures %+d  stale %+d"
              % (w["ticks_from"], w["ticks_to"], w["seconds"], w["d_published"],
                 w["d_timeouts"], w["d_failures"], w["d_worker_stale"]))
    for line in hcl_stale_lines[:10]:
        print("  serial: %s" % line)
    print("samples: %s" % jsonl)

    verdict = (qualifying and not stale_moved and not hcl_stale_lines
               and peer_transitions == 0)
    print("\nVERDICT: %s" % ("PASS — a busy worker kept its lease"
                             if verdict else "INCONCLUSIVE / FAIL (see above)"))
    return 0 if verdict else 1


if __name__ == "__main__":
    raise SystemExit(main())
