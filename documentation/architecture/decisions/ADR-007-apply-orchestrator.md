# ADR-007: Apply orchestrator and the mass-command policy

Status: Accepted, amended by ADR-012
Date: 2026-07-06

## Context

A bulk apply — writing the desired group-membership matrix, a scene, or a gear policy to
every affected device — expands into one DALI command per changed cell, up to 64 lamps
× 16 groups. Expanding it in the HTTP handler and publishing the expansion as one burst
couples the request to every queue depth on its path, fails half-way with commands
already executing, splits the declared outcome count from the component that produces
the outcomes, and holds the single httpd task for as long as the diff is large.

## Decision

A dedicated **apply orchestrator** worker (`dali2rust-operations-runtime`, its own task)
executes every bulk apply.

- **HTTP publishes one command and answers `202`.** `GroupApplyExecuteCommand`,
  `SceneApplyExecuteCommand` and `PolicyApplyExecuteCommand` are a few dozen bytes
  whatever the diff size. The handlers keep only request validation: the group and
  scene routes refuse with `409` while an apply of the same kind is running, and the
  group route answers an empty diff at once (`200` with the matrix, from a
  non-authoritative peek).
- **The orchestrator owns the diff.** It expands the desired-versus-applied diff from its
  own registry snapshot, publishes `OperationBeginCommand` with the authoritative
  expected-outcome count, and paces execution through one shared pacing core so that the
  DALI wire clocks the pipeline: at most one apply command is in flight, and cells that
  need no wire time are skipped in small batches. A missing outcome is re-published once
  (program commands and their readbacks are idempotent) and then replaced by a synthetic
  failure, and a terminal worker signal follows the last cell, so the tracker's count
  always converges. The run aborts by polling the tracker's own state, not by hearing a
  status event: a best-effort event could be dropped exactly when the abort matters. The
  pacing parameters are in the
  [module document](../../product-design/runtime-modules/apply-orchestrator/README.md).
- **Boundaries.** The registry never computes a diff or publishes a DALI command, and
  the operation tracker runs no business logic. The events the orchestrator
  consumes are matched inline, so its `APPLY_ORCHESTRATOR_HANDLED_EVENTS` const is kept by
  hand, one entry per paced family.

### Mass-command policy

**No producer may publish an unbounded burst onto the bus.** Expanding a bulk request
belongs to the worker that executes it, behind flow control: self-clocking on result
events, bounded batches, or consumer-side coalescing.

| Producer | Mechanism |
| --- | --- |
| Group, scene and policy apply | this orchestrator and its pacing core |
| HCL scheduler | coalesces its desired set per tick, caps it, and self-clocks on each command's confirmation |
| Poller | one read in flight, wire-time budget ([ADR-009](ADR-009-background-wire-time-budget.md)) |
| State fan-out | coalescing per tick; a group or broadcast expansion paces itself in batches of eight under one shared retry budget |
| Chunked config writes | a series of fixed maximum length, committed by a closing bracket ([ADR-012](ADR-012-async-chunked-config-writes.md)) |
| Discovery | each device published as the wire describes it ([ADR-021](ADR-021-required-event-delivery.md)) |
| Cluster / adapter proxy (not built) | per-adapter pacing and loop prevention |

### Rejected alternatives

- **Expansion inside the DALI worker** — a full-matrix apply would hold the adapter for
  seconds and starve interactive commands; apply cells are independent and must
  interleave with operator traffic.
- **A burst plus flow control in the handler** — holds the httpd task for the whole
  apply and keeps the half-applied failure mode.
- **Raising queue depths** — linear RAM for a structural problem; every new producer
  reopens the sizing question.

## Consequences

- Apply size is independent of every queue depth; a full 64 × 16 matrix applies in one
  `POST` with default queue sizes.
- One extra task and two small subscriber inboxes.
- A large apply's latency is set by the DALI wire for bound cells and by the bus task's
  drain interval for skip batches.
