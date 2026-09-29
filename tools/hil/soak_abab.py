import argparse
import datetime
import json
import time
import urllib.request
from pathlib import Path

from hil import config as hil_config
from hil import seriallog, serialmon, wsclient
from hil.api import Client
from hil.provoke import NameRewrite, first_registered

HERE = Path(__file__).resolve().parent

STATS_TOTALS = (
    "isr_ticks_lost_total", "isr_ticks_extra_total", "isr_ticks_deficit_raw_total",
    "isr_ticks_surplus_raw_total", "isr_late_ticks_total",
    "answer_staged_total", "answer_stage_late_total", "sniff_poll_late_total",
    "persist_flush_total", "persist_flush_slow_total", "persist_flush_ms_total",
    "persist_gate_waits_total", "persist_gate_timeouts_total",
    "backward_undecodable_total", "backward_frame_size_total", "backward_incomplete_total",
)
STATS_GAUGES = ("isr_max_gap_us", "answer_stage_max_ticks", "sniff_poll_gap_max_us",
                "persist_flush_max_ms")
REDUNDANCY = ("answered", "window_closed", "late", "suppressed", "aborted")
PROBES = ("owned", "unowned")
SNIFFER = ("frames", "decode_failed", "unsupported_len")


def _get(base, path, timeout=6):
    with urllib.request.urlopen(base + path, timeout=timeout) as r:
        return json.load(r)


def sample(base):
    out = {}
    try:
        dali = _get(base, "/api/v1/stats").get("dali") or {}
        for k in STATS_TOTALS + STATS_GAUGES:
            if k in dali:
                out[k] = int(dali[k])
    except Exception as e:
        out["stats_error"] = str(e)
    try:
        diag = _get(base, "/api/v1/diagnostics")
        for k in SNIFFER:
            if k in (diag.get("phy_sniffer") or {}):
                out["sniff_" + k] = int(diag["phy_sniffer"][k])
    except Exception as e:
        out["diag_error"] = str(e)
    try:
        pair = _get(base, "/api/v1/redundancy")
        arbitration = pair.get("arbitration") or {}
        for k in REDUNDANCY:
            if k in arbitration:
                out["red_" + k] = int(arbitration[k])
        probes = pair.get("probes") or {}
        for k in PROBES:
            if k in probes:
                out["red_" + k] = int(probes[k])
    except Exception as e:
        out["redundancy_error"] = str(e)
    return out


def delta(before, after):
    d = {}
    for k, v in after.items():
        if not isinstance(v, int) or k not in before:
            continue
        if k in STATS_GAUGES:
            d[k] = v if v > before[k] else 0
        else:
            d[k] = (v - before[k]) & 0xFFFF_FFFF
    return d


def log_offset(log):
    try:
        return Path(log).stat().st_size
    except OSError:
        return 0


def late_entries(log, offset):
    try:
        fh = open(log, "rb")
    except OSError:
        return {}
    with fh:
        fh.seek(offset)
        return seriallog.late_entries(raw.decode("utf-8", "replace") for raw in fh).tasks


def run_phase(name, seconds, boards, provoke, patch_every):
    t0 = datetime.datetime.now().isoformat(timespec="seconds")
    before = {b: sample(base) for b, (base, _) in boards.items()}
    offsets = {b: log_offset(log) for b, (_, log) in boards.items()}
    end, patches, failures = time.time() + seconds, 0, 0
    while time.time() < end:
        if provoke:
            try:
                provoke()
                patches += 1
            except Exception:
                failures += 1
            time.sleep(patch_every)
        else:
            time.sleep(2)
    after = {b: sample(base) for b, (base, _) in boards.items()}
    time.sleep(6)
    t1 = datetime.datetime.now().isoformat(timespec="seconds")
    result = {"phase": name, "t0": t0, "t1": t1, "patches": patches,
              "patch_failures": failures}
    for b, (_, log) in boards.items():
        result[b] = delta(before[b], after[b])
        result[b + "_late_by_task"] = late_entries(log, offsets[b])
    return result


def print_phase(r):
    print("\n=== %s  %s..%s  patches=%d (failed %d) ===" % (
        r["phase"], r["t0"], r["t1"], r["patches"], r["patch_failures"]))
    for board in ("dut", "peer"):
        if board not in r:
            continue
        d = r[board]
        keys = sorted(k for k, v in d.items() if v)
        print("  %s: %s" % (board, ", ".join("%s=%d" % (k, d[k]) for k in keys) or "all zero"))
        late = r[board + "_late_by_task"]
        if late:
            print("  %s late entries: %s" % (board, ", ".join(
                "%s×%d(max %d us)" % (t, c, m)
                for t, (c, m) in sorted(late.items(), key=lambda kv: -kv[1][0]))))


def summarise(results):
    out = {}
    for r in results:
        side = r["phase"][0]
        for board in ("dut", "peer"):
            if board not in r:
                continue
            acc = out.setdefault((side, board), {})
            for k, v in r[board].items():
                acc[k] = max(acc.get(k, 0), v) if k in STATS_GAUGES else acc.get(k, 0) + v
    print("\n=== A vs B (summed over phases) ===")
    for board in ("dut", "peer"):
        a, b = out.get(("A", board)), out.get(("B", board))
        if a is None:
            continue
        keys = sorted(set(a) | set(b or {}))
        print("  %s:" % board)
        for k in keys:
            av, bv = a.get(k, 0), (b or {}).get(k, 0)
            if av or bv:
                print("    %-34s A=%-8d B=%d" % (k, av, bv))
    return {"%s/%s" % k: v for k, v in out.items()}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--phase-s", type=int, default=300)
    ap.add_argument("--cycles", type=int, default=4)
    ap.add_argument("--ws", type=int, default=2, help="WebSocket clients on the DUT")
    ap.add_argument("--patch-every", type=float, default=3.0)
    ap.add_argument("--no-peer", action="store_true")
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    cfg = hil_config.load()
    boards = {"dut": (cfg.base.rstrip("/"), serialmon.log_path(cfg))}
    if not args.no_peer:
        peer = cfg.peer()
        boards["peer"] = (peer.base.rstrip("/"), serialmon.log_path(peer))
    api = Client(cfg)
    short = first_registered(api, cfg.lamp_short_set())
    rewrite = NameRewrite(api, short) if short is not None else None
    print("soak: boards=%s provocation=%s, %d WS client(s)" % (
        {b: v[0] for b, v in boards.items()},
        "PATCH name of SA%d every %.1f s" % (short, args.patch_every) if rewrite
        else "none: no gear in HIL_LAMP_SHORTS=%s is registered" % cfg.lamp_shorts, args.ws))
    ws = wsclient.Subscribers(boards["dut"][0], args.ws) if args.ws else None
    results = []
    try:
        for cycle in range(args.cycles):
            for side, provoke in (("A", False), ("B", rewrite is not None)):
                name = "%s%d %s" % (side, cycle + 1, "PROVOKED" if provoke else "quiet")
                r = run_phase(name, args.phase_s, boards, rewrite if provoke else None,
                              args.patch_every)
                print_phase(r)
                results.append(r)
    finally:
        if ws:
            ws.close()
            for line in ws.errors:
                print("soak: %s — the load ran one client lighter from then on" % line)
        if rewrite is not None and not rewrite.restore():
            print("soak: SA%d's name is not back to %r — restore it by hand"
                  % (short, rewrite.original))
    summary = summarise(results)
    if args.out:
        Path(args.out).write_text(json.dumps({"phases": results, "summary": summary},
                                             ensure_ascii=False, indent=1))
        print("\nwritten: %s" % args.out)


if __name__ == "__main__":
    main()
