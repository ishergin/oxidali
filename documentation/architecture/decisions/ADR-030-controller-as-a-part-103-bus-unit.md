# ADR-030: The controller as a Part 103 bus unit

Status: Accepted; amends ADR-003, ADR-017, ADR-018 and ADR-025
Date: 2026-10-28

## Context

IEC 62386-101 §4.6.5 makes a multi-master application controller conform to Part 103,
and 103 §9.2.3 says it shall accept the commands of Tables 21 and 22 from other
application controllers. The certification run for such a device begins with a broadcast
`QUERY DEVICE CAPABILITIES` (103 test 12.2.1) and ends there when nothing answers. What
the controller answers today is one query: `QUERY APPLICATION CONTROLLER ENABLED`, the
arbitration probe of DiiA 351 §7, from a table of four stateless slots behind the
redundancy lease ([ADR-018](ADR-018-controller-redundancy.md)). It also obeys another
controller's `ENABLE` / `DISABLE APPLICATION CONTROLLER`. The gap is
[conformance §8](../../reference/iec62386-conformance-gaps.md).

The table cannot grow into the rest. `COMPARE`, `VERIFY SHORT ADDRESS`,
`QUERY SHORT ADDRESS`, the `DTR` queries and `READ MEMORY LOCATION` answer from state
earlier frames built — the initialisation state and its 15-minute timer, `searchAddress`,
`randomAddress`, `DTR0`–`DTR2` — and a memory read increments `DTR0` as a side effect.
Every send-twice instruction needs the pairing rule of 101 §9.3. And the lease that gates
the arbitration answer would be wrong for everything else: 103 §9.9.1 keeps a passive
controller answering queries.

A segment may carry another master that commissions control devices. Such a master opens
with a broadcast `START QUIESCENT MODE`, randomises every unit whose `randomAddress` is
the factory `0xFFFFFF`, keeps unique short addresses, gives a new one to a unit without an
address or with a duplicate, and offers `ENABLE` / `DISABLE APPLICATION CONTROLLER` and
the power-cycle notification as per-device switches. Once the controller answers, all of
this reaches it.

## Decision

### One stateful model decides every answer

- A pure `Device103Unit` in `dali2rust-domain` owns the device's RAM variables (103
  Table 17: `DTR0`–`DTR2`, `searchAddress`, the initialisation state and timer, quiescent
  mode, `writeEnableState`, `powerCycleSeen`, `resetState`, `applicationControllerError`)
  and a copy of its NVM ones (`shortAddress`, `randomAddress`, `deviceGroups`,
  `operatingMode`, `powerCycleNotification`, `applicationActive`). It takes every frame
  the sniffer captured, in wire order — 16-bit, 24-bit and backward, so a frame between two halves
  splits a send-twice pair — and returns its effects and the answer table for the next
  frame. Our own frames never feed it, but one sent between two captures splits a pair
  all the same: the interrupt counts every forward frame on the line, ours included but
  not one that yielded before its first edge, and a capture whose count shows one of ours
  passed reaches the model after a frame it cannot read.
- The model answers nothing and executes nothing until it is hydrated from the registry,
  and a frame that reaches it before then leaves the RAM variables it may have moved
  unknown. The wire is now a mutating ingress, so the boot order keeps hydration before it
  as it does before HTTP; an early `QUERY SHORT ADDRESS` answered with MASK would invite
  another master to assign a new address. The model takes a newer copy only at a poll that
  finds no capture waiting, so each frame is judged under the copy whose table was live
  when it went by, and evidence names that copy.
- **The task decides before the frame, the interrupt answers it.** After every capture
  the `dali-sniff` drain feeds the model and publishes the table the model returns; the
  interrupt answers the next query from that table on its own. The arbitration table of
  ADR-018 becomes part of it. A capture is still routed by its length.
- **A lost capture invalidates the state it would have changed.** A capture the drain sees
  missing (a jump in `sniff_seq`) clears the `DTR`s, the search address and write enable,
  ends initialisation and splits a send-twice pair, and answers that depend on them stay
  silent until they are set again.

### An answer starts in its slot, whatever else is on the line

101 Table 17 footnote a starts a backward frame inside 5,5–10,5 ms of its forward frame
regardless of whether another transmitter has already started one, and 101 §9.1.1 exempts
backward transmissions from collision avoidance, detection and recovery. Holding while the
line is busy hides a duplicate short address from every other controller: the second unit
falls silent exactly when its answer would have corrupted the first. So the interrupt
changes in four places, each a requirement:

- The slot is counted from the last edge of the forward frame the answer belongs to, in
  every bus state, not by the idle counter that stops while a reception is in progress.
