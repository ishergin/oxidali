# 03 Bus and Backpressure

How typed frames move between workers: the three channels and how they route, the
current capacities, what a publisher sees under overload, and the delivery disciplines
built on top — required events, listeners for refusals, bounded producers, coalescing.

Scope: the in-memory bus (`dali2rust-bus`) and the rules every publisher and subscriber
follows. Payload declaration and the frame budget → [04](04-contracts-and-api-bridge.md);
how workers wait → [02](02-runtime-and-threading.md); DALI worker priorities →
[09](09-dali-protocol-rules.md).

## Channels

| Channel | Delivery | Subscription |
| --- | --- | --- |
| Commands | Routed by payload kind to the subscribers that declared it; unicast to the owner in production ([ADR-006](decisions/ADR-006-routed-command-delivery.md)) | `subscribe_commands(capacity, <WORKER>_HANDLED_COMMANDS)` |
| Events | Routed by declared kind; a kind nobody declared is dropped silently — the observed-only surface ([ADR-015](decisions/ADR-015-routed-event-delivery.md)) | `subscribe_events(capacity, <WORKER>_HANDLED_EVENTS)` |
| Confirmations | Broadcast | `subscribe_confirmations(capacity)` |

- A funnel (`subscribe_commands_and_events`) delivers both routed channels into one
  inbox in arrival order and declares both sets.
- The declared sets are the consts the dispatch tables generate
  ([04](04-contracts-and-api-bridge.md)). `bus_payload_ownership.rs` checks totality:
  every command has exactly one owner, every event has a consumer or an
  `OBSERVED_ONLY_EVENTS` entry.
- `target_adapter_id` is an adapter-instance filter applied by the consumer. `sender_id`
  and `Origin` are metadata; neither routes.
- Event subscribers carry a name to the diagnostics surface, so an overflow names its
  victim ([ADR-023](decisions/ADR-023-named-subscribers-and-shared-coalescing.md)).
- A `CommandArrivalObserver` on the publisher sees every command that passes the kind
  and size checks, on the publishing thread, from envelope metadata only, before it is
  queued: a command then dropped at a full ingress has still been announced, which costs
  at most one spurious yield. It exists because the DALI worker cannot see a command it
  has not dequeued; the operator-arrival signal ([09](09-dali-protocol-rules.md)) is
  raised there.
- The backend is `std_mpsc` on host and ESP-IDF `std` builds, behind the generic
  `Sender<T>` / `Receiver<T>` traits of `dali2rust-bus::channels`.

## Publishing

- Ingress never blocks. A publish returns `Queued`, `DroppedIngressFull` or
  `RejectedFrameTooLarge`; an oversize frame is refused before it is queued, with no
  heap fallback.
- The routing task drains ingress into subscriber inboxes. A stuck subscriber overflows
  only its own inbox and never stalls the bus.
- Order holds within a channel, not across channels: the routing task drains each
  ingress separately, so a command and an event that one thread published in that order
  can reach a funnel inbox in the opposite order.

## Capacities

Ingress and slot defaults are `BusConfig::default()`; inbox depths are set where the
composition subscribes (`dali2rust-adapters/src/runtime/bus_host.rs`).

| Queue | Depth |
| --- | --- |
| Commands / events / confirmations ingress | 128 / 64 / 32 |
| Pending confirmation slots (HTTP bridge) | 24 |
| DALI worker commands | 128 |
| Registry funnel | 64 |
| Operation tracker: commands / events / confirmations | 16 / 64 / 32 |
| Apply orchestrator: commands / events | 4 / 64 |
| Confirmation bridge | 32 |
| State-fanout projector events | 64 |
| Display events | 32 |
| HCL scheduler: funnel / confirmations | 64 / 32 |
| Poller: events / confirmations | 64 / 32 |
| Home Assistant bridge funnel | 64 |
| WebSocket fan-out events | 128 |
| Rules funnel | 128 |
| OTA commands | 2 |
| Redundancy: arbitration / replication / supervisor events | 16 / 8 / 4 |

- The two 128-deep event inboxes belong to the slowest consumers (per-event JSON
  projection, per-event rule evaluation), where a loss is visible to an operator.
- No depth is sized for a producer's burst. Raising a depth moves the cliff instead of
  removing it; flow control belongs to the producer (*Producers*, below).
- A queue slot holds an `Arc`, so depth costs slot memory, not envelope memory. A
  byte-copying queue (a FreeRTOS queue) cannot carry these frames, which is why the backend
  is `std::sync::mpsc` on the device as on the host.

## Overload and rejection

