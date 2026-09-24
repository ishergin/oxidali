from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import requests

DESCRIPTION = """Watches the pair while the operator pulls one unit's Ethernet cable.

Run it, unplug one unit when it says it is armed, leave it out for about ten
seconds, plug it back in. It samples both units once a second and passes when
the roles are unchanged across the outage, the standby recorded no takeover or
transition, the outage was real, and the standby kept answering probes through it."""

sys.path.insert(0, str(Path(__file__).resolve().parent))

from hil import config as config_mod
from hil.api import Client

SAMPLE_EVERY_S = 1.0
PROBE_TIMEOUT_S = 1.0
MIN_OUTAGE_S = 2.0


def _probe(client: Client) -> dict | None:
    try:
        base = client.cfg.base if hasattr(client, "cfg") else None
        health = requests.get("%s/api/v1/health" % base, timeout=PROBE_TIMEOUT_S).json()
        red = requests.get("%s/api/v1/redundancy" % base, timeout=PROBE_TIMEOUT_S).json()
        return {
            "role": health.get("role"),
            "uptime_s": health.get("uptime_seconds"),
            "active": red["active"],
            "owned": red["probes"]["owned"],
            "published": red["probes"]["published"],
            "takeovers": red["takeovers"],
            "stand_downs": red["stand_downs"],
            "transitions": len(red["transitions"]),
            "peer_unreachable": red["replication"]["peer_unreachable"],
        }
    except Exception:
        return None


def main() -> int:
    parser = argparse.ArgumentParser(description=DESCRIPTION)
    parser.add_argument("--minutes", type=float, default=8.0)
    args = parser.parse_args()

    cfg = config_mod.load()
    if not cfg.has_peer:
        raise SystemExit("no peer configured — this needs the pair")
    api = Client(cfg)
    peer = Client(cfg.peer())

    first_primary, first_peer = _probe(api), _probe(peer)
    if first_primary is None or first_peer is None:
        raise SystemExit("both units must answer before the cable is pulled "
                         "(primary %s, peer %s)" % (first_primary, first_peer))
    print("armed: primary %s (%s), peer %s (%s)"
          % (cfg.base, first_primary["role"], cfg.peer().base, first_peer["role"]))
    print("PULL an Ethernet cable now, leave it out ~10 s, plug it back in. "
          "Watching for %.0f min." % args.minutes)

    samples = []
    deadline = time.monotonic() + args.minutes * 60
    while time.monotonic() < deadline:
        now = time.monotonic()
        samples.append({"t": now, "primary": _probe(api), "peer": _probe(peer)})
        last = samples[-1]
        if last["primary"] is None or last["peer"] is None:
            print("  %.1fs  primary %s  peer %s"
                  % (now, "DOWN" if last["primary"] is None else "up",
                     "DOWN" if last["peer"] is None else "up"))
        time.sleep(SAMPLE_EVERY_S)

    out_dir = Path(cfg.state_dir) / "link_pull"
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    (out_dir / ("samples-%s.json" % stamp)).write_text(
        json.dumps(samples, indent=1, ensure_ascii=False), encoding="utf-8")

    def outage(side: str) -> tuple[float, float] | None:
        start = None
        best = None
        for sample in samples:
            if sample[side] is None:
                start = sample["t"] if start is None else start
            elif start is not None:
                span = (start, sample["t"])
                if best is None or (span[1] - span[0]) > (best[1] - best[0]):
                    best = span
                start = None
        if start is not None:
            span = (start, samples[-1]["t"])
            if best is None or (span[1] - span[0]) > (best[1] - best[0]):
                best = span
        return best

    def last_good(side: str) -> dict | None:
        for sample in reversed(samples):
            if sample[side] is not None:
                return sample[side]
        return None

    result = {}
    for side in ("primary", "peer"):
        span = outage(side)
        result["%s_outage_s" % side] = round(span[1] - span[0], 1) if span else 0.0
    end_primary, end_peer = last_good("primary"), last_good("peer")
    result["primary_role"] = "%s → %s" % (first_primary["role"],
                                          end_primary and end_primary["role"])
    result["peer_role"] = "%s → %s" % (first_peer["role"], end_peer and end_peer["role"])
    result["roles_unchanged"] = (end_primary and end_primary["role"] == first_primary["role"]
                                 and end_peer and end_peer["role"] == first_peer["role"])
    result["peer_takeovers_delta"] = (end_peer or {}).get("takeovers", 0) - first_peer["takeovers"]
    result["peer_transitions_delta"] = ((end_peer or {}).get("transitions", 0)
                                        - first_peer["transitions"])
    result["peer_unreachable_delta"] = ((end_peer or {}).get("peer_unreachable", 0)
                                        - first_peer["peer_unreachable"])
    result["peer_owned_delta"] = (end_peer or {}).get("owned", 0) - first_peer["owned"]
    result["primary_rebooted"] = bool(
        end_primary and end_primary["uptime_s"] < first_primary["uptime_s"])
    result["peer_rebooted"] = bool(end_peer and end_peer["uptime_s"] < first_peer["uptime_s"])

    span = outage("primary") or outage("peer")
    if span:
        before = [s for s in samples if s["t"] <= span[0] and s["peer"]]
        after = [s for s in samples if s["t"] >= span[1] and s["peer"]]
        if before and after:
            result["probes_owned_during_outage"] = (after[0]["peer"]["owned"]
                                                    - before[-1]["peer"]["owned"])

    print("\n--- a hand on the cable ---")
    for key, value in result.items():
        print("  %-30s %s" % (key, value))

    observed = max(result["primary_outage_s"], result["peer_outage_s"])
    if observed < MIN_OUTAGE_S:
        print("\nVERDICT: NOTHING WAS PULLED (longest outage %.1f s) — re-run and "
              "unplug a cable while it is armed." % observed)
        return 2
    passed = (result["roles_unchanged"] and result["peer_takeovers_delta"] == 0
              and result["peer_transitions_delta"] == 0
              and result.get("probes_owned_during_outage", 0) > 0)
    print("\nVERDICT: %s" % ("PASS — the network went away and the bus did not move"
                             if passed else "FAIL (see above)"))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