- Only a later forward frame, foreign or ours, voids a staged answer; a backward frame or
  a violation captured meanwhile does not.
- An answer never yields in its arming lead.
- An answer may start while another unit's backward frame is on the line. The reception
  in progress is abandoned, the two frames superimpose on the wire, and the dominant level
  wins; the tail left after ours is an ordinary backward-length capture. An answer started
  over another one is counted.

These changes live in `dali2rust-dali-phy` under ADR-010's rules, and
`just p4-isr-iram-check` and `just gear-sim-isr-iram-check` prove the linked images, which
`just ci` does not build.
The gear emulator stages its answers in the same cell
([ADR-031](ADR-031-gear-emulator-on-the-standby.md)), so an emulated gear and a real one
that answer the same query now superimpose like two real gears. Several logical units
behind one interface still merge their answers before staging (101 §9.5.2).

### The interrupt answers from a table the task built in advance

101 §9.8.5 gives a bus unit the settling time before the next possible frame to execute a
command, and every answer of the model depends only on the state the earlier frames left
and on the three bytes of the query. So after each frame the model can say what it would
answer to any next one, and the slot no longer waits for a task.

- **The answer table** is a pure function of the model's state: the address filter (short
  address, device groups, broadcast, unaddressed), the answer to each device query, and the
  answers of the special commands that have one (`COMPARE`, `VERIFY SHORT ADDRESS`,
  `QUERY SHORT ADDRESS`). It names the last capture it accounts for. The drain writes the
  copy the interrupt is not reading and then publishes it, so the interrupt never reads
  half of one.
- **The interrupt answers a 24-bit forward frame alone.** It decodes the capture, checks
  that the table was published after the drain took the capture just before this one,
  matches the address, looks the answer up and stages it for the slot. A table that is
  behind answers nothing and is counted; the drain never answers late in its place.
- **The check names one capture, not all of them.** A capture lost to a full ring moves
  `sniff_seq` without reaching the drain, so the drain sees the loss only as a jump in
  `sniff_seq`, and a table that carries state decides on that jump before it publishes.
  `sniff_seq` is 32 bits wide, so no stall the drain survives wraps it. The table also
  carries the tick it was published at, the drain republishes it at every poll, and the
  interrupt treats a table older than a few polls as behind: a stalled drain, whose model's
  timers stand still too, answers nothing, whether or not the lease gates the answer.
- **The lease is read at answer time.** `applicationControllerError` changes without any
  frame, so the table carries the lease's expiry in interrupt ticks and the interrupt
  applies it to the status byte and the arbitration answer.
- **The 24-bit decoder moves into the interrupt** and is the only decoder of that length:
  the drain takes the decoded frame with the capture, and captures of the other lengths
  stay decoded in the task. The port follows ADR-010's source rules and spreads its phase
  search over the idle ticks before the slot, so no tick carries it whole. It decodes
  every 24-bit frame on the segment, the input devices' events included, so it replaces
  the codec's 24-bit path only once a parity test over recorded bench captures shows the
  two decoders agree.
- **The drain's deadline is the next frame, not the slot.** A query is complete at the
  earliest 2,4 ms of settling (101 Table 20) and 20,8 ms of frame after the frame before
  it, so the drain has about 23 ms per frame where it had `ANSWER_ARM_TARGET_IDLE_TICKS`
  less `RX_IDLE_LINE_HIGH_TICKS` per query. It runs at the 10 ms pace in both roles; the
  1 ms pace is retired.
- **`dali-sniff` runs above the worker pool.** It parks between polls, so it cannot own a
  core, and at the workers' priority its worst measured staging delay was three quarters
  of the slot budget.
- The acceptance is a read-only bench run on which the stale-table counter and
  `isr_ticks_lost_total` stay 0 and the `dali-sniff` census stays within
  `tools/hil/stack_budget.txt`, with the model's frames measured on the linked image
  (`scripts/measure_stack_frames.py`) before the flash.

### NVM changes go through the registry

- The model answers from its own copy at once (101 §9.8.5 executes a command before the
  next frame) and publishes each NVM change as evidence for the registry, so the registry
  stays the single writer; the model hydrates from the DALI settings slice.
- The registry is the only publisher of the unit's NVM copy to the drain: it publishes the
  copy, its revision and the last wire change it acknowledged, at hydration, before an
  operator's change is confirmed and after each piece of evidence. The model adopts a newer
  revision only once every wire change it reported is acknowledged, so the copy it answers
  from never drops a change the registry has not seen.
