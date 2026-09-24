# ADR-003: The timer interrupt owns the DALI PHY

Status: Accepted, amended by ADR-010 and ADR-025
Date: 2026-06-04

## Context

DALI is bit-banged: the controller drives and samples the line itself at a 104 µs
tick (a quarter of a 416 µs half-bit). IEC 62386-101 allows only a narrow tolerance on
each half-bit, and a task-level polling loop on FreeRTOS cannot hold it: any
preemption, lock or scheduling delay lands directly on an edge. The controller is
dual-core and runs FreeRTOS in SMP mode, so a task and the interrupt can run at the
same instant on different cores, and single-core interleaving is never a form of
mutual exclusion.

## Decision

- **The GPTimer interrupt owns the PHY state machine.** `DaliBitbangPhy` is mutated
  only inside the interrupt. It generates every edge, samples the line, detects
  collisions, and captures every frame on the wire, ours and foreign.
- **Tasks talk to it through ISR-safe primitives only** — atomic cells, lock-free
  single-producer/single-consumer rings and atomic counters (the inventory is in
  [08](../08-dali-phy-and-transport.md)). Task code never calls the FSM's transmit, poll
  or sample functions and never reads its state directly.
- **The interrupt never allocates, blocks, logs, notifies a task, or touches the bus
  or HTTP.** Consumers poll the rings; a FreeRTOS notify would take a kernel lock.
- **Decisions stay in task context; timing belongs to the interrupt.** Decoding a
  capture, choosing a reply and building a frame happen in tasks. The interrupt
  transmits what a task handed it, on the tick the timing requires.

## Consequences

- The timing-critical path is small, auditable and bounded; everything above it can
  be tested on the host against the same types.
- What the compiler may emit into the interrupt is settled by
  [ADR-010](ADR-010-isr-crate-and-profile-exception.md): everything the interrupt
  executes lives in the `no_std` crate `dali2rust-dali-phy`, checked on the linked
  binary.
- How the interrupt is allocated and at which level it runs is settled by
  [ADR-025](ADR-025-phy-interrupt-above-critical-sections.md).
- The transmit-start path, the arming lead, collision handling and the answer slot are
  described in [08-dali-phy-and-transport.md](../08-dali-phy-and-transport.md).
- RMT-based capture and task-level critical sections are not part of the design.
