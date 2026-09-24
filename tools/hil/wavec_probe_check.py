import sys
import time

from hil import api as api_mod
from hil import config as config_mod
from hil import sniffer as sniffer_mod

INTERVAL_MS = 3000
OBSERVE_S = 12.0


def poller_counters(api):
    return api._req("GET", "diagnostics").get("poller", {})


def main(argv):
    cfg = config_mod.load()
    api = api_mod.Client(cfg)
    tap = sniffer_mod.SnifferTap(cfg)
    tap.wait_ready()

    before_settings = api._req("GET", "settings/poller")
    print("== bench acceptance: broadcast health probe ==")
    print("poller before: enabled=%s interval_ms=%s"
          % (before_settings.get("enabled"), before_settings.get("interval_ms")))

    base = poller_counters(api)
    print("counters before: %s"
          % {k: v for k, v in base.items() if k.startswith("health_probes")})

    try:
        api._req("PATCH", "settings/poller",
                 {"enabled": True, "interval_ms": INTERVAL_MS})
        with tap.window() as win:
            time.sleep(OBSERVE_S)
            texts = [f.get("decoded") or "" for f in win.frames()]
        after = poller_counters(api)
    finally:
        api._req("PATCH", "settings/poller",
                 {"enabled": before_settings.get("enabled", False),
                  "interval_ms": before_settings.get("interval_ms", 5000)})

    print("counters after:  %s"
          % {k: v for k, v in after.items() if k.startswith("health_probes")})

    broadcast = [t for t in texts if "broadcast" in t.lower()]
    control = [t for t in broadcast if "QUERY CONTROL GEAR PRESENT" in t]
    probe = [t for t in broadcast if "QUERY LAMP FAILURE" in t]
    addressed = [t for t in texts if "short " in t]

    print("\n-- broadcast frames observed in %.0f s --" % OBSERVE_S)
    for t in broadcast[:12]:
        print("   %s" % t)
    print("   control=%d probe=%d   (addressed frames in the same window: %d)"
          % (len(control), len(probe), len(addressed)))

    published = after.get("health_probes_published", 0) - base.get("health_probes_published", 0)
    invalid = after.get("health_probes_invalid", 0) - base.get("health_probes_invalid", 0)
    clear = after.get("health_probes_clear", 0) - base.get("health_probes_clear", 0)
    one = after.get("health_probes_one_failure", 0) - base.get("health_probes_one_failure", 0)
    several = after.get("health_probes_several_failures", 0) - base.get("health_probes_several_failures", 0)

    print()
    ok = True

    if published >= 1:
        print("PASS  %d probe(s) published in %.0f s at interval %d ms"
              % (published, OBSERVE_S, INTERVAL_MS))
    else:
        print("FAIL  the enabled poller published no probe")
        ok = False

    if control and probe:
        print("PASS  both frames are broadcast: control + lamp-failure query")
    else:
        print("FAIL  expected a broadcast control AND a broadcast lamp-failure "
              "query; control=%d probe=%d" % (len(control), len(probe)))
        ok = False

    if invalid == 0:
        print("PASS  every probe's positive control was answered (§3.13 Note 1)")
    else:
        print("NOTE  %d probe(s) reported invalid — the broadcast QUERY CONTROL "
              "GEAR PRESENT went unanswered, so those readings are void rather "
              "than clean. Worth a look if it is not transient." % invalid)

    print("      verdicts: clear=%d one_failure=%d several_failures=%d"
          % (clear, one, several))
    if several:
        print("NOTE  `several` on this bench is unexpected — the C6 fleet is "
              "bit-aligned by construction, so a violation here came from the "
              "real luminaires and is worth capturing.")

    print("\n-- cost --")
    print("   probe frames: %d   addressed (device sweep) frames: %d"
          % (len(control) + len(probe), len(addressed)))
    print("   §16.2.1's point: the sweep is not replaced by the probe, and this "
          "ratio is why.")

    print("\n%s" % ("ACCEPTED" if ok else "NOT ACCEPTED"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
