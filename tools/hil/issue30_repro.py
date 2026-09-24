import sys
import time

sys.path.insert(0, "/Users/ishergin/work/dali2rust/tools/hil")
from hil import api as api_mod
from hil import config as config_mod
from hil.foreign import ForeignMaster

ROUNDS = 20
TARGET_K = 2700


def probe(api):
    d = api.diagnostics()
    p = d["poller"]
    return {"sup": d["registry"]["runtime_updates_superseded"],
            "completed": p["reads_completed"],
            "preempted": p["reads_preempted"],
            "published": d["projector"]["runtime_updates_published"]}


def superseded(api):
    return probe(api)["sup"]


def main():
    cfg = config_mod.load()
    api = api_mod.Client(cfg)
    foreign = ForeignMaster(cfg)

    devices = api.devices()["physical_devices"]
    short = next((d["short_address"] for d in devices
                  if d.get("capabilities", {}).get("cct")), None)
    if short is None:
        raise SystemExit("no cct-capable lamp on the rig")
    print("using SA%02d" % short)

    before_poller = api.poller.get()
    print("poller before:", before_poller)
    api.poller.patch({"enabled": True, "interval_ms": 2000,
                      "include_dt8_color": True})
    print("poller now:", api.poller.get())

    prev = probe(api)
    s0 = prev["sup"]
    print("start:", prev)

    wins, losses, errors = 0, 0, 0
    try:
        for r in range(1, ROUNDS + 1):
            target = TARGET_K if r % 2 else 5000
            foreign.set_cct(short, target)
            time.sleep(2.6)

            st = api.state(short)["state"]
            k = st.get("color_temperature_kelvin")
            src = st.get("value_source")
            ok = k is not None and abs(k - target) <= 120
            wins += ok
            losses += (not ok)
            now = probe(api)
            print("round %2d: want=%-5s got=%-6s source=%-8s %-13s | reads+%d preempt+%d rruc+%d sup+%d"
                  % (r, target, k, src, "kept foreign" if ok else "LOST",
                     now["completed"] - prev["completed"],
                     now["preempted"] - prev["preempted"],
                     now["published"] - prev["published"],
                     now["sup"] - prev["sup"]))
            prev = now
    except Exception as exc:
        errors += 1
        print("error:", exc)
    finally:
        s1 = superseded(api)
        print("\nruntime_updates_superseded: %d -> %d (delta %d)" % (s0, s1, s1 - s0))
        print("rounds keeping the foreign colour: %d/%d" % (wins, wins + losses))
        api.poller.patch({"enabled": bool(before_poller.get("enabled")),
                          "interval_ms": before_poller.get("interval_ms", 5000),
                          "include_dt8_color": bool(before_poller.get("include_dt8_color"))})
        print("poller restored:", api.poller.get())
        api.off(short)


if __name__ == "__main__":
    main()
