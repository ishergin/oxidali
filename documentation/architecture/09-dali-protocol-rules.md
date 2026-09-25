# 09 DALI protocol rules

The IEC 62386 rules the DALI executor keeps when it builds frame sequences, reads answers
and shares the bus with other masters. Each rule is stated here once; the decision behind
it, where one was taken, is in the linked ADR.

Scope: what goes on the wire and how answers are read, above the PHY. The interrupt, the
transport and flash → [08](08-dali-phy-and-transport.md); what the product does not
implement → [conformance gaps](../reference/iec62386-conformance-gaps.md). The standards'
PDFs and the DiiA(SW)098bp digest are kept locally, outside the repository.

## Product path and diagnostic path

- Product modules publish semantic commands (typed `CommandEnvelope`s) and never
  assemble opcodes or frames. `DaliCommandPayload` (raw mode on `/api/v1/dali/raw`) is
  published only by the diagnostic `/api/v1/dali/*` routes and is never mapped onto
  physical devices or virtual lamps.
- Types hold the split: product code depends on `DaliProductController`; only the worker
  and diagnostics use `DaliApplicationController`, which adds `send_raw`.
- A 24-bit Part 103 frame is its own payload shape, worker branch and
  `DaliTransport::exchange_frame24`, never an overload of the 16-bit `wire_address` +
  `command` pair.

## Priority and yielding

- Priority comes from the command kind, never the opcode. `WIRE_CLASSES`
  (`dali-runtime/src/runtime/priority.rs`) has one row per command the worker handles,
  no default, and a test fails for a missing row: `WirePriority`, `YieldGranularity`,
  whether the kind drives a lamp, and the 103 §9.13.1 class of its opening frame (policy
  implementing a "should"; each row cites its clause). Only the diagnostic path refines
  the class from the opcode
  ([ADR-013](decisions/ADR-013-wire-priority-and-yield-granularity.md),
  [ADR-017](decisions/ADR-017-dali-transactions-and-frame-priority.md)).
- The class follows purpose: switching, dimming, identification, feedback drive and every
  commissioning kind (discovery, addressing, replacement, handover, the Part 103 scan and
  commissioning) open at priority 2, which §9.13.1 also allows for commissioning;
  configuration writes and group and scene programming at 3; an HCL setpoint at 4;
  attribute and memory-bank reads and both probes at 5 (the arbitration probe by DiiA 351
  §7).
- `WirePriority` says who is waiting: `Interactive` (a caller holds the HTTP task),
  `Attended` (answered `202` + operation id; someone watches), `Unattended` (nobody).
  A lease is revoked only by a strictly higher arrival. `Origin` refines only kinds whose
  producers answer differently: a poller read is `Unattended`, HCL and Home Assistant
  light commands are `Attended`, a rule's are `Interactive`.
- `YieldGranularity` says where a sequence may stop: `Frame` for reads, `Step` for
  configuration writes (`write-attributes`, Part 103 instance and feedback configuration)
  at a confirmed DTR → SET → read-back boundary, `Never` for discovery, commissioning and
  other units whose interruption leaves gear half-programmed.
- Background work never delays an operator, and that is not a setting: a quiet window,
  interactive-first ordering with preemption at frame boundaries, and a wire-time budget
  (after a background read of cost *d* the poller rests 3×*d*, so background holds at
  most a quarter of the wire; `interval_ms` is a floor)
  ([ADR-009](decisions/ADR-009-background-wire-time-budget.md)).
- Preemption is routine and never a device fault. A preempted read ends
  `ErrorCode::Preempted` and commits nothing from the interrupted stage; a preempted
  `write-attributes` keeps the attributes it confirmed before the yield and ends
  `Preempted`; the poller re-queues the target with no cooldown. No surface renders it as
  a failure.
- A priority is a settling window (101 Table 22), not a floor. The controller draws a
  random point inside it (footnote c) in whole PHY ticks, with a small guard above the
  band's floor and the arming lead subtracted, so the draw lands inside the band on the
  wire; the PHY's pre-idle ticks are the only measurement of the window a frame actually
  landed in.

## Transactions

- A unit the gear treats as one (DTR arming and its command, `ENABLE DEVICE TYPE` and its
  extended command, a send-twice pair, the device-type walk, a memory-bank chunk) runs in
  `controller.transaction(|c| …)`; every frame after the first goes at priority 1
  (101 §9.2), so the owner wins each following slot
  ([ADR-017](decisions/ADR-017-dali-transactions-and-frame-priority.md)).
  `transaction_exempt` marks an indivisible unit that may exceed §9.2's 400 ms guidance.
  The exempt kinds are closed: DT8
  colour staging with the command that activates it (including a target-state write and a
  scene programme), a scene-colour read, and 102/103 commissioning and discovery sessions.
  A new exempt call site is one of these kinds, or this list grows.
