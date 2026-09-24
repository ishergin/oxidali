# ADR-001: Typed bus and postcard budget

Status: Accepted
Date: 2026-06-04

## Context

HTTP handlers, runtime workers, the WebSocket and MQTT bridges and the host BDD suite
all exchange commands, events and confirmations. They need one contract surface, and
the in-memory bus that carries it must stay bounded and predictable on a controller
whose internal RAM is the scarce resource.

## Decision

- **One contract crate.** Canonical bus and shared message types live in
  `dali2rust-contracts::msg`. No other crate keeps a parallel DTO for them; API-side
  wrappers live in `dali2rust-api` and map to and from these types in one place.
- **Typed envelopes only.** Production code publishes `CommandEnvelope`,
  `EventEnvelope` and `ConfirmationEnvelope` values, never hand-built byte buffers.
- **`postcard` is the bus wire codec.** Frame sizes are measured in `postcard`
  bytes, and codec round trips are tested in that form.
- **A frame budget of 128 bytes** (`MAX_BUS_WIRE_BYTES`). Every payload is sized
  against it, and an oversize frame is refused at publish time
  (`RejectedFrameTooLarge`) before it reaches an ingress queue. There is no heap
  fallback for a large frame.
- **Fixed-size messages.** Bus payloads carry bounded text and arrays, not `String`,
  `Vec` or `serde_json::Value`.
- **JSON only at the edges.** JSON is the REST, WebSocket and MQTT boundary format
  and never the in-memory bus format.

## Consequences

- Contract evolution happens in one crate. [ADR-005](ADR-005-declarative-contract-macro.md)
  turns every payload's worst-case size into a generated test against the 128-byte
  budget.
- A payload that cannot fit is slimmed or split into a staged series
  ([ADR-012](ADR-012-async-chunked-config-writes.md)); it is never given a
  heap-backed exception. A transfer too large even for a series (slice
  export/import between controllers) bypasses the bus and sends only a short reload
  command over it.
- `verify_fixed_bus_guardrails.sh` rejects dynamic types in the contracts' message
  module, and `verify_contracts_codegen.sh` keeps the codec policy.
- The current codec and envelope rules are described in
  [04-contracts-and-api-bridge.md](../04-contracts-and-api-bridge.md).
