# ADR-015: Routed event delivery

Status: Accepted, amended by ADR-021
Date: 2026-08-07

## Context

Every event consumer already declares what it handles: the `dispatch_bus_events!` tables
generate `<WORKER>_HANDLED_EVENTS` consts ([ADR-005](ADR-005-declarative-contract-macro.md)),
and the ownership test checks that each event kind has a consumer or a documented
observed-only entry. Broadcasting every event to every subscriber ignores that
declaration: each inbox receives a clone of every frame, a worker that consumes one rare
kind wakes for all of them, and its overflow counter fills with frames it never wanted.
The worst-hit inboxes are the ones an operator sees — the WebSocket fan-out and the OLED
display.

## Decision

1. **Events route by declared kind set, exactly as commands do**
   ([ADR-006](ADR-006-routed-command-delivery.md)). `subscribe_events`,
   `subscribe_commands_and_events` and their named forms take the worker's
   `<WORKER>_HANDLED_EVENTS` const. One route table implementation serves both channels,
   built at spawn; an unknown name panics at spawn. Test observer taps declare
   `EVENT_VARIANT_NAMES` and receive everything.
2. **An event nobody declared is dropped silently, and that is legal.** There is no
   rejection and no `events_unrouted` counter: every observed-only kind routes to nobody
   by design, so such a counter would count normal operation. Totality — every kind
   consumed or documented as observed-only — stays the ownership test's job. An event
   that is the sole carrier of a fact someone waits for is a different matter and is
   published on a required-delivery path
   ([ADR-021](ADR-021-required-event-delivery.md)).
3. **Every declaring subscriber receives the event** — broadcast within the declaring
   set. Several workers declare `RuntimeStateChangedEvent`, and each of them gets it.
4. **The WebSocket and MQTT workers are declared consumers.** `WS_PROJECTED_EVENTS` sits
   beside the projection match in `dali2rust-api` and is held to it by tests. The MQTT
   bridge declares the kinds it consumes: runtime state, input events, and the registry
   changed-events that re-announce discovery configs. `HomeAssistantSettingsChangedEvent`
   stays undeclared on purpose: the bridge re-reads its settings from the read port, so
   an event dropped while it is parked cannot park it for ever.
5. **`OBSERVED_ONLY_EVENTS` means** published, counted at ingress, delivered to nobody.
6. **Confirmations stay broadcast**, and so does the synthetic `DeliveryRejected`
   fan-out to confirmation subscribers.

### Rejected alternatives

- **Consumer-side filtering** — the clone, the wake and the drain are paid before the
  consumer's `match` can decline.
- **An `events_unrouted` counter** — counts normal operation.
- **Routing by (kind, `target_adapter_id`)** — `target_adapter_id` stays a consumer-side
  filter, and some consumers must not filter by adapter at all.

## Consequences

- Per-subscriber `delivered` totals count only declared kinds, and `receiver_overflow`
  (and with it the WebSocket drop notice) reports only drops of frames the subscriber
  could have wanted.
- A subscriber that has nothing to do is not woken: the display worker declares a
  handful of rare kinds and nothing else.
- A consumed-but-undeclared kind is silent starvation, the event analogue of an unrouted
  command with no rejection to report it. The fence is that subscriptions and ownership
  rows use the same consts; the residual exposure is a consumer that matches events
  inline and keeps its const by hand, which needs its own test (the apply orchestrator
  has one per paced family).
- Subscribers carry names on the diagnostics surface
  ([ADR-023](ADR-023-named-subscribers-and-shared-coalescing.md)).
