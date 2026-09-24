# dali-arbiter — independent DALI wire witness

> **Targets an ESP32-C6 and is not ported to the ESP32-P4.** The firmware (an ESP-IDF
> C project for the C6) and its host-side decoder are the instrument's specification.

An edge recorder wired **receive-only** to the DALI bus, answering the question the
controller cannot answer about itself: is the frame it puts on the wire shaped the
way it should be? Why the firmware cannot judge its own transmit timing is in
[`08-dali-phy-and-transport.md`](../../documentation/architecture/08-dali-phy-and-transport.md);
this board is the independent observer it calls for.

Scope: the instrument's contract and build; its host-side decoder lives in the HIL
toolkit (below).

## The contract

- **Edges are captured by hardware** (the RMT peripheral latches pulse widths), and
  the capture path is IRAM-resident, so the instrument has none of the jitter it is
  measuring. A GPIO interrupt would add exactly that jitter.
- **The device decodes nothing.** It prints raw pulse widths, one line per frame,
  so the decode rules can be argued with and changed without a reflash:
  `E <seq> <t_end_us> <first_level> <d0> <d1> …` — `d0` starts at the first edge
  at `first_level`, levels alternate, and the last entry is the idle that ended
  reception. `#` lines are notes (banner, pin scan, periodic stats).
- **Never read its absolute numbers.** Its own opto path shortens every dominant
  pulse (by about 95 µs on the Pico-DALI2), so every measurement is a difference: a
  frame's first half-bit against the rest of that same frame, and our transmitter
  against a reference transmitter (the Wiren Board's master, or gear answers) in the
  same capture, so the bias cancels.
- **Receive only.** The TX line is parked recessive and never driven; a floating TX
  input could assert the bus on noise.

## Build (C6 wiring)

Pico-DALI2 receive output on GPIO 5, TX parked on GPIO 14. The boot banner prints a
transition count for every candidate GPIO, so a wrong pin does not look like a dead
bus.

```bash
source ~/.espressif/v5.5.3/esp-idf/export.sh
cd tools/dali-arbiter && idf.py set-target esp32c6 && idf.py build
idf.py -p <port> flash
```

## Host side

`hil/arbiter.py` parses, Manchester-decodes and scores captures (`capture`,
`summary`, `dump`). The `hil/issue27_*.py` scripts built on it compare our
transmitter with the Wiren Board's on the same wire (A/B by trial window), measure
the opening-half-bit deficit against idle time, and run a product-level loss soak.
