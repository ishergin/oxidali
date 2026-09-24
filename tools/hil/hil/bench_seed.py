import json
import sys

from hil import config, identity
from hil.api import Client

USAGE = """usage: python3 -m hil.bench_seed save|restore FILE

Seeds a volatile-registry build (DALI2RUST_PERSIST_DISABLE=1): `save` on the
persisting build while the registry is populated, flash the diagnosis build,
`restore` (discovery, virtual-lamp bindings, group matrix), then run the suite.
`restore` is idempotent."""

DISCOVERY_MODE = "scan_known_short_addresses"
DISCOVERY_TIMEOUT_S = 180


def _client() -> Client:
    return Client(config.load())


def save(path: str) -> int:
    api = _client()
    snap = api.config_snapshot()
    with open(path, "w") as fh:
        json.dump(snap, fh)
    scenes = snap.get("scenes") or []
    print("saved %d virtual lamps, %d matrix rows, %d/%d non-empty scenes -> %s"
          % (len(snap["vl"]["virtual_lamps"]),
             len(snap["group_matrix"]["rows"]),
             sum(1 for s in scenes if s["rows"]), len(scenes), path))
    return 0


def restore(path: str) -> int:
    api = _client()
    with open(path) as fh:
        snap = json.load(fh)

    view = api.wait_op(api.discovery(DISCOVERY_MODE), timeout_s=DISCOVERY_TIMEOUT_S)
    if view.get("status") != "succeeded":
        print("discovery did not succeed: %s" % view.get("status"), file=sys.stderr)
        return 1
    found = len(api._req("GET", "adapters/%d/physical-devices" % api.adapter)
                .get("physical_devices", []))

    identified = sum(
        1 for lamp in identity.collect(api, ensure_read=True)
        if lamp.gtin is not None and lamp.identification_number is not None
    )

    api.config_restore(snap)
    print("discovery ok (%d physical devices, %d with identity), config restored"
          % (found, identified))
    if identified == 0:
        print("WARNING: no lamp identity recovered — the suite will still skip",
              file=sys.stderr)
        return 1
    return 0


def main(argv) -> int:
    if len(argv) != 3 or argv[1] not in ("save", "restore"):
        print(USAGE, file=sys.stderr)
        return 2
    return save(argv[2]) if argv[1] == "save" else restore(argv[2])


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
