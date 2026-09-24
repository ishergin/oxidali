# ADR-012: Chunked config writes are staged, bracketed and answered 202

Status: Accepted
Date: 2026-08-03

## Context

`esp_http_server` runs one task that serves every socket inline, so a handler that waits
is downtime for every client, including the web UI's poll in every open tab. A payload
larger than `MAX_BUS_WIRE_BYTES` (128) — a scene matrix, a group-membership matrix, an
HCL schedule, the rules document — has to travel as a series of chunks. Publishing and
confirming each chunk with its own timeout holds the task for N times the confirmation
budget, and when such a write answers asynchronously there is no waiting caller left to
report a half-written matrix to.

## Decision

1. **A handler that publishes more than one frame shares one deadline.** A fresh
   `confirmation_timeout_ms` per frame is forbidden
   (`publish_batch_and_wait_for_success` is the shape to copy when a handler must wait
   at all).
2. **Chunks stage; a closing bracket applies the series as one unit.** The owner stages
   every chunk, and a closing bracket commits the series — `ConfigWriteCommitCommand` for
   the scene and group matrices, `RuleCommitCommand` for the rules document: one write
   lock, one revision bump, one changed event — or nothing. An HCL schedule has no
   bracket (below).
3. **The routes answer `202` + operation id** under `OperationType::ConfigWrite` and wait
   for nothing. The same answering discipline covers `write-attributes`, which reaches
   the wire and can outlast any confirmation budget.

Supporting rules:

- **The bracket is its own command**, not two fields on the row payloads: the
  scene-matrix chunk's worst case is 127 of 128 bytes, and one bracket concept must not
  exist in two protocols.
- **The bracket carries the chunk count.** The registry cannot see a frame that never
  arrived, only that fewer chunks staged than the writer declared; without the count a
  lost frame commits a short matrix silently.
- **One correlation id per bracketed write.** Every frame of a matrix or rules-document
  write — each chunk and the bracket — carries the same correlation id, which is also
  the operation's workflow id. The stage is keyed by resource and records which series
  filled it: a chunk from a newer series drops an older stage instead of appending to it,
  and a bracket closes only its own series. Two concurrent writes to one resource are
  last-writer-wins, and the older bracket is refused — a refusal, never a torn commit.
- **HCL schedules have no bracket and no shared id.** Each chunk carries its own
  correlation id; the chunk marked last is the commit, its id is the operation's
  workflow id, and only it signals the operation, so a middle chunk's confirmation never
  completes it. The stage is keyed by `schedule_id`: a chunk that opens a sequence
  replaces it, and a refused chunk drops it, so the commit then fails. A staged schedule
  id counts as taken, so `409 conflict` on create means "exists or is staging".
- **`ConfigWrite` is exempt from operation coalescing**, because it names a resource,
  not one activity per adapter.
- **Bounded staging.** At most two config-write stages exist at once, a stage older than
  15 s is reaped on the registry worker's idle turn, and a `ConfigWrite` operation's TTL
  is 10 s rather than the general ten-minute TTL.

### Rejected alternatives

- **Pacing the chunks through the apply orchestrator** — it paces wire time and counts
  one loopback outcome per cell; a registry write has neither, so it would have to
  invent outcomes.
- **Inline first/last markers on the row payloads** — they do not fit the scene chunk.
- **A `409` gate while an apply runs** — the UI's own write-then-apply sequence would
  race it; last-writer-wins is the existing semantic.
- **One operation type per resource** — the caller knows what it wrote, and the
  operation id names the resource.

## Consequences

- Clients poll the operation and must check that it committed before acting on the
  write (the web UI's `opCommitted`; `tools/hil` waits inside its config-write helper).
  A resolved `202` is not a landed write.
- A refused series (`config_write_chunk_count_mismatch`, `config_write_no_stage`,
  `config_write_stage_limit`) fails the operation and leaves the stored configuration
  untouched.
- The confirmations ingress (32) and the pending-slot pool (24) are sized for the
  largest series one handler publishes before any of it is collected.
- A rejected publish on these routes is heard by the operation tracker
  ([ADR-006](ADR-006-routed-command-delivery.md)).
