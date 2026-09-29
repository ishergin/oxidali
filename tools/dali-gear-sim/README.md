# dali-gear-sim — a board that answers as a fleet of DALI control gear

A bench instrument that runs on the redundancy pair's second ESP32-P4-ETH, borrowed for a
session. On the shared DALI wire it answers as a fleet of virtual control gear (DT6, DT8
Tc, DT8 RGB+Tc), so tests drive gear that is nobody's light: target states, groups,
scenes, HCL, a commissioning search, a poller sweep, a registry near its ceiling. It is
one bus load (~2 mA) however many gear it emulates.

Scope: the instrument, its build and its console protocol. The gear semantics are
`dali2rust-gear-model`, shared with the host dev server's simulated bus. Lending the
peer to the emulator and the HIL tier that drives the fleet live in
[`tools/hil`](../hil/README.md#virtual-gear).

## The one rule: never share an address with a real lamp

Two devices answering one short address do not degrade the bus, they kill it. So every
boot starts with **no fleet and no reserve**, and nothing is stored in flash. `reserve`
names every short address a real lamp holds on this wire: the controller's registry,
the Wiren Board master's device list and unpowered lamps a scan cannot see. `fleet` and
`enable` refuse until it is set, and a fleet asks for at most 64 − |reserve| addresses.
The model also refuses reserved addresses at construction and at `PROGRAM SHORT ADDRESS`.

## Build

A separate cargo workspace. Its `.cargo/config.toml` overrides the root's forced `MCU`
and sdkconfig for builds started in this directory. `sdkconfig.gear-sim.defaults` is
layered on the root's `sdkconfig.p4.defaults` and changes only what differs:
- the path to the root's `partitions-p4.csv` — one partition table per board, so the
  image fits the pair's OTA slot;
- no secondary console;
- `WARN` logs;
- no core dump on the console.

```bash
cd tools/dali-gear-sim && cargo build
just gear-sim-check                     # type-check against the shared crates
just gear-sim-isr-iram-check            # build, then the ISR-IRAM gate on the linked binary
```

The bench's boards hang on the Wiren Board's USB; the image reaches the peer only
through the HIL toolkit.

## Board and answer path

ESP32-P4-ETH + Pico-DALI2, wired like the controller: DALI TX on GPIO 14, inverted (pad
high = bus active); DALI RX on GPIO 17. The console is UART0 through the board's USB-UART.

The PHY is the firmware's `dali2rust-dali-phy`, on the gptimer driver path at interrupt
priority 3; [ADR-025](../../documentation/architecture/decisions/ADR-025-phy-interrupt-above-critical-sections.md)
keeps the emulator off the level-5 path. The interrupt runs on core 0, where it is
registered, and every emulator task is pinned to core 1, so critical sections on the
interrupt's core stay rare.

A decoded forward frame goes to the fleet in task context. The fleet's answer is staged
in the PHY's answer cell, and the interrupt sends it inside the IEC 62386-101 Table 20
window (`backward_window.rs`) — the mechanism the controller uses for its arbitration
answer. Several answering gear merge into one frame by OR-ing their dominant half-bits,
which is what the wired-AND bus does.

## Serial protocol

One port carries the event log and the console through one output path: the UART
driver behind the VFS, fed by a bounded queue with one writer, so the gear task never
waits on the UART. Data lines start with a token, everything else with `#`, so a reader
parses the stream without guessing:

```
# ready build=<sha> slot=<label> state=<token>   the console accepts commands (and `ready`)
C <t_us> A16 level 0 -> 180                     a gear's state changed
F <t_us> fe 90 Broadcast QUERY STATUS            a forward frame was heard     (log frame)
B <t_us> 84 n=1                                  an answer was staged          (log frame)
B <t_us> -- n=3 collision                        several gear answered differently
E <t_us> answer_cell_busy ...                    something went wrong
```

`state` is the running slot's OTA state as the bootloader keeps it:
- `pending_verify` — installed over OTA; any reset hands the board back to the controller;
- `none` or `valid` — written by wire; the role survives resets. `valid` means the slot kept
  the confirmation a controller wrote there before the emulator replaced it;
- `new`, `invalid`, `aborted`, `undefined` — anything else.

`C` lines carry the short and random address, `enabled`, groups, level, `identify`,
colour mode, mirek, xy, the RGBWAF channels, the level range, the Tc limits and the
scene levels. Lines the queue drops show up as `log_dropped` in `stats`.

| Command | |
| --- | --- |
| `ready` | the ready line again, for a reader that missed the boot |
| `reserve <a-b,c ...>` | the short addresses the fleet never holds; once per boot, before `fleet`; a second one is refused |
| `fleet <base> <dt6> <cct> <rgb>` | build the fleet from `base` upward, skipping the reserve; all disabled |
| `enable\|disable <addr\|all>` | put gear on or off the bus; enabling an address no gear holds is refused |
| `show [addr]` | the fleet table, or one gear |
| `stats` | loop counters; the answer cell (`sent`, `stale`, `expired`, `late`, `rejected`, `collided`); late ticks; forward frames per IEC 62386-101 Table 22 priority; violations (`enable_consumed`, `send_twice_interloper`) |
| `unaddress <count>` | take the short address off that many gear, leaving them as factory-fresh drivers |
| `autoact <addr> on\|off` | the DT8 Automatic Activation bit |
| `metering <addr> energy\|diagnostics\|both\|off` | DiiA Part 252/253 banks 202–207 and the device types that announce them |
| `luminaire <addr> off\|3\|4\|5\|bus-unit-on\|bus-unit-off` | memory bank 1's DiiA Part 251 extension at content format 3–5, and bank 0's `0x1B`/`0x1C` |
| `fail <addr> lamp\|none\|short\|open\|thermal\|derate\|<byte>` | `lamp` sets 102 status bit 1 alone; the rest set the Part 207 failure byte and let the model derive what follows |
| `drop <addr> <permille>` | withhold that fraction of answers |
| `log off\|change\|frame\|trace` | verbosity; keep `change` during a load run, `frame` makes the console the bottleneck |
| `reboot` | restart; under the OTA role this hands the board back to the controller |

How to read the answer cell on a shared wire:
- `stale` counts both a frame that moved the epoch before the answer went out and a
  yield to another transmitter, so it is non-zero wherever real gear or another master
  share the wire;
- `collided` stays 0 by construction: a backward frame is exempt from collision
  detection;
- `late`, `expired` and the late ticks are the timing verdicts.

## What it models

Implemented:
- the commissioning search (INITIALISE, RANDOMISE, COMPARE, WITHDRAW, PROGRAM/VERIFY
  SHORT ADDRESS);
- DAPC and the arc-power commands;
- group membership and the 16-scene table;
- level limits and fade registers;
- the status byte;
- memory banks 0 and 1 with DTR0 auto-increment;
- DT8 colour with the IEC 62386-209 staging rule;
- of the DT6 extended commands, the dimming curve and the Part 207 failure queries.

What the model leaves out is listed in
[`05-testing-and-bdd.md`](../../documentation/architecture/05-testing-and-bdd.md)
(*Host models and the mock transport*). An unknown extended opcode is ignored, so
`log trace` shows what a controller actually sends.

## Timing, and how far to trust it

A gear starts its backward frame 5.5–10.5 ms after the forward frame ends. The interrupt
decides when the answer starts. The emulator only has to stage it before the interrupt
arms it, `ANSWER_ARM_TARGET_IDLE_TICKS` after the forward frame ends, while the
capture completes after `RX_IDLE_LINE_HIGH_TICKS` (both in `dali2rust-dali-phy`); the
ring is polled once per tick.
`late`, `expired` and the late ticks are the PHY's reports on its own clock.

The independent check is to read an emulated gear from the Wiren Board's master.
Edge-level evidence needs a wire witness ([`tools/dali-arbiter`](../dali-arbiter/README.md)),
which has no board.

## Bring-up order

1. Put the image on the peer (`hil --peer role gear-sim`). The fleet is empty and
   transmits nothing; confirm from the log that it hears the traffic on the wire
   (`log frame`).
2. `reserve` every live address, `fleet`, `enable` one gear, query it from the
   controller, and check that `stats` shows it sent with no late ticks.
3. Read the same gear from the Wiren Board's master.
4. Scale up: four gear, then the tier's fleet; a poller sweep, a group apply.
5. Commissioning follows [`tools/hil/STRATEGY.md`](../hil/STRATEGY.md) §4.
