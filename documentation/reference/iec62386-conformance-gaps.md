# IEC 62386 conformance gaps

What the controller does not implement, or implements differently from IEC 62386 and
DiiA(SW)098bp: open gaps and accepted deviations only. An item leaves this document when
it is implemented.

Scope: conformance of the controller's own behaviour. The rules it does follow →
[09 DALI protocol rules](../architecture/09-dali-protocol-rules.md); product defects →
[`known-issues.md`](../product-design/known-issues.md); confirmations owed by the
installation → `tools/hil/STRATEGY.md`. Section numbers are stable references and are
not renumbered when a section closes.

## 0. Method and sources

- The IEC 62386 and DiiA PDFs, and the digest of DiiA(SW)098bp v1.10, are kept
  locally, outside the repository.
- For each part the normative text was read — scope, clause 9 (method of operation),
  clause 10 (variables), clause 11 (commands) and every command, variable and memory-bank
  table. Test procedures (clause 12) were read only where a sequence settles a question
  the normative text leaves open.

| Parts | Depth |
| --- | --- |
| 101 (2014+AMD1, 2018 consolidated) | §4.6, §4.7, §4.11, §7, §8.2, §8.3, §9.1–9.7; Tables 17, 20–27 |
| 102, 103 | clauses 9–11 complete |
| 201–210, 216–224 | clauses 9–11 complete |
| 250–253 (DiiA 2019 and IEC 2023 previews) | memory-bank tables complete |
| 301–306, 332, 333 | scope, instance or feature type, input-value model |
| 104, 105, DiiA 150, 341, 342, 351 | scope and the clauses that touch a shared bus |

- Not covered: the 101 ed3 (2022) amendments and 332 Annex A.
- Ratings: `BLOCKING`, `SHOULD FIX`, `GAP` and `MINOR` rate the consequence, not the
  effort; `ACCEPTED` marks a deliberate deviation and gives its reason.

## 1. Part 101 — multi-master behaviour

### 1.6 Command iteration is paced by the input device — `MINOR`

101 §9.4 (Tables 26 and 27) with 102 §9.8.2: `UP` / `DOWN` repeated at ≤ 175 ms form an
iteration — the first frame steps, the gear then fades at `fadeRate`, and each further
frame 12,6–180 ms after the previous one extends it, even with other frames in between.
The rules engine sends one `UP` or `DOWN` per `long_press_repeat` (`.dim_up()` /
`.dim_down()`), so the frames come at the push button's `tRepeat` (160 ms by default),
widened by the jitter of the rule's latency, and nothing on our side keeps the gap under
175 ms. Each frame fades for 200 ms, so a longer gap or a lost repeat pauses the fade
until the next frame. REST and Home Assistant offer no hold-to-dim, and
`STEP UP` / `STEP DOWN` are never sent. Part 209's colour steps define no iteration:
each frame is one step.

### 1.9 The §9.1.3 destroy-area test is not evaluated — `ACCEPTED`

§9.1.3 orders a break only when the aborted signal meets a destroy area of Tables 23/24;
otherwise the transmitter returns to collision avoidance, because the frame on the bus is
still valid (Note 1). The PHY breaks on every detected collision. That is conservative —
every receiver rejects the frame — and the destroy-area boundaries (100 / 356,7 /
433,3 / 476,7 µs) are about one 104 µs sampling tick apart, which a polled receiver
cannot resolve.

## 2. Part 102 — standard commands

### 2.2 Commands with no product caller

| Opcode | Command | Consequence |
| --- | --- | --- |
| 0xAA | `QUERY CONTROL GEAR FAILURE` | Modelled, never sent (ed2 §9.16.2, §11.5.4). Status bit 0 covers part of it; it is the second candidate for a broadcast health probe. |
| 0xA6 | `QUERY MANUFACTURER SPECIFIC MODE` | Modelled, never sent (§9.9, §11.5.27). |
| 0x23 / 0x9E | `SET` / `QUERY OPERATING MODE` | Never sent. A gear parked in a manufacturer-specific mode (0x80–0xFF) looks healthy, although there the standard guarantees only the 101 bus behaviour, the operating-mode commands and the special commands (§9.9) — not level commands or `QUERY STATUS`. |
| 0xC9, 0x24 | `WRITE MEMORY LOCATION – NO REPLY`, `RESET MEMORY BANK` | Never sent: the bank 1 `memory_luminaire` write uses the answering form, and no bank is reset — §3. |
| 0x08 | `ON AND STEP UP` | Never sent by the product. It lights dark members at their own `minLevel` (§11.3.10). |
| 0x03, 0x04, 0x07 | `STEP UP`, `STEP DOWN`, `STEP DOWN AND OFF` | Never sent — §1.6. |