- Evidence names the revision the model's copy came from, and the registry moves only the
  fields that differ from that copy, so an operator's change to another field meanwhile is
  kept. A field the operator set holds against a wire change the model made before it
  adopted that value; a wire change made after it replaces it.
  Evidence not yet acknowledged is sent again every second.
  `SAVE PERSISTENT VARIABLES` adds nothing: every change is already on its way to flash.
- The bus-unit variables live in the DALI settings slice, which is not transferable: a
  standby never clones the active unit's short address, random address or groups.
- `RANDOMISE` draws from the hardware random number generator. Both units run one image,
  so any seeded generator would give them one `randomAddress`, and a master that finds
  two units on one random address resets both short addresses.

### Answer only what the controller can vouch for

- Capabilities `0x01` (application controller, no instances), a status byte computed from
  real state, version bytes only for what is built, memory bank 0 without a GTIN it does
  not own (103 has no "not available" GTIN, so the identification number from the eFuse
  MAC is the unique part). No bank 201: 103:2014 §9.10.9 forbids banks 200–255 and test
  12.6.1 fails bank 0's last-bank byte above 199.
- `IDENTIFY DEVICE` shows an identification screen on the OLED for the procedure's timer
  (103 §9.14.3); nothing else is the identification.

### Quiescent mode

- Quiescent mode gates every forward frame we send — operator commands, HCL, rules, the
  poller, the arbitration probe and the power notification — while queries are still
  answered (103 §9.9.3). It ends with `STOP QUIESCENT MODE` or 15 min ± 1,5 min after the
  last `START`, and a power-on clears it.
- The gate is read before every frame, not only when a command starts: a command already
  on the wire when quiescence begins, or when the controller is made passive, stops at
  its next frame. A write or a programming ends with the gate's refusal; an attribute
  read or a bus-health probe ends as a yield (`preempted`). A started transaction or
  step finishes, because nothing yields inside one.
- Our own Part 103 commissioning is not stopped by the controller being made passive
  once it has started, so it ends the quiescence it began. Another controller's
  `START QUIESCENT MODE` stops it at its next unit, and it then sends neither
  `TERMINATE` nor `STOP QUIESCENT MODE`: that controller holds the bus quiet now.
- A hold inside a 102 discovery verify session or a commissioning search leaves the gear
  it initialised in initialisation, and the gear withdrawn so far withdrawn, until their
  initialisation timer lapses or another controller ends it, since no `TERMINATE` may be
  sent while the hold lasts.
- The model reports each `START` with its deadline and each end, and the registry holds
  the deadline in RAM; the gate, the poller and the HCL scheduler read it, and it lapses
  at that deadline even if the report of a `STOP` is lost.
- A write that needs the wire is refused with `409 controller_quiescent` and a
  `Retry-After` from the remaining time; quiescence is shown on `/api/v1/health` and the
  redundancy resource, because a master's commissioning run silences the lights'
  controller for its whole length.

### Functional liveness is an application-controller error

- The supervisor of ADR-018 renews its lease while the registry, the HCL scheduler and the
  rules engine keep turning, in both roles and on a single controller as on a pair; the
  lease no longer encodes passivity, which the model reads from `applicationActive`.
- `applicationControllerError` is TRUE while that lease has lapsed (103 §9.15).
  `QUERY DEVICE STATUS` reports it in bit 4, and `QUERY APPLICATION CONTROLLER ERROR`
  answers MASK while it is TRUE and NO otherwise. The bit is computed from the lease's
  expiry at answer time, so a stalled supervisor raises it by not renewing.
- `QUERY APPLICATION CONTROLLER ENABLED` is answered YES while `applicationActive` is TRUE
  and `applicationControllerError` is FALSE; otherwise it is not answered. This is the one
  accepted deviation of the answer path: 103 §11.6.16 ties the answer to
  `applicationActive` alone. It shows only while the controller is already failing, and it
  keeps the standby's probe one broadcast frame per period that no address change can
  break, and a failed controller fail-silent to every DiiA 351 type B unit, a third-party
  one included.

### `SEND TESTFRAME` while passive

103 §9.9.1 forbids a passive controller every forward frame but the power notification,
yet test 12.3.16 sends `SEND TESTFRAME` after `DISABLE APPLICATION CONTROLLER`
(§12.14.1, §12.14.3) and expects the frame. The test is followed for the 24-bit frame the
tests use: it is executed in both roles and refused in quiescent mode. The 16-bit form
is a command to gear and a passive controller refuses it. The DALI worker's one gate
admits the 24-bit form by command kind, beside the arbitration probe and the power
notification.

