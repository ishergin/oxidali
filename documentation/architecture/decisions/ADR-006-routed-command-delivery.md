# ADR-006: Routed command delivery

Status: Accepted, amended by ADR-015 and ADR-021
Date: 2026-07-04

## Context

Every command has exactly one owning worker, declared by the macro-generated
`<WORKER>_HANDLED_COMMANDS` consts and checked by the ownership test
([ADR-005](ADR-005-declarative-contract-macro.md)). Broadcasting each command to every
subscriber and filtering worker-side wastes an inbox slot per subscriber, lets an
unrelated burst overflow an owner's inbox, and turns an undeliverable command into a
silent drop that a waiting HTTP caller only learns about from its `504`.

## Decision

- **Route by declared kind.** `subscribe_commands(capacity, handled)` registers a
  subscriber with the same `<WORKER>_HANDLED_COMMANDS` const that drives its dispatch
  table. The bus task routes each command by `variant_index()` (the `postcard`
  discriminant) through a mask table built once at spawn; an unknown name panics at
  spawn.
- **Unicast in production.** The ownership test makes the table total with exactly one
  owner per kind. Several subscribers may declare one kind (several DALI worker
  instances); `target_adapter_id` stays a consumer-side filter, never a route key.
- **Zero accepted deliveries is a rejection.** An unrouted kind or an overflow of every
  declared inbox increments `commands_unrouted` / `receiver_overflow` and injects a
  synthetic `DeliveryRejected` confirmation, which the HTTP bridge turns into a fast
  `503` ([ADR-002](ADR-002-http-confirmation-bridge.md)).
- **The rejection bypasses the confirmations ingress**, fanned straight to the
  confirmation subscribers, so a rejection burst larger than that ingress cannot drop its
  own rejections ([03](../03-bus-and-backpressure.md) §Overload and rejection).
- **Every route that publishes has exactly one mechanism that hears a rejection.** The
  confirmation bridge hears it when a caller is waiting. When the route answered
  `202` + operation id, the operation tracker hears it through its own confirmations
  inbox and ends the operation `failed` with `CommandsIngressOverload`, instead of
  letting it run into its TTL. The DALI worker's own refusals (a disabled adapter, a
  passive controller) follow the same rule: a `202` route receives a failed worker
  signal, a request-scoped route receives the confirmation, and both name the cause.
- **Confirmations stay broadcast.** Events are routed by declared set too
  ([ADR-015](ADR-015-routed-event-delivery.md)).

### Rejected alternatives

- **Broadcast plus per-subscriber "required" policies** — index-coupled configuration,
  N frame clones per command, silent drops for the non-required workers.
- **Matching names at dispatch** — string scans on the hot path; `variant_index()` is a
  constant per arm.
- **A routing enum owned by the bus crate** — a second source of truth beside the
  dispatch tables.

## Consequences

- An owner's inbox sees only the kinds it owns, so its `receiver_overflow` counter is a
  meaningful signal rather than broadcast noise.
- Delivery failure is deterministic for every command family; `commands_unrouted`
  exposes a partial composition instead of a hang.
- With several instances declaring one kind, a rejection fires only when none of them
  accepted. If instances proliferate, the route key widens to
  (kind, `target_adapter_id`).
- Queue depths, the channel model and the listener rule for new routes are described in
  [03-bus-and-backpressure.md](../03-bus-and-backpressure.md).
