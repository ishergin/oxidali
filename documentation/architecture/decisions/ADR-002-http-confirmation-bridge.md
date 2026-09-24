# ADR-002: HTTP confirmation bridge

Status: Accepted
Date: 2026-06-04

## Context

HTTP handlers publish typed commands on the bus and must learn the outcome from the
worker that executed them. Each handler owning its own correlation layer would
duplicate the logic and let answers drift between routes. The wait itself is costly:
`esp_http_server` runs a single task for every socket, so a handler that blocks is
downtime for every client.

## Decision

- **One correlation point.** The confirmation bridge
  (`dali2rust-api::confirmation_bridge`) is the only place where an HTTP request is
  matched to its `ConfirmationEnvelope`: a fixed pool of pending slots, fed by one
  bridge thread that drains the confirmations channel.
- **Bounded slots.** The pool size is `DEFAULT_CONFIRMATION_SLOTS` (24), the same
  constant `BusConfig` uses, so composition and defaults cannot disagree.
- **One allocator.** Correlation ids come from one shared allocator per composed
  router.
- **Deterministic failure.** No free slot, a full command ingress, or a synthetic
  `DeliveryRejected` confirmation ([ADR-006](ADR-006-routed-command-delivery.md))
  answers `503` at once. A confirmation that does not arrive within
  `confirmation_timeout_ms` (2000 ms by default) answers `504`. A timeout releases the
  slot, not the command, so the command may still execute afterwards.
- **Several frames and long work** follow
  [ADR-012](ADR-012-async-chunked-config-writes.md): one deadline per request, and `202`
  with an operation id for work that can outlast it; for those routes the operation
  tracker, not the bridge, hears a rejection.

## Consequences

- Confirmation semantics and HTTP status codes are identical across handlers; a
  synthetic rejection behaves like any other confirmation at the bridge boundary.
- The confirmation path stays typed end to end inside the runtime; JSON appears only
  when the reply formatter renders the HTTP body.
- The full status-code mapping and the reply formatters are described in
  [04-contracts-and-api-bridge.md](../04-contracts-and-api-bridge.md).
