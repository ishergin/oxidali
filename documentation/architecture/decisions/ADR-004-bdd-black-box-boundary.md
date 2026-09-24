# ADR-004: BDD is black-box

Status: Accepted
Date: 2026-06-04

## Context

BDD scenarios prove user-visible behaviour of the composed host stack. When a scenario
asserts harness internals — registry records, counters, bus slots — it stops proving
what a client sees, it breaks on every refactor, and it invites production code to grow
hooks that exist only for tests. Those hooks then become a second, untested way to
reach state that the real path should produce.

## Decision

- **Drive through the product entry points only.** A scenario starts the stack through
  the same composition entry points as the host and firmware builds, with the mock DALI
  transport and the mock MQTT client at the port boundaries, and drives it over HTTP —
  or over the WebSocket or MQTT when that surface is the feature under test.
- **Assert only externally visible evidence:** HTTP responses and their DTOs, frames
  recorded by `MockDaliTransport`, WebSocket and MQTT messages, and file-system
  effects.
- **No harness internals.** Scenarios never call into `BusStackRuntime`, the registry
  store, counters or slot internals, and never seed state by writing records directly.
  State is created the way a client creates it.
- **No production test hooks.** Production crates carry no `bdd` feature, no `bdd_*`
  functions and no registry-seeding back doors.
- **Build requests with production helpers.** Payloads come from the contracts and API
  crates, never from hand-rolled bytes.
- **Lower-level evidence lives lower.** A behaviour that needs counters, direct bus
  publishing or registry access is proved by a crate integration test in the owning
  crate, which seeds through the production path.

## Consequences

- The suite is slower than unit tests but exercises the same paths a client does, and
  it survives internal refactors.
- `verify_bdd_layers.sh` and `verify_no_bdd_production_hooks.sh` enforce the boundary
  as merge gates.
- Feature layout, tags, step conventions and the executable/design mapping are
  described in [05-testing-and-bdd.md](../05-testing-and-bdd.md).
