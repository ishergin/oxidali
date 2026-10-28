# ADR-030: The controller as a Part 103 bus unit

Status: Proposed; amends ADR-018
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
  splits a send-twice pair — and returns an optional answer byte and a list of effects.
  Our own frames never feed it.
- The model answers nothing and executes nothing until it is hydrated from the registry.
  The wire is now a mutating ingress, so the boot order keeps hydration before it as it
  does before HTTP; an early `QUERY SHORT ADDRESS` answered with MASK would invite another
  master to assign a new address.
- **The task decides, the interrupt transmits, as now.** The `dali-sniff` drain feeds the
  model and stages its answer in the answer cell before any logging; the arbitration table
  becomes the degenerate case of the model. A capture is still routed by its length.
- **A lost capture invalidates the state it would have changed.** A dropped ring entry
  (`take_sniff_dropped`) clears the `DTR`s and the search state, and answers that depend
  on them stay silent until they are set again.

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

The change lives in `dali2rust-dali-phy` and keeps ADR-003, ADR-010 and ADR-025: the task
still decides, nothing new is generic or indexed, and `just p4-isr-iram-check` and
`just gear-sim-isr-iram-check` prove the linked images, which `just ci` does not build.
The gear emulator stages its answers in the same cell
([ADR-031](ADR-031-gear-emulator-on-the-standby.md)), so an emulated gear and a real one
that answer the same query now superimpose like two real gears. Several logical units
behind one interface still merge their answers before staging (101 §9.5.2).

### The sniffer polls every 1 ms

- An answerer that must answer any addressed query is always armed, so the 1 ms
  `dali-sniff` poll runs whenever the model is hydrated, in both roles
  ([ADR-003](ADR-003-isr-owned-phy.md) rules out an interrupt-to-task wake). Today it
  runs while the arbitration table is armed, that is on every active controller; the
  change reaches the passive one.
- The binding cost is staging latency, not CPU: a capture completes a closing run after
  its last edge, and the answer must reach the cell before the target slot opens, within
  `ANSWER_ARM_TARGET_IDLE_TICKS` less `RX_IDLE_LINE_HIGH_TICKS`. `answer_stage_max_ticks`
  and `answer_stage_late_total` in `/api/v1/stats` measure it, and the acceptance is a
  read-only bench run on which `answer_stage_late_total` stays 0 with the full model in
  the loop.
- The stack cost is the `dali-sniff` census against `tools/hil/stack_budget.txt`, the
  model's frames measured on the linked image (`scripts/measure_stack_frames.py`) before
  a flash.
- The shipping image has no FreeRTOS run-time statistics, so a per-task CPU share is not
  read from it. The drain reports the share of wall time it spends in its passes as a
  `/api/v1/stats` counter over the wire-load window, which the same read-only run reads.

### NVM changes go through the registry

- The model answers from its own copy at once (101 §9.7 executes a command before the
  next frame) and publishes each NVM change as evidence for the registry, so the registry
  stays the single writer; the model hydrates from the DALI settings slice.
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
- A write that needs the wire is refused with `409 controller_quiescent` and a
  `Retry-After` from the remaining time; quiescence is shown on `/api/v1/health`, the
  redundancy resource, the WebSocket and the OLED, because a master's commissioning run
  silences the lights' controller for its whole length.

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
(§12.14.1, §12.14.3) and expects the frame. The test is followed: the frame is executed in
both roles and refused in quiescent mode. The DALI worker's one gate admits it by command
kind, beside the arbitration probe and the power notification.

### Every boot is a power cycle

- 101 Table 3 defines an external power cycle by a supply loss of 5 s or more; this
  controller treats every boot as one, an OTA reboot and a watchdog reset included.
  `powerCycleSeen` is TRUE after every boot.
- `POWER NOTIFICATION` is sent once after every boot while `powerCycleNotification` is
  ENABLED: priority 2, uniformly delayed into 1,3–5 s after the power-on procedure, which
  ends at hydration (103 §9.12.2 and the window of test 12.4.16); quiescent mode started
  inside the delay suppresses it. `powerCycleNotification` keeps
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
- The passive unit pays the 1 ms poll that the active one already pays.