- A collision or `BusBusy` on the first frame does not start a transaction: the destroyed
  frame is still a first frame, and priority 1 is forbidden for it. A collision on a later
  frame does not un-start one: the retransmission stays at priority 1 and the yield shield
  holds until the bracket closes.
- No yield lands inside a started transaction, not even for our own interactive work.
  Operator-first rests on small units: a read is a series of short transactions, and
  `step_boundary` closes the open one when something is waiting.
- The retry unit is the whole unit, prelude included: only `Collision` and `BusBusy`
  retry it, and a 16-bit query whose window held a whole foreign forward frame (or, with
  the contention-retry knob, went unanswered while foreign traffic was seen) counts as a
  collision. `Preempted` and transport errors propagate.
- The two halves of a send-twice pair are judged by the settling the interrupt measured
  before the second: past Table 17's 75 ms the pair is our breach and is counted; past
  Table 20's 94 ms the gear cannot have read a pair, and the unit is re-run as `BusBusy`.
- A 16-bit frame sent on its own (`send_raw`) is retried frame by frame on the same
  outcomes, at priority 1 inside a started transaction; a frame that expected no answer
  reads anything in its window as no answer.
- A query that advances gear state on every frame the gear hears — `READ MEMORY
  LOCATION` (the DTR0 post-increment), `QUERY NEXT DEVICE TYPE` (the §11.5.13 cursor) — is
  sent once with no frame-level retry (`send_raw_once`); the caller re-arms or restarts the
  sequence, because a blind resend reads the next value under the previous one's name.
- An `ENABLE DEVICE TYPE` prelude is spent by the next command the gear executes
  (102 §11.7.14). A query the gear may have heard is re-sent with its prelude
  (`send_raw_enabled_query`), never bare.
- An extended configuration command goes out as `ENABLE DEVICE TYPE x, CMD, CMD`: one
  prelude, nothing between the halves (101 §9.3; 207 §11.3.4.1; 209 §11.3.4.2).
  `requires_repeat` comes from each part's configuration opcode range, and a
  configuration opcode without a typed variant is refused rather than single-framed.
- One mechanism per effect. Retrying the one path after a detected failure is fine; a
  second parallel route to the same effect is not, because it hides which route works.

## Reading answers

- A query window has three outcomes: an answer, silence (NO), and a violation. A
  frame-size or bit-timing violation in the backward window is a backward frame
  (101 §8.2.5): `DaliResponse::Violation`, whose `value()` is `None` and whose `is_yes()`
  is true. It is terminal, never retried, and not a collision. `COMPARE`,
  `VERIFY SHORT ADDRESS` and `QUERY CONTROL GEAR PRESENT` read it as YES;
  `QUERY SHORT ADDRESS` reads it as "several gear hold this address"
  ([ADR-020](decisions/ADR-020-violating-backward-frame-is-an-answer.md)).
- A violation after a frame that may lawfully draw several answers (first byte bit 7 set:
  broadcast, group, special command) is counted as `backward_multi_answer`, apart from the
  counters that mean "one gear was asked and its answer was unreadable".
- Several gear answering one window can also decode as a clean byte, so a clean answer to
  an addressed query does not prove a single responder (open ISSUE-124 in
  [`known-issues.md`](../product-design/known-issues.md)).
- A capture held dominant longer than any transmitter drives (`is_line_held`) is logged
  as a hold but still read as a §8.2.5 backward frame (open ISSUE-123): two control
  devices at one short address answering half a bit apart produce the same capture, so
  only repetition across probes tells them apart — which is why the Part 103 scan
  re-asks a violating address.
- A read that has passed its presence probe fails `device_absent` after three consecutive
  clean silences on queries Part 102 obliges a present gear to answer. A contended
  exchange counts as heard, and a device-type-scoped or edition-2-only query (`QUERY
  LIGHT SOURCE TYPE`) never counts, because its silence is a legal answer.
- MASK in an answer is not a value. `QUERY ACTUAL LEVEL` = MASK (start-up, or light
  expected and absent, 102 §11.5.20) publishes a setpoint that states nothing — power
  unknown, no level, no colour — while the status byte and last-seen are still published.
- A boolean broadcast query needs a positive control: YES is a backward frame, NO is
  silence (102 §3.28, §3.13), and silence may be a lost query (§3.13 Note 1). The
  bus-health probe broadcasts `QUERY CONTROL GEAR PRESENT`, then `QUERY LAMP FAILURE`; a
  probe whose control went unanswered is `invalid`, never `clear`, and a violation on the
  second frame means several gear answered. The two frames are one reading, and they add
  to the poller's sweep rather than replace it: two boolean frames cannot read levels,
  colour or attributes (schedule and budget:
  [poller](../product-design/runtime-modules/poller/README.md)).
