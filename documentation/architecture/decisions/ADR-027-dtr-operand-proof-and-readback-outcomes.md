# ADR-027: DTR operands are proved, and read-backs have three outcomes

Status: Accepted
Date: 2026-09-23

## Context

Every DALI command that takes its argument from `DTR0`/`DTR1`/`DTR2` — `READ MEMORY
LOCATION`, `SET SCENE`, `SET FADE TIME`, `SET SHORT ADDRESS`, the DT8 temporary-colour
stagings, the `QUERY COLOUR VALUE` selector, Part 103 configuration writes — is preceded
by special frames that carry no acknowledgement by protocol. A lost or foreign DTR write
is invisible: the command executes with whatever operand the register holds, and some
wrong operands are not recoverable (`SET SHORT ADDRESS` moves the gear to an address
nobody chose, possibly one another gear already holds; a stale `0x00` under
`STORE GEAR FEATURES/STATUS` disables colour). Send-twice guards against executing a
**corrupted** frame, never against a **lost** one.

IEC 62386-102 §9.10.4 recommends comparing `QUERY CONTENT DTR0` after a memory-read
sequence and §9.10.5 after a write; those queries are mandatory and side-effect-free.
IEC 62386-101 §9.1.3–9.1.4 make a collision destroy the frame for every receiver, while
§8.2.5 makes a violating frame in the backward window an answer.

## Decision

### Arm, prove, act once

- **Write the register, read it back, and act only on an exact echo.** Arming, proof and
  action are one transaction ([ADR-017](ADR-017-dali-transactions-and-frame-priority.md)),
  so a foreign DTR write cannot land between the proof and its use. A mismatch, silence
  or a violation all mean "not proved": re-arm within a bounded budget, then fail with a
  named reason. Never vote over repeated reads — a majority buys probability at many
  times the wire cost, and a read-back buys proof.
- **Proved operands (control gear):** the memory pointer before a memory-bank read (bank
  in `DTR1`, offset in `DTR0`), with the final `DTR0` position compared after the read
  as §9.10.4 recommends; the DT8 colour stagings (`DTR0`–`DTR2`), colour-limit stagings
  and DT8 configuration writes such as `STORE GEAR FEATURES/STATUS`; the
  `QUERY COLOUR VALUE` selector, re-armed and re-proved before every attempt because the
  gear replaces `DTR0` with the answer's low byte; `SET SHORT ADDRESS`; and the colour
  stagings of a scene row.
- **Proved operands (Part 103):** every DTR-armed write proves `DTR0` with an **addressed**
  `QUERY CONTENT DTR0`. The special write reaches every device, but the proof must name
  one: broadcast, every control device answers one window and the answers collide.
- **An effect read-back instead of a DTR proof** for `SET SCENE` (re-driven until
  `QUERY SCENE LEVEL` converges, within a bounded repair budget) and the `SET FADE TIME`
  family — fade time and rate, extended fade time, power-on and system-failure levels,
  minimum and maximum level — each of which queries the variable it set. For fade time
  that is stronger than proving `DTR0`; for a scene it is convergence rather than proof.
  This is a deliberate divergence from the list above, not a hole.
- **Group and broadcast operands cannot be proved:** the read-back is an addressed query
  by construction. Those writes keep effect-level verification only, and that gap is
  real.

### A verified write's read-back has three outcomes

| Outcome | Wire | Error code | Meaning |
| --- | --- | --- | --- |
| a different value | an answer | `VerifyFailed` | the device refused the write; the only case a retry can help |
| silence | no backward frame | `VerifyUnanswered` | the query or variable is what the device lacks, or the query was missed (102 §3.13 Note 1) — proof that we cannot tell, not that the write failed |
| a violating frame | §8.2.5 answer | `VerifyContended` | several units answered one window, or a foreign frame crossed it — never silence |

One classifier, through which every DTR-armed Part 103 write runs, maps the response to
these outcomes; `value() == None` is never used to decide, because it merges the last
two. The three are distinct, append-only error codes.

- **A contended read costs the field, never the device.** During a Part 103 scan a
  violating answer to one read drops that byte and counts the address as contended; the
  device is still published as present.

### Retries of 24-bit frames

- **`Collision` and `BusBusy` are retried** under the same retry policy as 16-bit frames:
  a collision destroys the frame for every receiver and a busy bus transmitted nothing, so
  the resend is the first delivery.
- **A foreign frame or a violation in a 24-bit frame's window is terminal**
  (`Frame24Fault::Contended`): the device may have executed what it heard, so the unit
  that may repeat is the sequence, and the executor decides. A frame that expected no
  answer (`send_twice`, `IDENTIFY DEVICE`, the `TERMINATE` that closes a commissioning
  session) and was destroyed is reported as a fault, never as success.
- **The arbitration probe is sent once and never retried**
  ([ADR-018](ADR-018-controller-redundancy.md)): its worker owns the cadence, and a
  destroyed probe is no verdict rather than a miss.

## Consequences

- A proof costs one query frame per armed operand, inside the transaction that already
  exists.
- The boundary of a `Step` yield ([ADR-013](ADR-013-wire-priority-and-yield-granularity.md))
  is one `DTR0 → SET → read back` triple confirmed before the next begins, so a yield
  never leaves a written-but-unconfirmed field.
- Operation failures name the outcome, so an operator can tell a refused value from a
  missing query from a duplicate address.
- The detailed DALI protocol rules are in
  [09-dali-protocol-rules.md](../09-dali-protocol-rules.md).
