# ADR-016: Part 103 input devices and a bounded rule engine

Status: Accepted
Date: 2026-08-08

## Context

IEC 62386-103 input devices — push-button panels, occupancy and light sensors — must be
first-class citizens of the validated path, and a button press must change light
locally, with no cloud, broker or browser involved. The PHY, the Manchester codec and the
always-on receive path already carry 24-bit frames; the product needs a 24-bit transmit
path, the Part 103 address space and commands, registry records, and a consumer that acts
on events. That consumer is a rule engine, and the maintainer wants its language to be
replaceable as the product develops without touching the engine.

## Decision

### Wire and commands

- **24-bit frames are a capability of the one transport.** `DaliTransport` has
  `exchange_frame24` (defaulted to refuse, so a transport without a 24-bit path answers
  `transport_unsupported_24bit`). There is one wire, one PHY, one command cell and one
  collision domain, so arbitration between 16- and 24-bit traffic stays a property of the
  single bus owner: `DaliWorker` is the only task that transmits, and 24-bit frames are
  full session citizens — priority, transactions and yields
  ([ADR-017](ADR-017-dali-transactions-and-frame-priority.md)).
- **Wire tables come from the standard, and captured frames prove them.** The IEC 62386
  and DiiA texts are the source, `python-dali` a cross-check, captured frames the proof
  ([05](../05-testing-and-bdd.md)). The encodings are in
  [09](../09-dali-protocol-rules.md#part-103-control-devices), the event codes a rule
  sees in [the rule language](../../product-design/runtime-modules/rules-engine/operations.md).
- **Events are observed facts; commands are DALI work.** Inbound events ride the
  observed-frame channel into the sniffer translator, which decodes them into
  `DaliInputEventObservedEvent` and keeps its contract: no commands, no registry writes.
  Outbound commands — commissioning, instance enumeration and configuration, Part 332
  feedback — are semantic commands executed inside `DaliWorker`, the boundary gear
  commissioning already set. There is no input-device runtime crate and no extra thread:
  one wire owner, one priority table, transaction brackets for free.
- **The two address spaces are independent and encode operands differently**, so 103
  operands are dedicated types that a shared 102 helper cannot be misused with
  ([09](../09-dali-protocol-rules.md)).
- **Event scheme 2 (short address + instance) is the scheme the controller configures
  and expects.** `SET EVENT SCHEME` can fail silently — scheme 2 needs a short address,
  and an instance without one reverts to scheme 0 without notice — so the scheme is set
  by its own instance write and read back, not by commissioning, which only addresses;
  an instance on any other scheme is shown as unconfirmed. The decoder still handles
  every scheme ([09](../09-dali-protocol-rules.md)).
- **Indicator LEDs are Part 332 feedback**, a feature on the panel's own device/instance
  address over 24-bit frames — not a control-gear channel. `SELECT FEEDBACK(group)` is
  the standard's radio-button primitive, and both query-opcode dialects are supported.
  `feedbackActive` is RAM with a power-on value of false, so indicator state is soft
  state that rules re-assert rather than configuration the controller stores.
- **The controller is also a Part 103 control device** with its own short address. It
  answers `QUERY APPLICATION CONTROLLER ENABLED` from `applicationActive`, and
  `applicationActive` false stops every forward frame except the arbitration probe
  ([ADR-018](ADR-018-controller-redundancy.md)). Other device queries are not answered
  while any of their bits would have to be invented.

### Input-device storage

Input devices have their own controller-global slice with its own version (the record
carries `adapter_id`): identity, our metadata and a compact per-instance summary
([snapshots](../../product-design/bus-contracts/snapshots.md)). Instance timers and sensor
parameters are **not** persisted: Part 333 lets an installer change them at the device,
so any stored copy can lie; they are shown with their read time and re-read on demand.

### The rule engine

- **The canon is the source text.** The rules document is stored byte for byte with the
  `lang_id` of the compiler that accepted it, so comments and formatting survive a save.
  There is no printer and no `model → text → model` property: with one direction there
  is nothing to keep in agreement. The typed model is a runtime artefact, rebuilt at
  commit and at boot.
- **The language is a replaceable crate.** `dali2rust-rules-model` owns the typed graph,
  the limits and the `RuleCompiler` / `NameResolver` traits; `dali2rust-rules-lang` is one
  compiler (`LANG_RULES_V1` = 1); `dali2rust-rules-runtime` consumes only the model and
  never sees source. A new language is a new compiler crate with a new `lang_id`.
- **One grammar, on the device.** The parser runs on the httpd task (iterative, bounded
  temporaries). The web UI is a text editor with no grammar of its own: it posts source to
  `POST /api/v1/rules/parse` and renders the device's diagnostics with line and column.
- **The document compiles as a whole.** A stored document that fails to compile — at
  boot, or when a name it references stops resolving — keeps its source and runs no
  rules; one that cannot be loaded whole (an unknown `lang_id`, torn banks) runs none
  either. Both report why.
- **Rule identity is the name.** Duplicates are refused; rules sharing a trigger run in
  ascending name order, so the outcome is deterministic.
- **A bounded executor, not a VM.** Every bound is structural and every refusal is
  counted: 64 rules; 1–4 triggers per rule (OR); up to 4 conditions (AND); up to 8 source
  actions per rule and 32 after the compiler expands `def` blocks (call depth ≤ 2),
  `repeat N` (N ≤ 8, not nested) and `if/else` (depth 1); chain depth 4; 32 pending
  continuations; 16 timers and 16 variables; 12 240 bytes of source.
- **No blocking sleep in an activation.** `after` and `wait` schedule a continuation on a
  bounded timer wheel; a worker parked in `sleep` is a worker not answering the next
  press.
- **Reads come from the read model, never from the wire.** A rule never issues a DALI
  query, whose cost would dwarf the "finger to light" budget; `trigger_to_publish_ms`
  travels on each activation event.
- **A rule's light command is `Interactive`**, amending
  [ADR-013](ADR-013-wire-priority-and-yield-granularity.md): the criterion is "a human is
  waiting now", and a person is standing at the panel. A rule beats HCL and the poller
  and does not preempt, or get preempted by, a UI slider. Other actions keep their kind's
  priority; `Origin::Rules` is not background.
- **One activation, one setpoint per target.** Light effects on the same target within
  one activation are merged field by field before publishing, because the DALI worker's
  coalescing would otherwise let the second command supersede the first.
- **An action failure does not abandon the activation.** The remaining actions run, and
  the activation reports `partial`. What fired is published per activation on the
  WebSocket `rules` channel (`RulesActivationEvent`); aggregates are on
  `/api/v1/stats`. Each rule's count and last outcome live in RAM and are served with
  its REST projection; there is no persisted per-rule tally.
- **Engine state is volatile.** Variables and timers live in RAM; a persistent variable
  would rewrite flash on every assignment. The `controller_starts` and
  `controller_becomes_active` triggers rebuild modes, so a standby's takeover is the same
  edge as a boot.
- **References are scoped `(adapter_id, id)`.** The resolver fills the scope in, and the
  grammar reserves and validates an `adapter=N` qualifier, so multi-adapter reach is a
  configuration question rather than a language change.

### Rules storage

The rules document has controller-global slices of its own: the source across banks and
a manifest that carries each rule's enabled bit, so enabling or disabling one rule is a
bit flip rather than a document rewrite
([snapshots](../../product-design/bus-contracts/snapshots.md)). Writes use the staged,
bracketed `202` protocol of [ADR-012](ADR-012-async-chunked-config-writes.md).

## Consequences

- One defaulted transport method; a transport without a 24-bit path answers honestly
  instead of timing out.
- Part 103 adds no thread; the rule engine adds one (`rules_worker`).
- The host gear model carries a Part 103 device fleet, so event schemes, radio buttons and
  rule triggers are testable without hardware
  ([ADR-014](ADR-014-gear-model-and-second-dali-endpoint.md)).
- Input devices transmit unsolicited, so collisions are ordinary traffic on the segment.
- MQTT- or Home Assistant-originated triggers, a poller liveness sweep over input devices
  and closed-loop daylight harvesting are outside this decision.
- The engine's operations and the language are described in
  [runtime-modules/rules-engine](../../product-design/runtime-modules/rules-engine/README.md)
  and [rest-api/resources/rules.md](../../product-design/rest-api/resources/rules.md).