- Arm, prove, act once: an operand staged in a DTR is proved by read-back before the
  command that consumes it, and a read-back tells a wrong answer, silence and a contended
  window apart ([ADR-027](decisions/ADR-027-dtr-operand-proof-and-readback-outcomes.md)).
  The 16-bit configuration write still takes an unanswered read-back as confirmation
  (open ISSUE-118).
- A multi-byte memory-bank value is read inside one latch (DiiA 252/253): chunks are
  field-aligned, a re-arm restarts the value, and a value no latch covers fails
  (`memory_bank_latch_lost`) rather than being stitched. MASK and TMASK are per width,
  signed widths included; value, MASK, TMASK and "not read" are four states.
- A memory-bank read chunk spans at most five locations (a latched value wider than that
  stays whole) and is one transaction: a longer one runs past 101 §9.2's 400 ms guidance
  on real gear.

## Faults and identification

- A fault is read in two steps. The runtime section (`QUERY STATUS`,
  `QUERY ACTUAL LEVEL`) raises a suspicion when `lampFailure`, `controlGearFailure` or an
  actual level of MASK shows one — 207 ties its failure bits to `lampFailure`, and its
  thermal states show only as MASK. Only then does the read spend one
  `ENABLE DEVICE TYPE 6` + `QUERY FAILURE STATUS` pair (not when the declared type set
  excludes DT6), and the section's booleans are decoded from that byte.
- `powerCycleSeen` (102 §9.16.9) is cleared by `RESET` and every level command, so it is
  evidence only from a status read that precedes any driving command: the poller's reads
  can report it, the target-state path cannot.
- Identification is the gear's own procedure: one `IDENTIFY DEVICE` send-twice pair
  (102 §9.14.3.2) starts a 10 s ± 1 s window the gear owns and restores from itself
  (§9.14.3.1). Nothing else is sent — no `RECALL MIN`/`MAX LEVEL`, no restoring level
  command — so no variable moves. Another pair extends the window and nothing shortens
  it, so the route takes no duration, and there is no fallback: the command's failure is
  undetectable. The indication may be a driver LED or a sound (§11.4.7), so acceptance
  is the wire and the variables.

## Device types and memory banks

