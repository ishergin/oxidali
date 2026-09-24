# ADR-019: A DALI endpoint for a host-based standby

Status: Deferred
Date: 2026-08-10

## Context

A standby controller could run on a Linux host — for example a rooted Android wall panel
with its own buttons and screen — instead of a second P4. The whole stack already builds
and runs on a host, and composition is generic over `DaliTransport`, so what such a
standby lacks is something that physically speaks DALI.

Two facts constrain the answer. Userspace bit-banging from Linux is excluded: a DALI
half-bit tolerates tens of microseconds, while Linux GPIO latency is hundreds. And a
commercial DALI bridge keeps the MAC layer in its own firmware, which costs exactly what
the product depends on: its own retry and contention policy, the priority ladder (a
priority is a settling window, which a bridge does not let us choose), transactions, and
the raw capture the sniffer translator needs.

[ADR-018](ADR-018-controller-redundancy.md) took a second ESP32-P4-ETH as the standby,
which proves arbitration with the same image, transport and wire. **Nothing in this ADR
is built.** It records the design to resume from if a host-based standby is wanted — for
a power domain independent of the DALI supply, or a panel whose buttons must keep working
while its host reboots.

## Decision

The design, deferred:

- **The endpoint is a coprocessor we own, not a bridge.** The question is where the MAC
  layer lives, and it stays in our firmware.
- **Deadlines on the coprocessor, decisions on the host.** The coprocessor generates and
  captures edges, does Manchester and bit timing, enforces the settling window (both
  bounds) and detects collisions, and captures every frame on the wire. The host runs
  the application controller, chooses priorities, owns retries and verify-repair, and
  translates observed traffic. The panel's Part 103 input-device unit (button state
  machine, event encoding, `instanceActive`) lives on the coprocessor, so a press still
  reaches the bus while the host reboots — the same split 101 §4.6.6 draws.
- **The link carries frames and intent, and reports what happened.** `postcard` frames
  over raw USB bulk on a vendor interface, not CDC-ACM (Android kernels often lack the
  class driver). Outbound: bytes, whether a backward frame is expected, and the requested
  priority. Inbound: bytes, frame kind and the observed `pre_idle_ticks`, without which
  the priority ladder is unverifiable. No semantic commands cross the link.
- **RP2350 with PIO** is the preferred MCU: PIO generates and samples the waveform with no
  CPU in the edge path, which removes the transmit-timing failure class instead of
  guarding against it, and a Pico 2 seats on the Pico-DALI2 interface already in use. No
  public DALI-over-PIO implementation exists, so the PIO program is new work.
  `dali2rust-dali-codec` is shared; `dali2rust-dali-phy` is not ported, because PIO
  replaces the bit-bang FSM. An nRF52840 becomes the better choice for a bus-powered
  variant, whose current budget is tighter.
- **The endpoint holds no product state**, except the input-device unit's own Part 103 NVM
  variables, which must survive the host being absent.
- **It declares itself as a bus unit** by its topology (351 §8.2, memory bank 201): a
  USB-attached unit is an externally powered type B drawing at most 2 mA from the bus.
- **On Android the controller is a native root daemon**, started by `init`, never an app;
  it holds a kernel wake lock, runs under an SELinux policy rather than a permissive
  system, and keeps its slice store on a path that survives an OTA.

## Consequences

- Resuming this means a new firmware workspace outside the root (the
  [ADR-014](ADR-014-gear-model-and-second-dali-endpoint.md) precedent) and a new host-side
  `DaliTransport` speaking the link protocol, beside the ESP transport.
- Open questions to answer before any build: whether collision detection lives in PIO or
  on the CPU; one coprocessor or two (so a controller update cannot silence the buttons);
  whether the chosen panel's USB port works in host mode, enumerates without a class
  driver and suspends; how a rooted panel keeps A/B updates; and which power domain the
  panel is on.