## 3. Memory banks

| Finding | Detail |
| --- | --- |
| Write path limited to bank 1 luminaire data | `ENABLE WRITE MEMORY` and `WRITE MEMORY LOCATION` are sent only by the typed write of Part 251's luminaire data in bank 1 ([09 §Device types and memory banks](../architecture/09-dali-protocol-rules.md#device-types-and-memory-banks)); `RESET MEMORY BANK` is never sent. Bank 1's OEM GTIN and identification number, Part 251's content format ID, bank 206's resettable counters, bank 207's luminaire constants, the failure counters banks 205 and 206 reset only through `RESET MEMORY BANK`, and the whole-bank latch of Parts 252 (§9.2.3) and 253 (§9.2.4) cannot be written. |
| Part 251 V4 and V5 fields | Written on the rule DiiA 251 gives the V3 fields (lockable, NVM). The 2023 IEC edition that defines the fields past `0x77` was not available in full, so their memory type is not checked against it. |
| Bank 201 (DiiA 250, DT49) | Not read: the integrated bus power supply's guaranteed and maximum current (0x04, 0x05) and its on/off status (0x06, NVM-RW), nor `QUERY ACTIVE POWER SUPPLY`. |

## 4. Part 209 — DT8 colour

### 4.2 Commands not implemented

| Command | Status |
| --- | --- |
| 245 `ASSIGN COLOUR TO LINKED CHANNEL` and its query 252 | `ACCEPTED`: not modelled. It assigns colours to channels linked in `RGBWAF CONTROL`, a state the product's `0x80` (driven channels unlinked) never creates, and a wrong assignment on working gear is destructive persistent configuration. DiiA v1.10 reserves both commands. |
| 240, 241, 246 (`STORE TY` / `xy-COORDINATE PRIMARY N`, `START AUTO CALIBRATION`) | Absent: DiiA 10.6.14 and 10.6.16 forbid Primary-N and auto-calibration. Any 239–246 opcode without a typed variant is refused rather than sent once. |

### 4.2.1 `RGBWAF CONTROL` across a power cycle — open question

- 209 Table 8 makes `RGBWAF CONTROL` `1 byte RAM` with default 63 (all channels linked),
  and footnote d says a RAM variable's power-up value shall be its default. 102 §9.13
  says a device shall retain its most recent configuration after an external power
  cycle, and its exception list does not name this variable. With 209 footnote b calling
  `1 byte RAM` non-persistent, the two clauses read as complementary and a gear that keeps
  the byte deviates — probably, since 102 never defines "configuration".
- One installed RGBWAF gear keeps the byte across mains loss: after a cycle that set
  `powerCycleSeen` on every gear it still answers the `0x80` the product asserted, with
  nothing rewritten. Telling whether the
  `0xC0` it held before (extended colour control, 098bp §10.6.13.5) is a vendor default
  or earlier writes takes a factory-reset gear or a second vendor
  (`tools/hil/rgbwaf_control_probe.py` reads the evidence). Untested: whether the same
  gear honours footnote d for its other RAM variables.
- The controller does not depend on the answer: it never assumes the boot state
  ([09](../architecture/09-dali-protocol-rules.md)).

### 4.3 New-edition variables (DiiA 10.6.13) — `GAP`

209:2011 leaves commands 239 and 244 reserved; the new edition assigns them.
`powerRatio`, `deratingFactor`, `enabledChannels`, REPORT LightOutput R–F (241–246),
`TcStepIncrement` (239) and `STORE ENABLED CHANNELS` (244) are not implemented. They
matter only for DALI-2 certification against 209 ed2.

## 5. Part 207 — DT6 LED

`REFERENCE SYSTEM POWER` (224), `ENABLE` / `DISABLE CURRENT PROTECTOR` (225, 226) and
`STORE DTR AS FAST FADE TIME` (228) are never written — commissioning settings with no
product surface. They are already declared send-twice by opcode range, and a writer
inherits the prelude-routing rule for opcodes shared with DT8 and Part 218.

## 8. Part 103 — the controller as a control device — `GAP`

