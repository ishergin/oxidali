# ADR-014: One gear model for every simulated DALI endpoint

Status: Accepted
Date: 2026-08-06

## Context

A few real luminaires prove that a command reaches a fixture and little else: the
commissioning search ends in a few steps, a poller sweep is cheap, and the registry sits
far below its 64 addresses. Load-shaped properties — a full bus against the single httpd
task, the wire-priority table under sixty reads, the registry's memory — need a device
that answers as many control gear. The observed half of the product (the sniffer
translator, the state fan-out from foreign frames, Part 103 input events and every
rule-engine input trigger) needs another master on the wire.

The repository already holds the pieces, built for the other side of the wire: the PHY
FSM is not master-shaped, the Manchester codec produces backward frames, and the host
simulator implements the IEC 62386-102/207/209 slave side. A gear emulator on a second
MCU needs all three, and it cannot be a member of the root workspace, whose `MCU` knob is
forced to the P4 ([ADR-011](ADR-011-retire-esp32s3.md)).

## Decision

1. **Gear semantics and the codec are shared crates.** `dali2rust-gear-model` holds the
   gear fleet (depending only on `dali2rust-domain` and `dali2rust-platform`) and, for
   Part 103, a 24-bit `DeviceFleet` with push buttons, occupancy sensors and Part 332
   feedback. `dali2rust-dali-codec` holds Manchester encode/decode and depends only on
   the PHY crate. `dali2rust-adapters` keeps a re-export shim and a thin
   `SimDaliTransport`, and `gear-model` is a host-only dependency there: a model of a gear
   does not belong in the firmware image.
2. **A hardware emulator is its own cargo workspace under `tools/`.** Its own
   `.cargo/config.toml` overrides `MCU` and `ESP_IDF_SDKCONFIG_DEFAULTS` for builds
   started there, with the same `force = true`, and leaves the root's `force` intact
   (building it: [tools/dali-gear-sim](../../../tools/dali-gear-sim/README.md)).
3. **Board bring-up is duplicated; semantics are not.** The emulator carries its own
   copy of the GPTimer and GPIO bring-up, a knowing exception to the duplication rule:
   making the controller's transmit path generic over a bench tool's board would put the
   firmware's most fragile code at risk. Gear behaviour is shared, because two copies of
   it would drift.
4. **Reserved short addresses are a model invariant.** A fleet takes the set of short
   addresses it must never hold and refuses them at construction and at
   `PROGRAM SHORT ADDRESS`, counting both refusals. The set must cover every real
   luminaire sharing the segment, because two devices answering one address kill the
   bus. The hardware emulator also brings a fleet with no stored history up disabled.
5. **The gear judges order; the master judges time.** A configuration command executes
   only on a second identical instance (101 §9.3, 102 §11.4.1). The model splits a pair
   only on a frame addressed to the same gear — narrower than §9.3, whose receiver
   rejects a pair with any active state in between — and counts what it discards, per
   gear, so a foreign master's traffic to its own fixtures is not reported as our
   violation. The 100 ms half of the clause is measured by the controller's own PHY, not
   by the model, which has no clock.
6. **The host simulator also plays the other master.** `SimDaliTransport` implements the
   observed-frame seam with foreign traffic only (our own frames reach the sniffer window
   where they are transmitted). Button events come from the model, so a masked edge, a
   disabled instance or quiescent mode produce nothing. The trigger is a stdin console in
   the host dev-server example (`press`, `release`, `foreign`, `occupancy`), never a REST
   route: the product surface must not gain a way to invent a frame that was never on a
   wire.

### Rejected alternatives

- **A vendored copy of the gear model under `tools/`** — copies drift.
- **Making the emulator a workspace member** — impossible without relaxing the root's
  forced board knobs.
- **Writing the emulator in C** — reimplements the tested slave semantics and maintains a
  second implementation for ever.
- **RMT instead of the bit-bang PHY** — precision the gear side does not need, a new
  encoder and decoder, and a transmit path without the arming lead.

## Consequences

- The host dev server's simulated bus and any hardware emulator cannot disagree about
  what a DALI gear does; the model has its own host tests.
- The observed half of the product is reachable on the host: the dev server and the BDD
  suite exercise the sniffer translator, the Part 103 event path and the rule triggers
  without hardware.
- The hardware emulator is not ported to the P4
  ([roadmap](../../product-design/roadmap.md)); `just gear-sim-check` only type-checks it
  against the shared crates ([10](../10-build-release-and-tooling.md)). It stays on the
  gptimer driver's interrupt path
  ([ADR-025](ADR-025-phy-interrupt-above-critical-sections.md)), and
  `verify_dali_isr_iram.py` applies to its binary as to the firmware's.
- Both shared crates are listed in `scripts/host_crates.txt`, so the function-length and
  duplication gates cover them.