- **Commands:** when no declared subscriber accepts a command — the kind is unrouted
  (`commands_unrouted`) or every owning inbox is full (`receiver_overflow`) — the bus
  injects a synthetic `DeliveryRejected` confirmation. It is fanned directly into the
  confirmation subscribers' inboxes, not through the confirmations ingress, so a burst
  of rejections cannot be lost there; it is dropped only when a subscriber inbox is
  itself full (`delivery_rejected_dropped`). It may overtake real confirmations queued
  earlier, which is safe: a rejected command never produces one. If at least one
  declared subscriber accepted, a sibling's overflow is only counted.
- **Events and confirmations:** a full inbox drops the delivery to that subscriber only.
- The HTTP bridge turns a rejection into `503` and a missing confirmation into `504`
  ([04](04-contracts-and-api-bridge.md)).

## A refusal needs a listener

- A route that answers `202` and an operation id waits for nothing, so the operation
  tracker holds its own confirmations inbox and fails the operation with
  `CommandsIngressOverload` when its command is rejected. A new `202` route inherits
  that; a route that invents a third answering discipline owes itself a listener.
- The DALI worker's gates (adapter `enabled: false`, a passive controller) refuse before
  the wire and deliver the refusal by the route's own discipline: an operation failure
  signal for a `202` route, a confirmation for a request-scoped one. The refusal names
  its cause (`adapter_disabled`, `controller_passive`), never the generic
  `execution_failed` an unanswering gear produces. Every kind the DALI worker handles
  drives some adapter's wire, so the adapter switch gates every one of them.

## Required events

[ADR-021](decisions/ADR-021-required-event-delivery.md).

- An event whose consumer can re-read the state it describes is best-effort. An event
  that is the only carrier of something that must survive — a registry record, a
  runtime commit, an operation's outcome — goes through
  `dali2rust_bus::publish_required`: the same ingress, retried on a bounded backoff
  (`REQUIRED_PUBLISH_BACKOFF_MS` on worker threads, the shorter
  `HANDLER_PUBLISH_BACKOFF_MS` on the httpd task).
- Each publisher declares `<PUBLISHER>_REQUIRED_EVENTS`; `bus_payload_ownership.rs`
  fails until everything the operation tracker consumes is required by some publisher.
  The tracker is the consumer that waits: an operation ends only on
  `OperationWorkerSignalEvent`.
- The backoff survives a burst, not a saturated bus. That boundary is the design; the
  schedule is not lengthened to chase it.
- The sleep belongs to a unit of work. `publish_required` takes a `budget_ms` and
  reports `slept_ms`; the DALI worker arms one budget per command from the same wire
  priority as its wire lease. Every mid-command publish is a `Series` and draws that
  shared pool; `Singleton` is only the frame that closes the unit — at most one per
  command, enforced by a debug assertion. Publishers that own their thread and hold no
  deadline pass `REQUIRED_PUBLISH_UNCAPPED`.

## Producers

- No producer publishes an unbounded burst
  ([ADR-007](decisions/ADR-007-apply-orchestrator.md)): bulk expansion belongs to the
  owning worker behind flow control. The apply orchestrator paces cell commands; the
  discovery scan hands each device to a sink as the wire describes it and returns a
  count, so publishes are an identity probe apart.
- During anyone's burst every other producer's events are collateral: a dropped applied
  fact fails an operator's request.

## Subscribers

- A subscriber is woken only for the kinds it declares. Declare facts, not numbers: a
  number floods the inbox.
- Read a setting from the registry; never learn it only from its changed-event, because
  hydration and a slice reload (import, replication) publish none. A worker that caches a
  setting refreshes it after a slice reload as it does after a write.
- A subscriber with nothing to do must not wake. The WebSocket worker parks on its hub
  while no client is connected; an unused surface costs one atomic load.

## Coalescing

- The DALI worker coalesces its inbox per target: a later command for the same target
  displaces an earlier one that has not started (the keys:
  [DALI worker](../product-design/runtime-modules/dali-worker/README.md)).
- A displaced target-state setpoint of the survivor's origin gives the survivor every
  field the survivor does not state (`LightSetpoint::merge_from`). Nothing of another
  origin is folded — the survivor's envelope carries the origin, which sets its priority
  and the runtime source of its commit — and nothing is folded across a lamp-driving
  frame that reaches the wire in between.
- A displaced command whose fields were carried is confirmed as success; one that
  contributed nothing is answered `superseded` (409). Coalescing may drop a frame, but a
  field the survivor does not state is never dropped silently.
- The worker never reorders two lamp-driving frames, so publish order is the order the
  lamps end in. A producer that means two verbs to land together merges them before
  publishing, as the rules engine does within one activation
  ([rules engine](../product-design/runtime-modules/rules-engine/README.md)).