101 §4.6.5 requires a multi-master application controller to conform to Part 103.
Built: 24-bit transmit and receive; commissioning, instance configuration, event
decoding and Part 332 feedback for input devices; and the controller itself as a bus
unit ([ADR-030](../architecture/decisions/ADR-030-controller-as-a-part-103-bus-unit.md)):
one model fed every frame on the segment answers the device queries (status,
capabilities, version, DTR content, memory bank 0 without a GTIN) and the special
queries from the interrupt, executes another controller's commissioning (`INITIALISE`,
`RANDOMISE`, address search, `COMPARE`, address programming) and its `ENABLE` /
`DISABLE APPLICATION CONTROLLER`, and hands every NVM change to the registry, which
persists it beside the short address set in `/api/v1/settings/dali`. While another
controller holds quiescent mode, no forward frame of ours leaves. A controller whose
watched workers stop turning reports an application controller error in either role.
Another controller's `SEND TESTFRAME` puts its frame on the wire at the priority it
names, in either role. After every boot the controller sends `POWER NOTIFICATION` while
`powerCycleNotification` is ENABLED.

Three deviations are `ACCEPTED`. Two are decided in
[ADR-018](../architecture/decisions/ADR-018-controller-redundancy.md): a passive standby
still sends the arbitration probe, which is how it finds an unowned bus, although 103
§9.9.1 allows a passive controller no forward frame but the power notification; and the
answer lapses with the liveness lease, so a wedged controller stops holding the bus,
although 103 §11.6.16 ties it to `applicationActive` alone. The third is decided in
ADR-030: our own Part 103 commissioning, once started, runs on after the controller is
made passive, so it ends the quiescent mode it began rather than leaving the bus quiet
until the 15-minute timer lapses.

Not built for the controller as a bus unit: a proven latency
for `SEND TESTFRAME`, which test 12.3.16 bounds by one settling time; a bench measurement
of the `POWER NOTIFICATION` time against test 12.4.16, whose window this design counts
from hydration; and the identification
screen of `IDENTIFY DEVICE`.
DiiA 351's bank 201 is omitted by decision, since 103:2014 §9.10.9 reserves banks
200–255. DALI-2 certification is out of reach until the rest is built.

## 9. Parts not implemented

### 9.1 Control-gear types

| Part | DT | What cannot be asked |
| --- | --- | --- |
| 202 | 1 | Self-contained emergency lighting: modes, battery, function and duration tests, their scheduling and results — no statutory evidence. The largest 2xx gap. |
| 203 | 2 | HID: thermal load and overload time, HID failure and status. |
| 204 | 3 | Low-voltage halogen: protector and reference block. |
| 205 | 4 | Incandescent dimmer: the only 2xx part with live electrical metering by command. |
| 206 | 5 | 1–10 V converter: output range, pull-up, physical minimum, dimming curve. |
| 208 | 7 | Switching: the thresholds and the error hold-off are read (`dt7_switching`) but not written, and `REFERENCE SYSTEM POWER` is never sent. |
| 210 | 9 | Sequencer: up to 250 points run by the gear itself. Out of scope: no DT9 gear, and the product drives slow dynamics from the controller. |

### 9.2 Control-gear features

| Part | DT | Feature |
| --- | --- | --- |
| 216 | 15 | Load referencing (`loadDeviation`, `REFERENCE SYSTEM POWER`) |
| 217 | 16 | Thermal gear protection: flags and counters |
| 218 | 17 | Dimming curve selection — `ACCEPTED`: unsupported, and it shares opcodes with DT6 and DT8 |
| 220 | 19 | Centrally supplied emergency operation |
| 221 | 20 | Load shedding: three reduction factors |
| 222 | 21 | Thermal lamp protection |
| 224 | 23 | Non-replaceable light source: the advertisement is the payload, kept in the device-type set |

253 §4.3: a gear supporting device type 52 should not support 16 or 21, because bank 205
supersedes both.

### 9.4 Control devices and system parts

| Part | Status |
| --- | --- |
| 305, 306 | Colour and general-purpose sensor events are carried untyped (instance type and raw value). |
| 302, 304 | Input values travel as the raw 10-bit magnitude: the scaling (103 AMD1 §9.8.2) is not available. |
| DiiA 341, 342 | Bluetooth Mesh and Zigbee gateways: relevant only if the controller becomes a gateway. |
| DiiA 351 | Luminaire-mounted control devices: not implemented as a device; the controller answers their arbitration probe (§8). |
| 104, 105, DiiA 150 | Not needed: alternative media, firmware update over DALI, auxiliary supply. |

## 10. DiiA(SW)098bp — open requirements

