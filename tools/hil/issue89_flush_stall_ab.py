import json, re, subprocess, sys, time, urllib.request, datetime

DUT = "192.168.13.116"
LOG = "/Users/ishergin/work/dali2rust/tools/hil/state/persist/serial.log"
TIMING = re.compile(r'^(\S+) .*DALI sniff timing:.*stage max=(\d+) ticks over budget=(\d+) of (\d+) poll gap max=(\d+) us')
FLUSH = re.compile(r'^(\S+) .*slow flush_dirty_slices took (\d+) ms')

def counters():
    d = json.load(urllib.request.urlopen(f"http://{DUT}/api/v1/diagnostics", timeout=6))
    r, s = d["redundancy"], d["phy_sniffer"]
    return {k: r[k] for k in ("answered", "window_closed", "late", "suppressed", "aborted")} | \
           {"failed": s["decode_failed"], "unsup": s["unsupported_len"], "frames": s["frames"]}

def patch():
    urllib.request.urlopen(urllib.request.Request(
        f"http://{DUT}/api/v1/adapters/0/physical-devices/0",
        data=b'{"name":""}', method="PATCH",
        headers={"Content-Type": "application/json"}), timeout=8).read()

def window(t0, t1):
    stages, overs, totals, gaps, flushes = [], 0, 0, [], []
    for line in open(LOG, errors="replace"):
        m = TIMING.match(line)
        if m and t0 <= m.group(1) <= t1:
            stages.append(int(m.group(2))); overs += int(m.group(3))
            totals += int(m.group(4)); gaps.append(int(m.group(5)))
            continue
        m = FLUSH.match(line)
        if m and t0 <= m.group(1) <= t1:
            flushes.append(int(m.group(2)))
    return stages, overs, totals, gaps, flushes

def phase(name, seconds, provoke):
    t0 = datetime.datetime.now().isoformat(timespec="seconds")
    before = counters()
    end = time.time() + seconds
    n = 0
    while time.time() < end:
        if provoke:
            try: patch(); n += 1
            except Exception: pass
            time.sleep(1.5)
        else:
            time.sleep(2)
    after = counters()
    time.sleep(6)
    t1 = datetime.datetime.now().isoformat(timespec="seconds")
    st, ov, tot, gp, fl = window(t0, t1)
    d = {k: after[k] - before[k] for k in before}
    print(f"\n=== {name} ({seconds}s, {n} patches) ===")
    print(f"  probes answered {d['answered']}, window_closed {d['window_closed']}, late {d['late']}, "
          f"suppressed {d['suppressed']}, aborted {d['aborted']}")
    print(f"  bad captures: failed {d['failed']}, unsup {d['unsup']}  (frames {d['frames']})")
    if st:
        st.sort(); gp.sort()
        print(f"  stage ticks: p50 {st[len(st)//2]} max {max(st)} | over budget {ov} of {tot}")
        print(f"  poll gap us: p50 {gp[len(gp)//2]} max {max(gp)}")
    print(f"  slow flushes (>250ms): {len(fl)}  total {sum(fl)} ms  max {max(fl) if fl else 0} ms")
    return d, ov, tot

if __name__ == "__main__":
    s = int(sys.argv[1]) if len(sys.argv) > 1 else 150
    phase("A  QUIET (no patches)", s, False)
    phase("B  PROVOKED (flush every 1,5 s)", s, True)
