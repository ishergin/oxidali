import re, sys, datetime
CAP = re.compile(r'^(\S+) .*DALI sniff: .*capture=\[([0-9a-f ]+)\]')
HELD = re.compile(r'^(\S+) .*DALI line held: (\d+) run')
THRESHOLD = 24
def maxrun(hexs):
    bits = []
    for b in hexs.split():
        v = int(b, 16)
        bits += [(v >> i) & 1 for i in range(7, -1, -1)]
    best = cur = 0
    for b in bits:
        cur = cur + 1 if b == 0 else 0
        best = max(best, cur)
    return best
caps, helds = [], []
for line in open(sys.argv[1], errors="replace"):
    m = CAP.match(line)
    if m and m.group(1) >= sys.argv[2]:
        caps.append((datetime.datetime.fromisoformat(m.group(1)), maxrun(m.group(2))))
        continue
    m = HELD.match(line)
    if m and m.group(1) >= sys.argv[2]:
        helds.append(datetime.datetime.fromisoformat(m.group(1)))
big = [c for c in caps if c[1] >= THRESHOLD]
print(f"since {sys.argv[2]}: {len(caps)} dumped captures, {len(big)} with a dominant run >= {THRESHOLD} ticks")
print(f"  `DALI line held` lines: {len(helds)}")
W = datetime.timedelta(seconds=8)
unexplained = [c for c in big if not any(abs(h - c[0]) <= W for h in helds)]
for t, r in big[:6]:
    near = any(abs(h - t) <= W for h in helds)
    print(f"   {t.strftime('%H:%M:%S')} maxrun={r} ticks ({r*104/1000:.1f} ms)  held-line nearby: {near}")
if big and unexplained:
    print(f"  !! {len(unexplained)} of {len(big)} had NO held line — the probe's path is not firing")
elif big:
    print("  probe path CONFIRMED live by the wire itself")
else:
    print("  nothing long enough yet — inconclusive, keep waiting")
