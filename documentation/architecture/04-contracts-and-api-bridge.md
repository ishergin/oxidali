# 04 Contracts and the API Bridge

What crosses a boundary and in which encoding: the shared message types, how a bus
payload is declared and encoded, where JSON is allowed, how a counter reaches a surface,
and how an HTTP request waits for its confirmation.

Scope: contract mechanics. Channel semantics → [03](03-bus-and-backpressure.md); the
external REST, WebSocket and MQTT contracts and the bus families →
[`../product-design/rest-api/`](../product-design/rest-api/README.md) and
[`../product-design/bus-contracts/`](../product-design/bus-contracts/README.md); the
change checklist → [`contract-stability-checklist.md`](../product-design/contract-stability-checklist.md).

## One home per type

- Canonical bus and shared types live in `dali2rust-contracts/src/msg/`; the typed bus
  and its `postcard` budget are [ADR-001](decisions/ADR-001-typed-bus-and-postcard-budget.md).
- No parallel DTOs in `api`, `adapters` or tests. Wrappers live in
  `dali2rust-api/src/contracts/`; a payload's builder and mapping change in one place.

## Bus payloads

[ADR-005](decisions/ADR-005-declarative-contract-macro.md).

- Every command and event payload is declared inside `declare_bus_payloads!` with a
  mandatory worst-case `budget` sample; the macro generates the struct, the union
  variant, `From`, and the budget, discriminant and round-trip tests. `, max = N` needs a
  stated justification; no variant uses it.
- The budget tests size an envelope whose meta varies only in `sender_id`,
  `correlation_id` and `target_adapter_id`, because nothing stamps `sequence_no`,
  `timestamp_ms`, `bus_id` or the cluster and proxy origin ids after build. A layer that
  starts stamping them widens the probe first, or frames near 128 B are refused at
  ingress.
- Variant order is the `postcard` discriminant and is append-only, frozen by the
  `FROZEN_*_ORDER` snapshots. A new variant is appended; the snapshot is never reordered
  to make a failure go away.
- Envelopes come from the generic `command_envelope` / `event_envelope` with a named
  struct literal or an associated constructor; confirmations from the
  `dali2rust_contracts::bus::confirm_build` helpers.
  Patch-mask bits are associated consts.
- Dispatch is table-driven: `dispatch_bus_commands!` / `dispatch_bus_events!` generate
  the `<WORKER>_HANDLED_*` consts the subscriptions declare. No hand-written catch-all
  arm.
- The wire codec is `postcard`, and an encoded frame is at most `MAX_BUS_WIRE_BYTES`
  (128) or the publish fails. Bus messages and registry state are fixed-size: no
  `String`, `Vec` or `serde_json` (`verify_fixed_bus_guardrails.sh`).
- The DALI wire byte field is `wire_address`, never a bare `address`.
- A 16-bit control-gear frame travels as `DaliCommandPayload`; Part 103's 24-bit frames
  have payload shapes of their own and are never squeezed into that tuple. Product code
  publishes semantic commands; the raw frame is diagnostic
  ([09](09-dali-protocol-rules.md)).

## JSON boundaries

- JSON exists only at the REST, WebSocket and MQTT / Home Assistant boundaries, never on
  the bus.
- WebSocket frames are built in `dali2rust-api/src/ws/` and Home Assistant payloads in
  `dali2rust-api/src/ha/`, nowhere else; the runtime crates carry already-serialized
  strings, and `ws-runtime` has no `serde_json` dependency.
- A WebSocket payload has the shape of the REST resource it mirrors, so a client that
  falls back to polling parses one dialect.
- Path parameters have their own percent-decoder: `+` is an ordinary character in a path
  (RFC 3986 §3.3) and a space only in a query string.

## Confirmation bridge

[ADR-002](decisions/ADR-002-http-confirmation-bridge.md).

- The bridge (`dali2rust-api::confirmation_bridge`) is a fixed pool of pending slots,
  sized from `BusConfig::confirmation_slots`, fed by one thread subscribed to the
  confirmations channel. It is the only place an HTTP request meets its confirmation.
- Correlation ids come from one `CorrelationIdAllocator` shared by the router.
- No free slot, or command ingress full → `503`. A synthetic `DeliveryRejected` frees the
  slot like a real confirmation and answers `503`. No confirmation within
  `confirmation_timeout_ms` (2000 ms) → `504`.
- A timeout releases the slot, not the command, which may still execute.
- A slot is freed only by its confirmation or by `cancel` (`ConfirmationHandle` has no
  `Drop`), so every early exit cancels the registrations it will not collect, or the pool
  leaks into permanent `503`s.
- A read-after-write handler samples the registry's apply counter strictly before it
  publishes (sampled after, it waits for an increment that already happened) and bounds
  its extra wait after the confirmation by `APPLY_WATCH_BUDGET_MS` or the confirmation
  timeout, whichever is smaller (`publish_and_await_apply`).
- Routes that answer `202` and an operation id do not wait on the bridge
  ([03](03-bus-and-backpressure.md)).

## Counter surface

- A counter is spelled in several places: the `AtomicU32` in its runtime crate, the DTO
  in `api`, the mapping in `adapters`, the worst-case sample in `api/src/ws/snapshot.rs`,
  the TypeScript mirror in `web/app/src/api/types.ts`, string keys in the HIL suite and
  BDD steps, and the product-design diagnostics table. The compiler holds only DTO →
  mapping → sample; `scripts/verify_counter_surface.py` compares the names of the rest,
  and a counter that counts the wrong thing is still a test's job.
- The gate finds a counter by its shape — a one-line `pub name: AtomicU32,` field of a
  top-level `pub struct …Counters` — and silently misses any other spelling.
- A counter kept off the surface is listed with a reason in
  `scripts/counter_surface_internal.txt`, whose entry count is frozen in
  `counter_surface_internal_budget.txt` and only goes down.
- Counters are `u32` on the wire; they wrap as their mechanism does.
- A new counter goes to `/api/v1/stats`. The periodic WebSocket diagnostics frame has a
  hard ceiling (`DIAGNOSTICS_SNAPSHOT_CEILING_BYTES`) and no headroom; raising the
  ceiling is not the fix.
