# ADR-009: Background DALI work — ordering, preemption, wire-time budget

Status: Accepted, amended by ADR-013
Date: 2026-07-30

## Context

The interval poller occupies the DALI bus continuously and unattended. One background
read can hold the wire for seconds (a DT8 colour sweep is dozens of frames), which is
longer than an operator's `confirmation_timeout_ms` of 2000 ms, and no retry or attempt
count bounds duration. Two facts shape the answer:

- **No admission test can close the window.** Any check — a quiet timer, an exact
  "worker idle" signal — is evaluated before the read starts, and the operator's command
  can arrive right after admission was correctly granted. Declining to start narrows the
  window; only yielding closes it.
- **Priority cannot live on the bus.** Commands route by kind alone
  ([ADR-006](ADR-006-routed-command-delivery.md)), the same command kind has background
  and interactive producers, and a second, shallower inbox would need a new variant and
  still deliver admission, not preemption.

## Decision

The rule is **background work never delays an operator**, and it holds whatever the
settings say. Four invariants implement it:

- **I1 — ordering.** The DALI worker re-drains its inbox before every command and serves
  higher-priority commands first. It is a stable partition, so the order within each
  priority holds.
- **I2 — preemption at frame boundaries.** Yieldable work runs under a `WireLease`
  (`with_wire_lease`). The controller checks the lease before every attempt of every
  exchange and answers `Preempted`, so the retry schedule cannot stretch a yield. The
  exposure is one settle plus one exchange (`BACKGROUND_YIELD_BUDGET_MS`, 70 ms).
  Non-yieldable work takes no lease.
- **I3 — wire-time budget.** After background work costing `d` on the wire, the poller
  stays off the wire for `3 × d`, so background work holds at most a quarter of the wire.
  `interval_ms` is therefore a floor, not a period. Everything the poller transmits is
  charged — reads and the broadcast bus-health probe alike (`charge_wire_time` takes a
  duration and does not care who measured it) — and both are published behind the same
  gates. An expired probe charges nothing: an elapsed guard window is not a measurement.
- **I4 — one read in flight.** On a serial bus a deeper window buys no throughput and is
  exactly the queue depth ahead of a waiting operator, so the poller has no concurrency
  setting. The probe takes its own pending slot, not the read slot.

A lease is revoked by an **arrival edge**, not a pending balance: the lease captures a
ticket and yields when a strictly higher-priority arrival moves it, the arrival being
stamped on the publishing thread by a `CommandArrivalObserver`
([03](../03-bus-and-backpressure.md)). A balance would need a matching decrement that a
dropped, rejected, filtered or coalesced command never provides. An edge costs at most
one spurious yield, and that costs no wire time because the check precedes the frame.
The observer sees only contract metadata, so the bus learns nothing about DALI.

A 400 ms quiet window (`INTERACTIVE_QUIET`) before a background read starts is kept as
hysteresis only, so the poller does not step into the middle of an interaction.

A **preempted read is not a device fault** (what it keeps: [09](../09-dali-protocol-rules.md)):
the target is simply re-read on a later cycle.

### Rejected alternatives

- **Admission only** (longer quiet window, exact idle signal) — cannot close the window.
- **Credit inversion** (the worker pulls a read when idle) — still admission, and it moves
  the poller's state into the DALI runtime.
- **A second, lower-priority inbox** — a new variant, a wider route key, and admission
  instead of preemption.
- **Section-level preemption** — one colour section is most of a read, so it bounds
  almost nothing.
- **Smaller retry or content-confirm budgets for background reads** — lowers data
  integrity and raises cooldowns without bounding duration; reducing what is read is the
  budget's job, not a second mechanism's.

## Consequences

- `reads_preempted` rises whenever an operator is working; that is the mechanism
  succeeding, and it stays out of `reads_failed` and the diagnostics fault set.
- `duty_deferred` counts once per due read the budget held back, and
  `interactive_deferred` once per busy-wire episode, so a poller held off by the budget or
  by operator traffic is visible rather than looking stalled.
- Sustained interactive traffic starves the poller entirely; that is the intended order.
- A cycle does not drain every device when the budget holds it; target selection resumes
  after the device polled last, so the tail of an installation is not starved.
- [ADR-013](ADR-013-wire-priority-and-yield-granularity.md) generalises I1 and I2 from
  "the poller" to every command kind by answering discipline, and adds where each kind may
  stop. The poller's settings and counters are described in
  [runtime-modules/poller](../../product-design/runtime-modules/poller/README.md).
