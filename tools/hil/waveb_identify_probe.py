import sys
import time

from hil import api as api_mod
from hil import config as config_mod
from hil import sniffer as sniffer_mod

LEVEL = 120
QUERY_STATUS = 0x90
QUERY_ACTUAL_LEVEL = 0xA0
POWER_CYCLE_SEEN = 0x80
FORBIDDEN = ("DAPC", "RECALL MAX", "RECALL MIN", "OFF", "GO TO LAST ACTIVE LEVEL")


def query(api, short, opcode):
    frame = (((short << 1) | 1) << 8) | opcode
    resp = api.raw(frame, expects_backward=True)
    if not resp.get("success"):
        return None
    return resp.get("backward_frame")


def main(argv):
    short = int(argv[1]) if len(argv) > 1 else 0
    cfg = config_mod.load()
    api = api_mod.Client(cfg)
    tap = sniffer_mod.SnifferTap(cfg)
    tap.wait_ready()

    print("== bench acceptance: IDENTIFY DEVICE on short %d ==" % short)

    api.dapc(short, LEVEL)
    time.sleep(1.5)
    before_status = query(api, short, QUERY_STATUS)
    before_level = query(api, short, QUERY_ACTUAL_LEVEL)
    print("before: actual_level=%s status=%s powerCycleSeen=%s"
          % (before_level,
             None if before_status is None else hex(before_status),
             None if before_status is None else bool(before_status & POWER_CYCLE_SEEN)))

    with tap.window() as win:
        time.sleep(1.0)
        resp = api.identify(short)
        print("identify accepted: %s" % resp.get("status"))
        view = api.wait_op(resp, timeout_s=30)
        print("operation: %s  mechanism=%s"
              % (view.get("status"),
                 (view.get("result") or {}).get("identify_mechanism")))
        time.sleep(2.0)
        texts = [f.get("decoded") or "" for f in win.frames()]

    mine = "short %d" % short
    ours = [t for t in texts if mine in t]
    print("\n-- frames addressed to short %d during identify --" % short)
    for t in ours:
        print("   %s" % t)
    if not ours:
        print("   (none decoded)")

    identify_frames = [t for t in ours if "IDENTIFY DEVICE" in t]
    level_frames = [t for t in ours if any(f in t for f in FORBIDDEN)]

    after_status = query(api, short, QUERY_STATUS)
    after_level = query(api, short, QUERY_ACTUAL_LEVEL)
    print("\nafter:  actual_level=%s status=%s powerCycleSeen=%s"
          % (after_level,
             None if after_status is None else hex(after_status),
             None if after_status is None else bool(after_status & POWER_CYCLE_SEEN)))

    print()
    ok = True

    if len(identify_frames) == 2:
        print("PASS  the 0x25 send-twice pair is on the wire (§12.1.3.3)")
    else:
        print("FAIL  expected exactly two IDENTIFY DEVICE frames, saw %d"
              % len(identify_frames))
        ok = False

    if level_frames:
        print("FAIL  §9.14.3.1: identification affects no variables, but a level "
              "command was sent: %s" % level_frames)
        ok = False
    else:
        print("PASS  no level command anywhere in the identify sequence")

    if before_status is None or after_status is None:
        print("FAIL  QUERY STATUS unanswered — cannot judge powerCycleSeen")
        ok = False
    elif (before_status & POWER_CYCLE_SEEN) != (after_status & POWER_CYCLE_SEEN):
        print("FAIL  powerCycleSeen changed across identify — §9.16.9")
        ok = False
    else:
        print("PASS  powerCycleSeen survived identify (§9.16.9)")

    if before_level is None or after_level is None:
        print("FAIL  QUERY ACTUAL LEVEL unanswered — cannot judge the level")
        ok = False
    elif before_level != after_level:
        print("NOTE  actual level moved %s -> %s. Not necessarily a defect: "
              "§9.14.3.1 lets the light output sit anywhere while the procedure "
              "runs, and it is restored when the procedure ends (10 s ± 1 s)."
              % (before_level, after_level))
    else:
        print("PASS  actual level unchanged")

    api.off(short)
    print("\n%s" % ("ACCEPTED" if ok else "NOT ACCEPTED"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