| § | Requirement | Status |
| --- | --- | --- |
| 5.6 | `lampFailure` depends on the light-source type; a converter (type 253) detects failure at its output | `QUERY LIGHT SOURCE TYPE` is read and shown; failure handling does not use it. |
| 7.1.1 | An NVM variable survives a power cycle only ≥ 30 s after its write (or 300 ms after `SAVE PERSISTENT VARIABLES`) | Not modelled: scenes programmed just before a power cut are lost silently. |
| 10.6.13 | New-edition 209 variables | §4.3. |

## 15. Input devices

### 15.3 Coexistence with a built-in application controller — `GAP`

A Part 103 bus unit may contain an application controller that drives gear itself —
standalone DALI-2 wall switches, whose button-to-action mapping is vendor-specific.
103 §9.9.1 lets another controller switch it off with `DISABLE APPLICATION CONTROLLER`;
it then sends no forward frames but still answers and monitors. The input-device scan
reads `QUERY DEVICE CAPABILITIES` (bit 0 `applicationControllerPresent`) and
`QUERY DEVICE STATUS`, so such a unit is visible, but no product path sends it
`DISABLE APPLICATION CONTROLLER`: it keeps driving gear beside this controller.

### 15.4 Part 333 manual configuration is heard, not managed — `GAP`

The manual configuration event (333 §9.3) is decoded, fires the rules trigger
`manual config changed` and dates the record of every address it names. What the device
holds is not read: no scan sends `QUERY MANUAL CONFIGURATION 103` / `3xx`, so
`manual_config_active` stays `false`, and a write to a variable under local control is
discarded by the device without the product knowing. `SET MANUAL CONFIGURATION`, the
capability queries and `QUERY ANY MANUAL CONFIGURATION ENABLED` are never sent.

## 16. Control gear never speaks unbidden

### 16.3 Power cycles nobody looks for — `GAP` by default, `MINOR` with the watch on

102 Table 12 bit 7, `powerCycleSeen`, is set at power-on. By default nothing looks for it
unprompted: only an explicit attribute read or the enabled poller's own status reads, at
its interval, can report it, and a disabled poller reads none. The poller setting
`power_cycle_watch`, off by default, closes the gap: the bus-health probe then ends with
a broadcast `QUERY POWER FAILURE` (102 §11.5.15) behind its positive control and runs
with the poller off; a YES sweeps the adapter's gear with status reads unless the enabled
poller reads `runtime_status` in its cycle, and the registry forgets the RAM state of a
gear whose bit it sees set after it knew it clear
([09](../architecture/09-dali-protocol-rules.md) §Faults and identification; schedule,
sweep and hold-off: [poller](../product-design/runtime-modules/poller/README.md)). With
the watch on, what it does not see:

- A second power cycle before anything clears the bit: the bit is already set.
- A cycle after a clearing command that reached the gear but committed nothing to the
  registry — a foreign master's `UP`, `DOWN`, `RESET` or a `DAPC` of MASK, a group,
  broadcast or scene frame reaching gear no virtual lamp binds, a scene row with no
  applied target: the registry still holds the bit as set or unknown, so a set bit is
  no news.
- A cycle whose bit a committed level command clears before a status read sees it —
  an `OFF`, a `DAPC` or a scene, from the operator, Home Assistant or a rule: the gear
  and the registry both hold the bit clear, and the later read agrees. The window runs
  from the power's return to the gear's status read, in both modes.
- A gear the sweep does not read: with `skip_unbound_virtual_lamps` the sweep skips
  gear no virtual lamp binds.
- A cycle while the answer stands at YES because some other gear nobody drives keeps
  its bit set: it waits for the hold-off's next sweep, which counts answers from the end
  of the last sweep, about an hour with the poller off.
- How soon, which is how long that window stays open: without a cycle that reads
  `runtime_status`, up to one probe period to the YES and then the gear's place in the
  sweep; with one, the gear's turn in the cycle, about one `interval_ms` when a cycle
  reads every target and longer when the wire budget stretches it, and for a gear the
  cycle saw absent, the first cycle after a YES that is news — the first, the first
  after a NO, then one per `RESWEEP_AFTER_YES` answers
  ([poller](../product-design/runtime-modules/poller/README.md)).

## 17. Priority

Ordered by consequence per unit of work:

1. §16.3 — `power_cycle_watch` closes it and is off by default, so the work left is
   its default.
2. §9.1 Part 202 — needed the day an emergency fixture joins the segment.
3. §8 — the controller as a Part 103 control device; the route to DALI-2 certification.
4. §3 — the remaining memory-bank writes (bank 1's OEM identity, `RESET MEMORY BANK`), when a commissioning surface asks for them.