The model accepts the instruction and the sniffer translator carries it to the DALI worker
as a command; a sniffed frame never becomes one. The model drops the instruction while it
has seen another controller start quiescent mode, so the request never waits on the
registry learning it. The frame is announced at the priority
its `PPP` bits name, which replaces the command kind's class for that unit, so priority 1
opens a frame only here: no command kind's class opens at it (101 Table 22).

### `RESET` keeps the initialisation

103 Table 17 resets `initialisationState` to DISABLED, yet test 12.7.1 runs `RANDOMISE`
after `RESET` and expects it executed, and 102 marks the same variable "no change". The
test is followed: `RESET` leaves `initialisationState` and its timer as they were.

### Every boot is a power cycle

- 101 Table 3 defines an external power cycle by a supply loss of 5 s or more; this
  controller treats every boot as one, an OTA reboot and a watchdog reset included.
  `powerCycleSeen` is TRUE after every boot.
- `POWER NOTIFICATION` is sent once after a boot whose power-on procedure, which ends at
  hydration, finds `powerCycleNotification` ENABLED, and only if it is still ENABLED when
  the delay ends: priority 2, the unit's short address and lowest group, delayed into
  1,3–5 s after that procedure (103 §9.12.2 and the window of test 12.4.16). Quiescent mode
  started inside the delay suppresses it, even when it stops again before the delay ends. `powerCycleNotification` keeps
  the factory value DISABLED of Table 17, so nothing is sent until another controller or
  the operator enables it.

### Another controller can commission this one

103 §9.2.3 makes the controller accept the commands of Tables 21 and 22 from other
application controllers and leaves a correctly working system to integration. The
controller accepts them; it refuses only what it can see is wrong itself, and it records
the rest.

- The DALI settings surface shows each bus-unit variable and whether the operator or the
  wire set it last; a value the wire changed is logged and counted.
- A unit refuses an operator's short address equal to its configured peer's, and the
  reverse. A pair on one address would superimpose every addressed answer, and the other
  master would re-address one of them.
- The pair's takeover does not depend on any short address: the probe stays broadcast.
- Planned switchover verifies what it hands over. After `ENABLE APPLICATION CONTROLLER`
  at `peer_device_short_address`, the active unit asks the peer
  `QUERY APPLICATION CONTROLLER ENABLED` at that address — a passive unit now answers
  queries — and stands down only on YES. A peer that does not answer, at an address the
  wire may have moved, leaves the bus with the active unit and fails the request with a
  named cause.

### Rejected alternatives

- **The strict arbitration answer, with liveness read by the standby.**
  `applicationControllerError` exists only in `QUERY DEVICE STATUS`, and a broadcast one
  is garbled by every other control device on the segment, so the standby would read it
  with a second frame addressed to its peer. That frame either doubles the probe's wire
  time — the probe already destroys foreign frames that start with it — or takes every
  other period from it and slows the detection of a dead peer. It depends on the peer's
  short address, which this design makes writable from the wire, so a moved address has to
  read as no verdict, or it would bring up a second master, and the wedge detection is lost
  without a sign. A wedged controller still answering YES would also keep a third-party
  type B unit off the bus for good.
- **Holding on a busy line, without touching the interrupt** — hides duplicate short
  addresses, see above.
- **Staging each answer from the drain on a 1 ms poll**, as the arbitration answer was —
  a deadline of a few milliseconds per query on an ordinary task, whose margin shrinks with
  every worker and every frame of network traffic on its core.
- **Waking the drain through a software-pended lower-level interrupt** — possible on the
  P4, where an edge-triggered line's pending bit is writable, but it shortens only the
  wake; the per-query deadline stays.
- **The whole model in the interrupt** — the same answers with the whole of Part 103's
  state machine above the kernel, timing no host test measures, and NVM values written
  beside the registry.
- **Refusing commissioning from the wire** — 103 §9.2.3 requires accepting it, and a
  certification run commissions the device.
- **Ignoring quiescent mode** — it exists so that another controller's address search is
  not frustrated by forward frames; ignoring it breaks exactly that search.

## Consequences

- DALI-2 certification of the controller becomes reachable; until it is built conformance
  §8 stays open.
- Another controller can commission this one: its short address, groups and random
  address become state the wire can change, and the settings surface shows where a value
  came from.
- A master's commissioning run stops this controller from driving light for its length,
  and at most 15 min after a `START QUIESCENT MODE` without a `STOP`.
- The redundancy pair's arbitration probe keeps its frame and its broadcast, answered by
  the model; the lease's lapse becomes a reported application-controller error.
- No task latency reaches the answer slot. The drain runs at the 10 ms pace in both roles,
  and the interrupt carries one decode and one lookup per 24-bit frame.
- The 24-bit decoder is interrupt code: a change to it is proven on the linked image and
  by the parity test.
