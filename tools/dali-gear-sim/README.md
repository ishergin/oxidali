# dali-gear-sim — a board that answers as ~54 DALI control gear

> **Targets an ESP32-C6 and is not ported to the ESP32-P4.** It is pinned to the C6:
> the `riscv32imac` target (the P4 is `imafc`), `MCU`, `sdkconfig.c6.defaults`,
> `partitions-c6.csv` and the C6 pin map. `just gear-sim-check` proves only that this
> crate still compiles against the workspace crates it shares (gear model, codec, PHY,
> domain, platform), not that any hardware works.

A load-test instrument. On a real DALI segment it answers as a fleet of virtual
control gear (DT6, DT8 Tc, DT8 RGB+Tc) so the controller meets a full bus: the
whole commissioning search, a poller sweep over every address, group and broadcast
applies, a registry at its 64-address ceiling. It is one bus load (~2 mA) however
many gear it emulates.

Scope: the instrument, its build and its console protocol. The gear semantics are
`dali2rust-gear-model`, shared with the host dev server's simulated bus; the HIL
toolkit's console client is `tools/hil/hil/gearsim.py`.

## The one rule: never share an address with a real lamp

Two devices answering one short address do not degrade the bus, they kill it. The
model refuses the reserved block (`DEFAULT_RESERVED_SHORT_ADDRESSES`, 0..=9) at
construction and at `PROGRAM SHORT ADDRESS`; the default fleet (27 DT6, 18 Tc,
9 RGB+Tc) fills 10..=63. The TX pad is parked recessive before anything else runs,
and a fleet with no stored history comes up **disabled**. The reserved block must
cover every real lamp on the wire the fleet joins — check it against that wire's
addresses before enabling a single gear.

## Build

A separate cargo workspace: its `.cargo/config.toml` overrides the root's forced
`MCU` and sdkconfig for builds started in this directory. Nothing in `cargo fw`,
`cargo flash` or `hil flash` builds it.

```bash
cd tools/dali-gear-sim && cargo build   # `cargo run` flashes and monitors (debug profile)
just gear-sim-check                     # type-check only
just gear-sim-isr-iram-check            # build, then the ISR-IRAM gate on the linked binary
```

It writes its fleet to flash, so its PHY interrupt must reach no flash-resident
code: run the ISR-IRAM gate before it goes on a wire. C6 wiring (ESP32-C6-Pico +
Pico-DALI2): DALI TX on GPIO 14, inverted (pad high = bus active); DALI RX on GPIO 5.

## Serial protocol

One port carries the event log and the console. Data lines start with a token,
everything else with `#`, so a reader parses the stream without guessing:

```
C <t_us> A05 level 0 -> 180            a gear's state changed
F <t_us> fe 90 Broadcast QUERY STATUS  a forward frame was heard     (log frame)
B <t_us> 84 n=1 t=58                   a backward frame was sent     (log frame)
B <t_us> -- n=3 collision              several gear answered differently
E <t_us> window_missed idle_ticks=95   something went wrong
# ...                                  notes, banners, console replies
```

`t=` is the idle-tick count the answer was submitted at, which the `stats`
histogram summarises. A console reply is a run of `#` lines and ends when they stop.
Opening the port reboots a C6, so a client opens it once per session;
`HIL_GEAR_SIM_PORT` names it and is never auto-detected.

| Command | |
| --- | --- |
| `show [addr]` | the fleet table, or one gear |
| `stats` | counters, the submit-timing histogram, forward frames per IEC 62386-101 Table 22 priority, violations (`enable_consumed`, `send_twice_interloper`) |
| `base <addr>` | where the next `fleet` starts; reserved addresses are skipped |
| `fleet <dt6> <cct> <rgb>` | rebuild from the base address up (comes up disabled) |
| `unaddress <count>` | take the short address off that many gear, leaving them as factory-fresh drivers; `fleet` restores them |
| `enable\|disable <addr\|all>` | put gear on or off the bus |
| `autoact <addr> on\|off` | the DT8 Automatic Activation bit (RAM, not saved) |
| `metering <addr> energy\|diagnostics\|both\|off` | DiiA Part 252/253 banks 202–207 and the device types that announce them |
| `luminaire <addr> off\|3\|4\|5\|bus-unit-on\|bus-unit-off` | memory bank 1's DiiA Part 251 extension at content format 3–5, and bank 0's `0x1B`/`0x1C` |
| `fail <addr> lamp\|none\|short\|open\|thermal\|derate\|<byte>` | `lamp` sets 102 status bit 1 alone; the rest set the Part 207 failure byte and let the model derive what follows |
| `drop <addr> <permille>` | withhold that fraction of answers |
| `log off\|change\|frame\|trace` | verbosity; keep `change` during a load run, `frame` makes the console the bottleneck |
| `save` / `load` / `erase` | the stored fleet |
| `reboot` | |

## What it models

Implemented: the commissioning search (INITIALISE, RANDOMISE, COMPARE, WITHDRAW,
PROGRAM/VERIFY SHORT ADDRESS), DAPC and the arc-power commands, group membership,
the 16-scene table, level limits and fade registers, the status byte, memory banks
0 and 1 with DTR0 auto-increment, and DT8 colour with the IEC 62386-209 staging
rule; of the DT6 extended commands, the dimming curve and the Part 207 failure
queries. What the model leaves out is listed in
[`05-testing-and-bdd.md`](../../documentation/architecture/05-testing-and-bdd.md)
(*Host models and the mock transport*). An unknown extended opcode is ignored, so
`log trace` shows what a controller actually sends.

## Timing, and how far to trust it

A gear starts its backward frame 5.5–10.5 ms after the forward frame ends. The
emulator aims at 6.5 ms, submitting its answer at the matching PHY idle-tick count
less the transmitter's arming lead, and asserts both bounds at compile time
(`src/answer.rs` has the derivation), so a change to the PHY's lead fails the
build. The `stats` histogram is a self-report on its own clock. The independent
check is to read an emulated gear from the Wiren Board's master; edge-level
evidence needs the wire witness in [`tools/dali-arbiter`](../dali-arbiter/README.md),
which has no board either.

## Bring-up order

1. Flash. The fleet is disabled and transmits nothing; confirm from the log that it
   hears and describes the traffic on the wire.
2. `enable` one gear, query it from the controller (`hil api cmd`), and check that
   `stats` puts the answers inside the window.
3. Read the same gear from the Wiren Board's master.
4. Scale up: poller sweep, group apply, a full registry.
5. Commissioning re-addresses every gear on the wire, real lamps included: only on a
   wire where that is allowed, one test at a time.
