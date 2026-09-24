# ADR-026: DT8 colour activation

Status: Accepted
Date: 2026-09-23

## Context

A DT8 colour write (IEC 62386-209) is two steps: the colour is staged into the gear's
temporary registers, and a later command activates it. §9.12.5 offers exactly two
activations:

- **any arc-power command**, but only while the `Automatic Activation` bit (bit 0 of
  `GEAR FEATURES/STATUS`) is set — Table 5: with the bit clear, `DAPC 0–254` means no
  colour change and no change in the temporaries;
- **command 226 `ACTIVATE`**, which "shall stop a running fade and start a new fade that
  only changes colour", after which the temporaries are set to MASK.

Three facts constrain the choice. A group or broadcast address has no readable level and
no readable `Automatic Activation` bit: an addressed query to a group collides every
member's answer. `ACTIVATE` followed by an arc-power command stops the colour fade it
just started, with nothing left in the temporaries to re-activate. And an arc-power
command on a gear already at the commanded level is not an arc-power *change*, so it
activates nothing. Stacking several activations on one write hides which of them works:
a fixture that changes colour does so despite the others, and no test or wire trace can
tell the mechanisms apart.

## Decision

- **Stage, then activate, as one unit.** Staging and its activating command form one
  exempt transaction ([ADR-017](ADR-017-dali-transactions-and-frame-priority.md)): a
  foreign `DAPC` between them would commit half a colour.
- **A colour is staged only when the setpoint states one** (the `states_color()` rule of
  [06](../06-registry-and-persistence.md)): a bare "switch on" stages nothing and sends no
  `ACTIVATE`.
- **What closes the write depends on what the setpoint states:**

  | Setpoint | Frames after staging |
  | --- | --- |
  | a level, or OFF | `DAPC level` (OFF is `DAPC 0`, so it fades through the gear's `fadeTime`); the arc-power command is the activation, and no `ACTIVATE` is sent |
  | colour only (no power, no level) | `ACTIVATE` alone — no arc-power change, so a fixture the operator left dark stays dark |
  | ON without a level | `ACTIVATE`, then `GO TO LAST ACTIVE LEVEL` |
  | nothing | no frames |

  These branches are the same for every address class — short, group and broadcast.
- **`ACTIVATE` never precedes a level-carrying arc-power command.** What is forbidden is
  the order, not the command: `ACTIVATE` blanks the temporaries and a following `DAPC`
  stops the colour fade `ACTIVATE` started.
- **Switch-on without a level sends `ACTIVATE` first.** `GO TO LAST ACTIVE LEVEL` is what
  switches a dark gear on, but on a gear already at that level it activates nothing, and
  a group has no level to re-assert with `DAPC`. `ACTIVATE` promotes the temporaries
  immediately; sent after, it would stop the level fade `GO TO LAST ACTIVE LEVEL` just
  started and freeze a dark member part-way up.
- **No `QUERY STATUS` gate in front of the activation.** §9.12.5 applies the temporaries
  on reception of the activating command, not over the course of a running fade, so
  waiting on `fadeRunning` leaves a colour staged during a fade unapplied. Every
  activation the standard offers stops a running fade (§9.12.5 for `ACTIVATE`, Table 5
  for arc power MASK), so that cost is inherent in activating at all. A `lampOn` gate is
  unnecessary because `ACTIVATE` carries no arc-power change.
- **Not `DAPC 255`.** Arc power MASK is the first route and depends on the
  `Automatic Activation` bit; with the bit clear, Table 5 makes it "stop fading, no colour
  change, no change in temporaries" — a silent failure — and on a group the bit can be
  neither read nor repaired. `ACTIVATE` carries no such condition.
- **The level-carrying route depends on the bit, so the bit is read and restored.** The
  controller reads `GEAR FEATURES/STATUS` (247) with the colour attributes. On a short
  address, when a stored observation says the bit is clear and the controller setting and
  per-device exception permit it, the colour write first restores it — writing `0x01`
  (never echoing the queried byte, whose top bits are read-only) and accepting
  `answer & 0x01`. Table 8 makes the byte RAM with a power-up default of set, so this
  restores the standard's default.
- **One activation per address class, retried rather than stacked** (the general rule is
  in [09](../09-dali-protocol-rules.md)). On a short address, a colour-status read-back
  after staging that shows the wrong colour type or an out-of-range value re-stages once,
  as a whole unit with its prelude; an unanswered status is accepted rather than
  re-driven blind.
- **Verification is short-address only.** The colour-status check follows the staging,
  and the `RGBWAF CONTROL` read-back (251) runs after the activating command, because 251
  answers the promoted byte. A group or broadcast write cannot be read back
  ([ADR-027](ADR-027-dtr-operand-proof-and-readback-outcomes.md)).

## Consequences

- Every path that stages a colour activates it, so the applied fact never claims a colour
  that is still sitting in the temporaries.
- A colour-only command during a level fade freezes the brightness where the fade had
  got to. That is inherent in activating; a client that sends brightness and colour in one
  command takes the level branch and activates in a single fade.
- `ACTIVATE` is one frame with its `ENABLE DEVICE TYPE 8` prelude (209 §11.3.4.2 makes
  only commands 239–246 send-twice), and 102 §9.18 makes non-DT8 members of a mixed group
  ignore it.
- The DALI protocol rules around colour (RGBWAF control, sRGB channels, scene colour) are
  in [09-dali-protocol-rules.md](../09-dali-protocol-rules.md) and
  [ADR-022](ADR-022-rgbwaf-channels-are-srgb.md).