- A device type is a set. `DeviceTypeSet` (bit N = type N) travels on discovery progress
  and the `Common102` read and is stored on the record. `None` (the §11.5.13 walk never
  finished) and empty (the gear's own 254) are different facts, and the merge is sticky,
  so a degraded walk never erases a clean one. The set means "known to support": a DT8
  probe answered behind its own `ENABLE DEVICE TYPE 8` joins it.
  `device_type_discovered` is the product's reduction of the set to colour controls.
- 102 §9.18 has a gear ignore extended commands of a type it does not support. So an
  override may narrow the declared set, never widen it — a widened one would offer
  controls whose frames the gear ignores (`422`, checked only once the set is known) —
  and an extended configuration write (DT8 Tc limits, the DT6 dimming curve) needs no
  device-type gate: a gear without the type ignores it.
- The device-type walk is one non-yieldable unit; a break restarts it once, then it fails
  (`bus_contended`, `device_type_enumeration_incomplete`) rather than commit a subset.
- Bank 0 `0x1C` is one byte whose bit x is Part 15x (DiiA(SW)098bp Table 4); a byte with
  a bit outside that range claims no part.

## Addressing control gear

- Unaddressed commissioning takes the lowest short address that the registry does not
  know and that nothing answers on the wire (a violation counts as an answer): the
  registry protects a powered-down gear, the wire catches an address the registry forgot.
- A commissioning attempt that failed after `WITHDRAW` is not re-run: the gear has left
  the search, and a new search would program the next gear onto the same short address.
- `VERIFY SHORT ADDRESS` answers only inside an `INITIALISE` session, so a re-address
  outside one is verified with `QUERY STATUS` at the new address; ingress has already
  refused an occupied target, so only the moved gear can answer there.

## DT8 colour (Part 209)

- How a staged colour is activated, per address class, is decided in
  [ADR-026](decisions/ADR-026-dt8-colour-activation.md).
- Automatic Activation (bit 0 of `GEAR FEATURES/STATUS`) decides whether an arc-power
  command applies a staged colour, and `GO TO SCENE` depends on it too. It is read with
  the colour attributes and restored as ADR-026 describes, when the controller setting
  and the device's exception allow it
  ([settings-dali](../product-design/rest-api/resources/settings-dali.md)).
- RGBWAF control (§9.1): only normalised colour control takes colour from the dim levels
  and brightness from the level. The RGB write reads 251 and writes `0x80` (normalised,
  driven channels unlinked) only on a mismatch, under the `dt8_rgbwaf_control_assert`
  permission, since 098bp's extended control (`0xC0`) is also legal. Boot state is never
  assumed; `0x80` is a fixed point of the unlink rule. The read-back after activation
  fails the write only when a driven channel is still linked: a control type with the
  driven channels unlinked (`0xC0` too) passes, and a gear that does not answer 251 is
  neither written nor failed.
- Every live RGB write and every RGB scene row also stages W, A and F (zero for a
  three-channel colour), whatever the control-byte permission: under normalised control
  each channel scales against the largest of R..F, so a stale channel both lights up and
  dims the others (209 §9.1).
- Scene programming reuses the live staging helpers, which therefore never activate, and
  never stages `RGBWAF CONTROL` (237): `STORE DTR AS SCENE` consumes a temporary without
  promoting it (209 §9.12.5).
- A scene's stored colour is read inside one exempt transaction, from the
  `QUERY SCENE LEVEL` that loads the shared REPORT registers to the last REPORT value:
  any of five commands from any master — `QUERY ACTUAL LEVEL` among them — reloads those
  registers (209 §9.12.6).
- `QUERY COLOUR VALUE` splits MSB and DTR0 only for 16-bit values (209 §9.9); one-byte
  values come whole. `colour_value_is_wide()` is the one definition, shared with the gear
  model. When a gear does not answer, DTR0 still holds the selector, so reading it back
  yields the selector, not a value.
- RGBWAF channels are sRGB above the wire and linear on it: encode at the one write
  point, decode at every read point (attributes, scene reports, the sniffer translator),
  floor a non-zero channel at dim level 1
  ([ADR-022](decisions/ADR-022-rgbwaf-channels-are-srgb.md)).
- A convergence check compares in wire space, through the encoder — the sRGB table, or
  `kelvin_to_mirek` for Tc — never through a decoder (ADR-022).

## DT6 (Part 207)

- The dimming curve is written with `SELECT DIMMING CURVE`, DTR0-armed and send-twice
  behind `ENABLE DEVICE TYPE 6`, with no device-type gate (above). What a gear without
  DT6 leaves on the attribute, and the values the write accepts, are in
  [physical devices](../product-design/rest-api/resources/physical-devices.md).
- Opcodes `0xE3`/`0xEE` are also DT8 commands and Part 218 (DT17, unsupported) ones; a
  decoder or model routes by the live `ENABLE DEVICE TYPE` prelude before it classifies
  the opcode.

## Part 103 control devices

- Commands execute inside `DaliWorker`; events are decoded by the sniffer translator
  ([ADR-016](decisions/ADR-016-input-devices-and-rule-engine.md)). Control devices and
  control gear have independent address spaces.
- Encodings differ where they look alike: `PROGRAM`/`VERIFY`/`QUERY SHORT ADDRESS` take a
  raw `00AAAAAA` operand (103 §11.10.10) against 102's `0AAAAAA1`, `INITIALISE` is
  inverted (`0x7F` unaddressed, `0xFF` all), and control devices have 32 device groups,
  not 16. `ShortAddressOperand` and `InitialiseScope103` make the mistake a type error.
- An event message is a bitwise OR, not one of a list: an occupancy event carries
  movement, occupancy, the still flag and the sensor kind at once (§9.4.3). 302 and 304
  carry an opaque 10-bit magnitude: the raw field travels and nothing invents a scale
  (the scaling is §9.4 of the conformance gaps).
- Scheme 0 is legal and carries no device identity; its events are published and counted
  (`input_events_ambiguous_scheme`), and the decoder takes all five schemes. The product
  expects scheme 2 and shows any other as unconfirmed. A device falls back to scheme 0
  silently when the scheme's precondition (its short address, its group) disappears
  (103 §9.6.3), so an instance write sets the scheme last, after `instanceActive`, and
  reads it back. Commissioning addresses devices and writes no scheme.
- DTR proofs on a control device are addressed, never broadcast, and which 24-bit
  outcomes are retried is decided in ADR-027.

## Sharing the bus with a foreign master

- When our path and a known-good foreign master drive the same gear differently, diff
  the two frame sequences before guessing. The Wiren Board master runs `wb-mqtt-dali` on
  `python3-dali`, whose source is on that board (`/usr/lib/python3/dist-packages/dali/`).
- Judging our signal's reliability against the foreign master's is a bench measurement;
  its oracles and their limits are in [`tools/hil/STRATEGY.md`](../../tools/hil/STRATEGY.md) §3.
