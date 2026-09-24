#!/usr/bin/env python3

import sys

from hil import api as api_mod, config as cfg_mod

RGBWAF_SHORT = 1
EXPECTED_FEATURES = 0x62

DT8_QUERIES = (
    (251, "RGBWAF CONTROL"),
    (249, "COLOUR TYPE FEATURES"),
    (248, "COLOUR STATUS"),
    (247, "GEAR FEATURES/STATUS"),
)
ENABLE_DEVICE_TYPE_8 = (0xC1 << 8) | 8


def dt8_query(client, short: int, opcode: int):
    client.raw(ENABLE_DEVICE_TYPE_8, expects_backward=False)
    answer = client.raw(((short * 2 + 1) << 8) | opcode, expects_backward=True)
    return answer.get("backward_frame") if answer.get("success") else None


def main() -> int:
    client = api_mod.Client(cfg_mod.load())
    values = {}
    print(f"SA{RGBWAF_SHORT} — DT8 colour bytes, read with the prelude:")
    for opcode, label in DT8_QUERIES:
        value = dt8_query(client, RGBWAF_SHORT, opcode)
        values[opcode] = value
        shown = "NO ANSWER" if value is None else f"{value} (0x{value:02X})"
        print(f"  {opcode:>3}  {label:<24} {shown}")

    features = values.get(249)
    if features != EXPECTED_FEATURES:
        print(f"\n!! COLOUR TYPE FEATURES is {features}, expected "
              f"{EXPECTED_FEATURES} (0x62). This may not be the RGBWAF fixture "
              f"any more — check which short address it holds before reading "
              f"anything into 251.")

    control = values.get(251)
    if "--after-power-cycle" not in sys.argv:
        print(f"\n251 = {control!r} — BASELINE only. No verdict: this script "
              f"cannot tell whether the fixture lost power, and the answer only "
              f"means something if it did. Re-run as\n"
              f"    .venv/bin/python3 rgbwaf_control_probe.py --after-power-cycle\n"
              f"once SA{RGBWAF_SHORT} has been off the mains and back, before "
              f"anything writes colour.")
        return 0

    verdict = {
        255: "vendor default is MASK — 0xC0 on 2026-08-13 was MASK with the "
             "channel bits cleared by our own Tc writes. Premise confirmed.",
        63:  "the gear holds Table 10's documented default (all channels "
             "linked) — so 0xC0 came from somewhere else and needs explaining.",
        128: "the byte survived the power cycle: not `1 byte RAM` on this "
             "vendor. Record it — the no-op-RGB-write argument assumes it is.",
    }.get(control)
    print(f"\n251 = {control!r}: {verdict or 'unexpected value — record it verbatim.'}")
    print("Now write the reading into documentation/reference/"
          "iec62386-conformance-gaps.md §4.2.1 and close the open item.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
