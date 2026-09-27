# ADR-031: The gear emulator borrows the standby, on the installation's wire

Status: Accepted
Date: 2026-09-26

## Context

The bench is a production installation: every lamp on the wire is somebody's light, and a
visible change needs the owner's go-ahead for that run. Tests that drive light — target
states, groups, scenes, HCL, commissioning — therefore piled up as bench debt. The gear
emulator ([ADR-014](ADR-014-gear-model-and-second-dali-endpoint.md)) answers as a fleet of
control gear, but its board is gone. The only other board is the redundancy pair's
standby ([ADR-018](ADR-018-controller-redundancy.md)), on the same wire as the owner's
lamps, powered and reached through the Wiren Board. A Wiren Board reboot takes both boards
with it, and the Mac that runs the toolkit is not always on the installation's network.

## Decision

1. **The standby is lent to the emulator for a session, in one of two ways.** Both run
   through one toolkit command, `hil --peer role`:
   - **OTA:** the emulator's application image goes into the standby's inactive slot through
     the controller's own update route (ADR-024). The emulator never confirms itself, so the
     bootloader rolls it back at the next reset of any kind. Its ready line must report the
     slot `pending_verify`; anything else ends the switch, and `new` means the bootloader
     kept the image, so the operator writes the controller back by wire.
   - **Wired:** the merged image, built with the controller's bootloader and partition table,
     is written by esptool running on the Wiren Board itself. The role survives resets, and
     the controller is written back the same way.
2. **The emulator stores nothing.** Every boot starts with no fleet and no reserve, and
   `fleet` and `enable` refuse until the reserve names every address a real lamp holds.
3. **A session is an envelope around the tests.** Its rules are
   [`tools/hil/STRATEGY.md`](../../../tools/hil/STRATEGY.md) §4.8:
   - the reserve covers every address a real lamp may hold, and the fleet parks above it;
   - the session uses only groups no live lamp holds, proven silent on the wire;
   - the toolkit refuses every request and frame that is not on the tier's list, and a
     tripwire judges the controller's own transmit log with the same list after every test;
   - Home Assistant never sees a session lamp or group;
   - a ledger written before the first registry write drives the teardown, and the standby
     returns only after it is clean, its registry then compared with the controller's.
4. **Commissioning runs on the installation only onto emulated gear.** It uses the expert
   steps with an explicit park address, never the product's unaddressed commissioning,
   which picks the lowest free address. A separate flag and a go-ahead per run are required,
   and a pre-check must find no live gear without a short address.

### Rejected alternatives

- **A console over Ethernet on the emulator** — a second network identity on the wire and
  a network stack in a tool that only needs a UART.
- **Marking the emulator valid after an OTA** — the standby would stay lent after a Wiren
  Board reset, with nobody to hand it back.
- **espflash on the Wiren Board** — its aarch64 release needs a newer glibc than the
  board's system provides; esptool is pure Python and the board already has pyserial.
- **Commissioning with the product's unaddressed mode** — it hands the first emulated gear
  an address inside the reserve, which the Wiren Board master polls.

## Consequences

- While the standby is lent there is no failover: the active controller keeps the wire,
  and the redundancy tier and its acceptance scripts cannot run.
- A Wiren Board reset during an OTA session hands the standby back before the teardown.
  The run is inconclusive; the teardown still runs from the ledger, and the standby's
  registry is checked afterwards.
- The emulator's `C` lines are an oracle independent of the controller's registry, but they
  report the model's state, not light; optical and fade behaviour stay on real lamps.
- The tripwire cannot tell who sent a frame, so the owner's own use of the lights during a
  test stops the session.
- The runbook is [`tools/hil/README.md`](../../../tools/hil/README.md#virtual-gear); the safety
  rules are [`tools/hil/STRATEGY.md`](../../../tools/hil/STRATEGY.md) §4.
