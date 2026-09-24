# ADR-005: Declarative contract macro and dispatch tables

Status: Accepted
Date: 2026-07-04

## Context

A bus command or event touches the payload struct, its union variant, a `postcard`
budget test, a round-trip test, the owning worker's dispatcher and the handler. Kept by
hand, those steps drift: a budget test is forgotten, a dispatcher `match` ends in
`_ => {}` and silently swallows a command nobody routed. The contract set grows with
every stage, so the mechanical part must be generated and the ownership must be checked.

## Decision

- **One declaration point.** Every `BusCommandPayload` / `BusEventPayload` member is
  declared inside `declare_bus_payloads!` (`dali2rust-contracts/src/msg/commands.rs`,
  `events.rs`). One entry expands into the struct, the union variant, `From<T>` into the
  union, and per-variant budget, discriminant and round-trip tests.
- **Wire order is append-only.** A variant's `postcard` discriminant is its declaration
  index. The generated `*_VARIANT_NAMES` consts are frozen by snapshot tests
  (`FROZEN_COMMAND_ORDER`, `FROZEN_EVENT_ORDER`); appending extends the snapshot,
  inserting or reordering fails it.
- **A worst-case budget sample is mandatory.** Each entry carries
  `budget = <worst case>;` (full text, `Some` everywhere, maximal varints), and the
  generated test asserts that the worst-case envelope fits `MAX_BUS_WIRE_BYTES`
  ([ADR-001](ADR-001-typed-bus-and-postcard-budget.md)). `, max = N` documents a variant
  whose theoretical worst case exceeds 128 bytes; it requires a stated justification and
  freezes the measured ceiling. No variant uses it.
- **Struct literals and a generic constructor instead of per-payload builders.**
  `command_envelope` / `event_envelope` take `impl Into<Payload>`; semantic variants are
  associated constructors on the payload types, and patch-mask bits are associated
  consts. `confirm_build` remains, because `ConfirmationEnvelope` is a fixed shape
  rather than a union and does not grow with new commands.
- **Dispatch tables.** Workers route through `dispatch_bus_commands!` /
  `dispatch_bus_events!`. One invocation generates the dispatch function (running on the
  worker's own thread) and a `<WORKER>_HANDLED_*` const derived from its arms, so the two
  cannot drift. `dali2rust-adapters/tests/bus_payload_ownership.rs` asserts that every
  command has exactly one owner and every event has a consumer or a documented
  `OBSERVED_ONLY_EVENTS` entry. A worker whose events are matched inline keeps its const
  by hand, and the same test holds it.

### Rejected alternatives

- **An own proc-macro crate** — the documented escalation path if `macro_rules!` limits
  bite; not needed for the current type count.
- **Derive-crate hybrids** — the struct list and the union list stay two hand-kept lists.
- **External IDLs and code generators** — generated code is heap-backed, which violates
  the fixed-size rule, and worst-case samples become Rust in strings. Schemas flow
  outward from the Rust source (TypeScript bindings, schema hashes) if an external
  consumer ever needs them, never inward.
- **A visitor trait with default no-op methods** — the defaults reintroduce silent
  ignoring.
- **A runtime handler registry on the bus task** — runs business logic on the bus
  thread, forces `Sync` on worker state and destroys per-worker FIFO order.

## Consequences

- A new command costs one macro entry, one snapshot line, one dispatch arm in the owning
  worker, the handler, and its tests. Forgetting the owner fails the ownership test by
  name; nothing outside the owner needs editing.
- Budget coverage is total by construction.
- The same `<WORKER>_HANDLED_*` consts drive bus routing
  ([ADR-006](ADR-006-routed-command-delivery.md), [ADR-015](ADR-015-routed-event-delivery.md)).
- The step-by-step recipe for a new command or event is in
  [11-extension-recipes.md](../11-extension-recipes.md).
