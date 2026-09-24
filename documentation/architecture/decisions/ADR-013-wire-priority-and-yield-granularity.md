# ADR-013: Wire priority and yield granularity

Status: Accepted, amended by ADR-016 and ADR-017
Date: 2026-08-04

## Context

[ADR-009](ADR-009-background-wire-time-budget.md) makes background work yield to an
operator, but "background" there means the poller. A full attribute read with memory
banks, a `write-attributes` run and a discovery scan each hold the wire for seconds, far
beyond the 2000 ms confirmation budget; the operator's request then answers `504`, and
since a timeout releases the confirmation slot but not the command, the lamp changes
after the operator was told it failed.

Two observations shape the fix:

- **The origin does not say who is blocked; the route's answering discipline does.**
  Whether someone holds the single httpd task on a confirmation, or was handed
  `202 + operation_id`, is fixed per command kind. `Origin` only says who asked.
- **"Who yields to whom" and "where may it stop" are different questions.** A read may
  stop at any frame; a discovery run may not stop at all, because it holds an
  `INITIALISE` session and interrupting it leaves gear withdrawn for up to 15 minutes.

## Decision

Two axes, both read off the **command kind**, in one table
(`dali2rust-dali-runtime/src/runtime/priority.rs`) that is exhaustive over
`DALI_WORKER_HANDLED_COMMANDS` with no default — a command without a row fails a test.
Which kinds sit where is that table's business; the rule it implements is in
[09](../09-dali-protocol-rules.md).

### Axis 1 — `WirePriority`: who yields to whom

| Priority | Who is waiting |
| --- | --- |
| `Interactive` | a human is waiting now: an HTTP caller blocked on a confirmation, or a rule acting on a button press ([ADR-016](ADR-016-input-devices-and-rule-engine.md)) |
| `Attended` | an operator watches an operation; nobody is blocked |
| `Unattended` | nobody asked |

A lease is revoked only by an arrival of **strictly higher** priority; equal priorities
never preempt each other, so two attended reads cannot livelock.

`Origin` refines a kind only where its producers answer differently (the refinements are
listed in 09): an HCL setpoint is `Attended` so that a manual override beats the
scheduler, and a Home Assistant command is `Attended` because nobody holds the httpd task
while a person watches.

**Lamp-driving commands stay FIFO among themselves.** Priority may reorder a batch only
across frames that do not both drive the lamps: of two lamp-driving frames, the one that
runs last is the state the operator is left with, so a promoted newer frame would be
overwritten by the older one. The table's third column (`drives_lamp`) marks those kinds,
and the worker falls back to the first lamp-driving frame in the batch whenever the
priority winner drives the lamps.

### Axis 2 — `YieldGranularity`: where it may stop

| Granularity | For | Why | Worst operator wait |
| --- | --- | --- | --- |
| `Frame` | reads | DTR is scratch, a memory-bank position is proven per chunk, an aborted stage publishes nothing | about one frame (~50 ms); a started transaction finishes first ([ADR-017](ADR-017-dali-transactions-and-frame-priority.md)) |
| `Step` | configuration writes | each field is a proven `DTR0 → SET → read back` triple, and the executor stops at the first failure reporting what it confirmed | one triple (~0.5 s) |
| `Never` | sessions and short units | a session or a short unit that is its own boundary | the whole run |

`Never` work takes no lease, and neither does `Interactive` work whatever its row says.

### Displaced work

What each kind of displaced work keeps is in 09. `Preempted` is its own error code
because preemption is routine — an operator working the UI preempts their own reads — and
a surface that paints every failure red would report the mechanism working as a fault.

Discovery does not yield, deliberately: commissioning is a maintenance operation, and a
`504` on an interactive route during it is legitimate. The surface owes the operator an
explanation, not a shorter wait. A `504` keeps its meaning: the bus is genuinely occupied
or wedged.

### Rejected alternatives

- **Widen `Origin::is_background()`** — the same origin publishes the operator's `PUT`,
  which must never yield.
- **Classify by `Origin` alone** — it says who asked, not who is blocked.
- **Frame-level preemption for everything** — corrupts a write mid-triple and breaks a
  discovery session.
- **Resume a preempted read** — needs partial commits; a re-read is cheap, and the
  operator asked for a snapshot.
- **One command per attribute section** — a second completion rule for the same latency.

## Consequences

- The lease types are `WireActivity` / `WireLease` / `with_wire_lease`; the activity
  signal keeps one arrival counter per yieldable priority, and `step_boundary()` marks the
  one place a `Step` lease may yield.
- `read_attributes_preempted` and `write_attributes_preempted` count yields and stay out
  of the execution-failure and diagnostics fault sets.
- A group or scene apply yields to an operator setpoint between rows.
- [ADR-017](ADR-017-dali-transactions-and-frame-priority.md) adds a fourth column — the
  IEC 62386-103 §9.13.1 purpose a kind's frames announce on the wire — and makes a yield
  land on a transaction boundary.
