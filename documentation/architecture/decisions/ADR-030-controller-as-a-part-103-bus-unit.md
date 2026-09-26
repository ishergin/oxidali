# ADR-030: The controller as a Part 103 bus unit

Status: Deferred
Date: 2026-09-26

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
the arbitration answer is wrong for everything else: 103 §9.9.1 keeps a passive
controller answering queries.

**Nothing in this ADR is built.** It records the design and the decisions it needs before
code.

## Decision

The design, deferred:

- **One stateful model decides every answer.** A pure `Device103Unit` in
  `dali2rust-domain` owns the device's RAM variables (103 Table 17: `DTR0`–`DTR2`,
  `searchAddress`, the initialisation state and timer, quiescent mode, `writeEnableState`,
  `powerCycleSeen`, `resetState`) and a copy of its NVM ones (`shortAddress`,
  `randomAddress`, `deviceGroups`, `powerCycleNotification`, `applicationActive`). It
  takes every frame the sniffer captured, in wire order — 16-bit, 24-bit and backward, so
  a frame between two halves splits a send-twice pair — and returns an optional answer
  byte and a list of effects. Our own frames never feed it.
- **The task decides, the interrupt transmits, as now.** The `dali-sniff` drain feeds the
  model and stages its answer in the answer cell before any logging; the arbitration
  table becomes the degenerate case of the model. A capture is still routed by its length.
- **NVM changes go through the registry.** The model answers from its own copy at once
  (101 §9.7 executes a command before the next frame) and publishes the change as a
  registry command, so the registry stays the single writer; the model hydrates from the
  DALI settings slice. A short address set by another controller's
  `PROGRAM SHORT ADDRESS` lands in `/api/v1/settings/dali`.
- **Answer only what the controller can vouch for.** Capabilities `0x01` (application
  controller, no instances), a status byte computed from real state, version bytes only
  for what is built, memory bank 0 without a GTIN it does not own (103 has no "not
  available" GTIN, so the identification number from the eFuse MAC is the unique part).
  No bank 201: 103:2014 §9.10.9 forbids banks 200–255 and test 12.6.1 fails bank 0's
  last-bank byte above 199.
- **Quiescent mode gates every forward frame we send**, the arbitration probe included,
  while queries are still answered (103 §9.9.3).
- **A lost capture invalidates the state it would have changed.** A dropped ring entry
  (`take_sniff_dropped`) clears the `DTR`s and the search state, and answers that depend
  on them stay silent until they are set again.

## Decisions owed before code

1. **The sniffer poll becomes permanent.** An answerer that must answer any addressed
   query is always armed, so the 1 ms `dali-sniff` poll that today runs only while the
   arbitration table is armed runs always ([ADR-003](ADR-003-isr-owned-phy.md) rules out
   an interrupt-to-task wake). Its CPU and stack cost is measured on the bench before this
   is accepted.
2. **Answering on a busy line.** 101 Table 17 footnote a starts an answer in its slot even
   when another unit has started one. Today the interrupt holds while the line is busy and
   the lead yields, so the controller falls silent exactly when two units share an
   address — which hides the duplicate from `VERIFY SHORT ADDRESS`. Fixing it is an
   interrupt change and must keep `verify_dali_isr_iram.py` green.
3. **The lease and `applicationActive`.** The lease silences the arbitration answer when
   the worker stalls; 103 §11.6.16 ties that answer to `applicationActive` alone. Either
   the lapse stays a documented deviation, or liveness moves into
   `applicationControllerError`.
4. **`SEND TESTFRAME` while passive.** 103 §9.9.1 forbids forward frames from a passive
   controller except the power notification, yet test 12.3.16 expects test frames with the
   application controller disabled.
5. **What counts as a power cycle for `POWER NOTIFICATION`.** 101 Table 3 defines an
   external power cycle as a supply loss of 5 s or more; whether an OTA reboot or a
   watchdog reset sends the notification is a product choice.

## Consequences

- DALI-2 certification of the controller becomes reachable; until then conformance §8
  stays open.
- Another controller can commission this one: its short address, groups and random
  address become state the wire can change, and the settings surface must show where a
  value came from.
- The redundancy pair's arbitration probe keeps its semantics, now answered by the model.
