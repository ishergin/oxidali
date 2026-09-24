# ADR-017: DALI transactions and forward-frame priority

Status: Accepted, amended by ADR-020
Date: 2026-08-09

## Context

The segment is shared with other masters, so the controller has to follow IEC 62386-101
multi-master timing and the use of priorities in IEC 62386-103 §9.13.1. Two mechanisms
are required:

- **Priority by purpose.** §9.13.1 classifies a frame by what it is *for*. Purpose is not
  recoverable from an opcode: an operator's DAPC and the scheduler's DAPC are the same two
  bytes.
- **Transactions.** 101 §9.2 lets one master send a sequence another may not split: every
  frame after the first goes at priority 1, whose settling window closes before priority
  2's opens, so the owner wins each following slot. A DTR staging sequence, an
  `ENABLE DEVICE TYPE` prelude and its command, a send-twice pair, a memory-bank run and
  the device-type enumeration are all such sequences.

Neither works unless three things hold: a priority is a settling **window** with both
bounds (Table 22 — a frame that leaves late announces a lower priority, whatever was
meant); settling is enforced once, from the last edge of any master, at tick resolution;
and a frame that arrives after Table 20's limit is not credited to us as an answer.

**Scope.** This implements 101 multi-master timing and the §9.13.1 use of priorities for
every frame the controller sends, 16-bit and 24-bit alike (`exchange_frame24` is a full
session citizen). It does not make the controller a conforming Part 103 control device;
that half is limited to what [ADR-016](ADR-016-input-devices-and-rule-engine.md) and
[ADR-018](ADR-018-controller-redundancy.md) need.

## Decision

### D1 — Priority comes from the command kind, never the opcode

`DaliPriority` names the ladder's purposes (`Transaction`, `UserAction`, `Configuration`,
`Automatic`, `PeriodicQuery`) and carries both `min_settle_us` and `max_settle_us`.
`TransactionPriority` holds the four classes allowed to open a transaction; priority 1 is
not representable there, so §9.13.1's "shall not start a transaction at priority 1" is a
type-system fact.

The class is the fourth column of `WIRE_CLASSES`
([ADR-013](ADR-013-wire-priority-and-yield-granularity.md)'s table); how that table is
kept and which purpose each kind announces are in [09](../09-dali-protocol-rules.md).
`Origin` refines the class only for an HCL setpoint (`Automatic`). A Home Assistant
command keeps its class even though its `WirePriority` differs — the two axes answer
different questions. The raw diagnostic path is the one exception: its class is refined
from the decoded opcode, because there the opcode is all the information there is.

Within the window the settling time is drawn at random, as Table 22 footnote c "strongly
recommends"; how every draw stays inside the band on the wire is in 09.

### D2 — A transaction is a closure

`transaction(|c| …)` on `DaliApplicationController` is re-entrant and defaults to a
pass-through, so test doubles need nothing. A closure rather than begin/end, because an
early `?` would otherwise leave priority 1 armed outside a transaction.
`transaction_exempt` marks indivisible units that may exceed §9.2's 400 ms guidance (the
closed list is in [09](../09-dali-protocol-rules.md)): the duration bound is a "should";
the atomicity is the point.

How a collision meets a transaction (the rule is in 09) follows from two clauses:
§9.1.3–9.1.4 has the break annihilate the frame for every receiver, so a destroyed first
frame is still a first frame and may not go at priority 1; §9.2 entitles the owner of a
started transaction to each following slot, so a collision on a remaining frame does not
end it.

### D3 — A yield never lands inside a started transaction

The rule is in 09, and it holds against our own work too: to the gear there is no
difference between our interactive frame and a foreign master's — an interloper eats the
`ENABLE DEVICE TYPE`, clobbers the DTR or voids a send-twice pair. Operator-first
therefore rests on unit size, not on interrupting units.

### D4 — Both halves of §9.2's 400 ms

The per-transaction duration is measured across the frames, first to last on the wire.
Overruns split into `transaction_should_exceedances` (exempt units, expected non-zero)
and `transaction_budget_exceeded` (everything else, must stay zero). The periodic bus
release — a settling longer than priority 5's maximum between successive transactions —
is a raised minimum idle rather than a sleep, it is owed only during continuous traffic
(an idle spell already is the release), and a released frame is booked as priority 5.

### D5 — The wire is counted

`DaliWireCounters` (`dali.wire.*` on the diagnostics surface) count frames by priority,
collisions, foreign frames in our window, violations, retries and exhaustion, transaction
starts, re-runs and leaks. `transaction_leaks` and `transaction_budget_exceeded` are gates
at zero; the rest are observations, and outcomes are counted separately from retries so
their ratio is readable.

### D6 — The PHY reports the settling each frame actually had

`pre_idle_ticks` travels on every transmit completion and every capture: the idle count
when the frame was armed or heard. It is the only honest measure of a frame's realised
priority; `p1_window_late` counts intended priority-1 frames that left past 14,7 ms.
Settling is enforced once, in the interrupt, from the last edge of any master, and the
backward window is judged by the interrupt's idle counter rather than the task's clock,
because a task-side deadline is late by however long the task took to wake (both
mechanisms: [08](../08-dali-phy-and-transport.md)).

### D7 — Collision recovery has one owner

The PHY drives the break and waits out the Table 25 recovery (08); the controller adds no
delay of its own. The reduced-settling restart §9.1.4 asks for is not built — a
[conformance gap](../../reference/iec62386-conformance-gaps.md).

### D8 — The unit is retried, not the frame

`send_command` is one transaction containing the prelude and both halves of a send-twice
pair, and a retry restarts it from the first frame, only after the failures in which
nothing executed (the rule is in 09). Resending one frame of a unit produces sequences
the standard forbids: a third copy of a send-twice command, or an extended command after
its prelude was spent, which the gear decodes as a different standard command. For the
same reason a send-twice pair whose halves the gear cannot have read as a pair is re-run
as a unit (09).

### Rejected alternatives

- **Opcode-derived priority as a fallback** — a second source of truth that fires exactly
  where a class was forgotten.
- **Deriving the class from `WirePriority`** — `Attended` covers a Home Assistant switch,
  an HCL setpoint and an operator's read, which are different purposes.
- **An RAII guard instead of a closure** — shared state for something one worker owns,
  a lifetime not tied to the controller, and no way to report a budget overrun.
- **Chunking every unit to fit 400 ms** — leaves the gear holding half a colour for the
  next master's DAPC, or silently under-reports device types. The memory-bank read is
  chunked because its pointer is re-proved per chunk; it is the unit that genuinely
  divides.
- **Wrapping the whole `INITIALISE` search** — seconds of held bus on a shared segment;
  the unit is one probe (`SEARCHADDR ×3 → COMPARE`).
- **A batched transmit path for every unit** (08's `exchange_transaction`) — the
  interrupt-side gate keeps priority-1 misses rare, and per-frame settling samples are
  what `p1_window_late` is computed from. The batch stays built and tested as the remedy
  if that changes.

## Consequences

- Configuration traffic announces priority 3 and operator switching priority 2, so our
  configuration never outranks a foreign master's operator. Reads announce priority 5 and
  wait longer for a gap on a busy segment, which is what a reader should do.
- The worst operator wait during a read is one transaction, not one frame; memory-bank
  chunk boundaries are yield points.
- A command kind without a wire class fails the build; that is the cost of having no
  default.
- The Table 20/22 constants and the PHY gate are described in
  [08-dali-phy-and-transport.md](../08-dali-phy-and-transport.md); the protocol rules
  built on transactions in [09-dali-protocol-rules.md](../09-dali-protocol-rules.md).
