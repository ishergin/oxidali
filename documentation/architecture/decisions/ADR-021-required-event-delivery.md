# ADR-021: An event that is the sole carrier of a fact is delivery-required

Status: Accepted
Date: 2026-08-18

## Context

[ADR-015](ADR-015-routed-event-delivery.md) makes events best-effort on the premise that
nobody waits on an event. The operation tracker does: a tracked operation such as a
discovery run ends only when an `OperationWorkerSignalEvent` arrives, and when that frame
is dropped at a full events ingress the operation sits `running` until its TTL and then
reports `timed_out` for work that finished long before. The bus task drains ingress only
between its own waits, so a producer that publishes a burst in microseconds meets a queue
that will not be drained for milliseconds; the frames at the end of the burst — the
outcome among them — are the ones lost. Raising the queue depth only moves that cliff.

## Decision

1. **Two classes of publisher, split by what the event carries, not by how important it
   feels.** An event that is the sole carrier of a fact a client can observe and nothing
   will re-send — a registry record, a runtime commit, an operation's outcome — is
   published on a required-delivery path. An event that notifies observers of state they
   can re-read (`PhysicalDeviceChangedEvent`, `RuntimeStateChangedEvent`, every registry
   changed-event) stays best-effort; its consumer's fallback is a read.
2. **The mechanism is a bounded backoff on the one path, never a second path.**
   `dali2rust_bus::publish_required` retries `DroppedIngressFull` on a schedule and reports
   the outcome and the time it slept. Worker threads use `REQUIRED_PUBLISH_BACKOFF_MS`
   (0, 25, 50, 100, 150, 200, then 250 × 4 — 1 525 ms in total); the single httpd task
   uses `HANDLER_PUBLISH_BACKOFF_MS` (0, 15 — one drain interval), because a blocked
   millisecond there is downtime for every socket.
3. **The sleep belongs to a unit of work, not to a frame.** `publish_required` takes a
   `budget_ms` that truncates the schedule (never rescales it — a shorter step would
   retry before the bus task drains). The DALI worker arms one budget per command from the
   command's `WirePriority` ([ADR-013](ADR-013-wire-priority-and-yield-granularity.md)),
   so a new command kind inherits one: `Interactive` 400 ms (the applied fact is published
   before the confirmation, inside a 2 000 ms deadline), `Attended` 3 000 ms for the whole
   command, `Unattended` one schedule. Publishers that own their thread and hold no
   deadline pass `REQUIRED_PUBLISH_UNCAPPED`.
4. **A series shares the budget; only the closing frame is exempt.**
   `required_publish::Kind::Series` (attribute chunks, memory-bank chunks, discovery
   progress, and anything else published mid-command) draws down the shared pool.
   `Kind::Singleton` is by definition the frame that closes the unit of work — at most one
   per command, enforced by a `debug_assert` — and gets the command's full allowance, so
   a long series can never spend the budget of the terminal signal.
5. **The boundary is a burst.** A bounded backoff survives a producer that fills the
   ingress and stops; it does not survive a bus saturated for longer than the schedule,
   and must not — that would be an unbounded queue blocking a worker thread.
6. **The declaration is a fence.** Each publisher exports `<PUBLISHER>_REQUIRED_EVENTS`,
   and the ownership test fails until every kind the operation tracker consumes is
   declared required by someone. An `OBSERVED_ONLY_EVENTS` kind may never be declared
   required: it routes to nobody, and a backoff would hold a queue slot for a frame with
   no destination.
7. **No producer publishes an unbounded burst** ([ADR-007](ADR-007-apply-orchestrator.md)),
   the discovery scan included: it publishes each device as the wire describes it and
   returns a count, not a collection, so the burst cannot be rebuilt by writing the
   obvious loop ([03](../03-bus-and-backpressure.md) §Producers).

### Rejected alternatives

- **A deeper events ingress** — moves the cliff; every new producer reopens the sizing.
- **A second route for the outcome** (the tracker also consuming a completion event) —
  two mechanisms for one effect hide which one works.
- **Outcomes on the confirmations channel** — that channel serves a caller that waits.
- **A blocking publish** — couples wire progress to bus drain latency.
- **A tracker liveness timeout shorter than the TTL** — a faster wrong answer for a run
  that is genuinely still going.

## Consequences

- A required publish costs latency only when the ingress is actually full, and only
  within the owning command's budget.
- In the DALI worker, `event_publish_retried` counts backoffs spent (the guarantee
  exercised), `event_publish_failed` counts exhaustion (a saturated bus, a different
  investigation) and `event_publish_backoff_ms` counts the time the budget bounds. Other
  required publishers keep their own retry and failure counters, beside the worker they
  describe, since a retry count is only readable there.
- On a realistic workload the backoff is rarely reached, because producers do not
  burst; `dali2rust-bus/tests/publish_required.rs` exercises it against a deliberately
  filled ingress.
