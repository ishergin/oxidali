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

### 1.6 Command iteration is not used — `GAP`

101 §9.4 (Tables 26 and 27) with 102 §9.8.2: `UP` / `DOWN` repeated at ≤ 175 ms form an
iteration — the first frame steps, the gear then fades at `fadeRate`, and each further
frame 12,6–180 ms after the previous one extends it, even with other frames in between.
No product path sends `UP`, `DOWN`, `STEP UP` or `STEP DOWN` (the sniffer translator only
decodes them from other masters); hold-to-dim is the rules engine repeating relative
DAPC. Part 209's colour steps define no iteration: each frame is one step.

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
| 0x23 / 0x9E | `SET` / `QUERY OPERATING MODE` | Never sent. A gear parked in a manufacturer-specific mode (0x80–0xFF), where the standard guarantees nothing else, looks healthy. |
| 0x81, 0xC7, 0xC9, 0x24 | `ENABLE WRITE MEMORY`, `WRITE MEMORY LOCATION` (± `NO REPLY`), `RESET MEMORY BANK` | No memory-bank write path — §3. |
| 0x05, 0x06, 0x08 | `RECALL MAX LEVEL`, `RECALL MIN LEVEL`, `ON AND STEP UP` | Never sent by the product. With the writable `maxLevel` / `minLevel` they are two of the three presets reachable by one group frame (the third is a scene); `ON AND STEP UP` lights dark members at their own `minLevel` (§11.3.10). |
| 0x01–0x04, 0x07 | `UP`, `DOWN`, `STEP UP`, `STEP DOWN`, `STEP DOWN AND OFF` | Never sent — §1.6. |
| 0xFF | `QUERY EXTENDED VERSION NUMBER` | Sent only behind `ENABLE DEVICE TYPE 6`, so no other device type's extended version can be read, although DiiA 12.7.2 asks for one per discovered type. |

## 3. Memory banks

| Finding | Detail |
| --- | --- |
| No write path | `ENABLE WRITE MEMORY`, `WRITE MEMORY LOCATION` and `RESET MEMORY BANK` are never sent. Bank 1's OEM GTIN and identification number, Part 251's NVM-RW luminaire data, bank 206's resettable counters and Part 253's lock-byte latch (§9.2.3) cannot be written. |
| Bank 201 (DiiA 250, DT49) | Not read: integrated bus power supply current and status. |

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
- The installed RGBWAF fixture keeps the byte across mains loss: after a cycle that set
  `powerCycleSeen` on every fixture it still answers the `0x80` the product asserted,
  with nothing rewritten. So the origin of the `0xC0` (extended colour control under
  098bp §10.6.13.5) it held before the product first asserted `0x80` — a vendor default
  or earlier writes — cannot be read off a reboot. Settling it needs a factory-reset
  fixture or a second vendor; `tools/hil/rgbwaf_control_probe.py` reads the evidence.
- Untested: whether the same gear honours footnote d for its other RAM variables (a RAM
  variable parked off its default, then a mains cycle).
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
decoding and Part 332 feedback for input devices; and, for the controller itself, an
answer to `QUERY APPLICATION CONTROLLER ENABLED` (DiiA 351 §7's arbitration probe) while
`applicationActive`, obedience to `ENABLE` / `DISABLE APPLICATION CONTROLLER` from
another controller, and a short address set in `/api/v1/settings/dali`.

Not built for the controller as a bus unit: `QUERY DEVICE STATUS` and the other device
queries — status, capabilities, version, DTR content (answering with bits it cannot
vouch for would be worse than silence); memory bank 0 with its identity per logical
unit, and DiiA 351's bank 201 (device type and the type B arbitration byte, which is
never written `0x00`: that value disables arbitration); being commissioned by another
controller (`INITIALISE`, `RANDOMISE`, address search, `COMPARE`, address programming);
`POWER NOTIFICATION`; and quiescent mode. DALI-2 certification is out of reach until
they are.

## 9. Parts not implemented

### 9.1 Control-gear types

| Part | DT | What cannot be asked |
| --- | --- | --- |
| 202 | 1 | Self-contained emergency lighting: modes, battery, function and duration tests, their scheduling and results — no statutory evidence. The largest 2xx gap. |
| 203 | 2 | HID: thermal load and overload time, HID failure and status. |
| 204 | 3 | Low-voltage halogen: protector and reference block. |
| 205 | 4 | Incandescent dimmer: the only 2xx part with live electrical metering by command. |
| 206 | 5 | 1–10 V converter: output range, pull-up, physical minimum, dimming curve. |
| 208 | 7 | Switching: four programmable thresholds, error hold-off, switch status — the likely next need after DT8. |
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
| 333 | Manual configuration: not implemented. |
| DiiA 341, 342 | Bluetooth Mesh and Zigbee gateways: relevant only if the controller becomes a gateway. |
| DiiA 351 | Luminaire-mounted control devices: not implemented as a device; the controller answers their arbitration probe (§8). |
| 104, 105, DiiA 150 | Not needed: alternative media, firmware update over DALI, auxiliary supply. |

## 10. DiiA(SW)098bp — open requirements

| § | Requirement | Status |
| --- | --- | --- |
| 5.6 | `lampFailure` depends on the light-source type; a converter (type 253) detects failure at its output | `QUERY LIGHT SOURCE TYPE` is read and shown; failure handling does not use it. |
| 7.1.1 | An NVM variable survives a power cycle only ≥ 30 s after its write (or 300 ms after `SAVE PERSISTENT VARIABLES`) | Not modelled: scenes programmed just before a power cut are lost silently. |
| 10.6.13 | New-edition 209 variables | §4.3. |
| 12.7.2 | `ENABLE DEVICE TYPE x` → `QUERY EXTENDED VERSION NUMBER` per discovered type | DT6 only — §2.2. |

## 15. Input devices

### 15.3 Coexistence with a built-in application controller — `GAP`

A Part 103 bus unit may contain an application controller that drives gear itself —
standalone DALI-2 wall switches, whose button-to-action mapping is vendor-specific.
103 §9.9.1 lets another controller switch it off with `DISABLE APPLICATION CONTROLLER`;
it then sends no forward frames but still answers and monitors. The input-device scan
reads `QUERY DEVICE CAPABILITIES` (bit 0 `applicationControllerPresent`) and
`QUERY DEVICE STATUS`, so such a unit is visible, but no product path sends it
`DISABLE APPLICATION CONTROLLER`: it keeps driving gear beside this controller.

## 16. Control gear never speaks unbidden

### 16.3 Nothing looks for `powerCycleSeen` unprompted — `GAP`

102 Table 12 bit 7 is set at power-on, so a gear that shows it has lost power since it
was last driven. A read that sees the bit appear makes the registry forget the RAM state
it held for that gear ([09](../architecture/09-dali-protocol-rules.md)), but only an
explicit attribute read or the poller, which is off by default, reads it. The cheap
detector is a broadcast `QUERY POWER FAILURE` (0x9B, 102 §11.5.15): any answer means some
gear has cycled. As a boolean broadcast query it needs a positive control.

## 17. Priority

Ordered by consequence per unit of work:

1. §16.3 — look for `powerCycleSeen` unprompted, with a broadcast `QUERY POWER FAILURE`.
2. §2.2 `0xFF`, §10 12.7.2 — the extended version of every discovered device type.
3. §9.1 Part 202 — needed the day an emergency fixture joins the segment.
4. §8 — the controller as a Part 103 control device; the route to DALI-2 certification.
5. §3 — a memory-bank write path, when a commissioning surface asks for one.
