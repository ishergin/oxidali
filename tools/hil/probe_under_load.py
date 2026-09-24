from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

DESCRIPTION = """What the arbitration probe costs, and survives, while the bus is busy.

Samples both units every 5 s: the pair's wire cost, probes without a verdict and
why, and whether anything moved the bus. Run it once on a quiet bus and once under
load, naming each with --label, and compare the two. It drives nothing: every
number is read from /api/v1/diagnostics and /api/v1/redundancy."""

sys.path.insert(0, str(Path(__file__).resolve().parent))

from hil import config as config_mod
from hil.api import Client

SAMPLE_EVERY_S = 5.0
PROBE_EXCHANGE_MS = 40.0


def _snapshot(api: Client, peer: Client) -> dict:
    diag = api.diagnostics()
    red = diag["redundancy"]
    wire = diag["dali_wire"]
    sniff = diag["phy_sniffer"]
    poller = diag["poller"]
    out = {
        "t": time.monotonic(),
        "answered": red["answered"],
        "late": red["late"],
        "window_closed": red["window_closed"],
        "suppressed": red["suppressed"],
        "aborted": red["aborted"],
        "cell_busy": red["cell_busy"],
        "worker_stale": red["worker_stale"],
        "forward24": sniff["forward24"],
        "unsupported_len": sniff["unsupported_len"],
        "decode_failed": sniff["decode_failed"],
        "load_permille": wire["load_permille"],
        "load_own_permille": wire["load_own_permille"],
        "collisions": wire["collisions"],
        "foreign_in_window": wire["foreign_in_window"],
        "corrupted_in_window": wire["corrupted_in_window"],
        "exchange_retries": wire["exchange_retries"],
        "poller_reads_completed": poller["reads_completed"],
        "poller_reads_preempted": poller["reads_preempted"],
        "poller_duty_deferred": poller["duty_deferred"],
        "poller_interactive_deferred": poller["interactive_deferred"],
    }
    peer_red = peer._req("GET", "redundancy")
    peer_diag = peer.diagnostics()["redundancy"]
    out.update({
        "peer_published": peer_red["probes"]["published"],
        "peer_owned": peer_red["probes"]["owned"],
        "peer_unowned": peer_red["probes"]["unowned"],
        "peer_probe_failed": peer_diag["probe_failed"],
        "peer_takeovers": peer_red["takeovers"],
        "peer_stand_downs": peer_red["stand_downs"],
        "peer_transitions": len(peer_red["transitions"]),
        "primary_takeovers": api.redundancy.get()["takeovers"],
    })
    return out


def _delta(first: dict, last: dict, key: str) -> int:
    return last[key] - first[key]


def main() -> int:
    parser = argparse.ArgumentParser(description=DESCRIPTION)
    parser.add_argument("--minutes", type=float, default=10.0)
    parser.add_argument("--label", default="unlabelled",
                        help="what this window IS (quiet / poller / wb+poller …)")
    args = parser.parse_args()

    cfg = config_mod.load()
    if not cfg.has_peer:
        raise SystemExit("no peer configured — the probe comes from the standby")
    api = Client(cfg)
    peer = Client(cfg.peer())
    if api.health().get("role") != "active":
        raise SystemExit("run this against the ACTIVE unit's config (it answers the probes)")

    out_dir = Path(cfg.state_dir) / "probe_load"
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    jsonl = out_dir / ("%s-%s.jsonl" % (args.label, stamp))

    samples = []
    deadline = time.monotonic() + args.minutes * 60
    print("window %.0f min, label %r — reading both units every %.0f s"
          % (args.minutes, args.label, SAMPLE_EVERY_S))
    with jsonl.open("w", encoding="utf-8") as fh:
        while time.monotonic() < deadline:
            try:
                sample = _snapshot(api, peer)
            except Exception as exc:
                sample = {"t": time.monotonic(), "error": repr(exc)}
            samples.append(sample)
            fh.write(json.dumps(sample, ensure_ascii=False) + "\n")
            fh.flush()
            time.sleep(SAMPLE_EVERY_S)

    good = [s for s in samples if "error" not in s]
    if len(good) < 2:
        raise SystemExit("not enough samples: %s" % samples[:2])
    first, last = good[0], good[-1]
    seconds = last["t"] - first["t"]
    published = _delta(first, last, "peer_published")
    owned = _delta(first, last, "peer_owned")
    unowned = _delta(first, last, "peer_unowned")
    failed = _delta(first, last, "peer_probe_failed")
    receive_loss = _delta(first, last, "unsupported_len") + _delta(first, last, "decode_failed")
    loads = [s["load_permille"] for s in good]
    own_loads = [s["load_own_permille"] for s in good]

    def pct(n, d):
        return "%.3f %%" % (100.0 * n / d) if d else "n/a"

    summary = {
        "label": args.label,
        "seconds": round(seconds, 1),
        "probes_published": published,
        "probes_owned": owned,
        "probes_unowned": unowned,
        "probes_failed_to_transmit": failed,
        "unowned_share": pct(unowned, owned + unowned),
        "failed_share": pct(failed, published),
        "primary_answered": _delta(first, last, "answered"),
        "primary_receive_loss": receive_loss,
        "invariant_unowned_le_receive_loss": unowned <= receive_loss,
        "late": _delta(first, last, "late"),
        "window_closed": _delta(first, last, "window_closed"),
        "suppressed": _delta(first, last, "suppressed"),
        "aborted": _delta(first, last, "aborted"),
        "cell_busy": _delta(first, last, "cell_busy"),
        "worker_stale": _delta(first, last, "worker_stale"),
        "collisions": _delta(first, last, "collisions"),
        "foreign_in_window": _delta(first, last, "foreign_in_window"),
        "corrupted_in_window": _delta(first, last, "corrupted_in_window"),
        "exchange_retries": _delta(first, last, "exchange_retries"),
        "poller_reads_completed": _delta(first, last, "poller_reads_completed"),
        "poller_reads_preempted": _delta(first, last, "poller_reads_preempted"),
        "poller_duty_deferred": _delta(first, last, "poller_duty_deferred"),
        "wire_load_permille_min_med_max": [min(loads), sorted(loads)[len(loads) // 2], max(loads)],
        "own_load_permille_min_med_max": [min(own_loads),
                                          sorted(own_loads)[len(own_loads) // 2],
                                          max(own_loads)],
        "probe_share_of_window": pct(owned * PROBE_EXCHANGE_MS / 1000.0, seconds),
        "takeovers": _delta(first, last, "peer_takeovers"),
        "stand_downs": _delta(first, last, "peer_stand_downs"),
        "transitions_added": _delta(first, last, "peer_transitions"),
    }
    (out_dir / ("%s-%s.summary.json" % (args.label, stamp))).write_text(
        json.dumps(summary, indent=1, ensure_ascii=False), encoding="utf-8")

    print("\n--- probes under load: %s ---" % args.label)
    for key, value in summary.items():
        print("  %-34s %s" % (key, value))
    print("samples: %s" % jsonl)
    if summary["takeovers"]:
        print("\nNOTE: the bus changed hands in this window — that is the finding, "
              "not the load figure beside it.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
