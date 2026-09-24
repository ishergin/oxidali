# Architecture

How dali2rust is built, as it is today, and the rules the code must keep. When a
document here disagrees with the code, the code wins and the document is corrected.

Scope: as-built mechanisms and invariants. Why a decision was taken →
[`decisions/`](decisions/README.md); what the product does →
[`../product-design/`](../product-design/README.md).

## Reading order

1. [01 Overview](01-overview.md) — crates, composition root, composed runtimes, crate boundaries.
2. [02 Runtime and threading](02-runtime-and-threading.md) — tasks, waiting, the single HTTP task, logging.
3. [03 Bus and backpressure](03-bus-and-backpressure.md) — channels, capacities, overload, required events, coalescing.
4. [04 Contracts and API bridge](04-contracts-and-api-bridge.md) — payloads, codec, JSON boundaries, counters, confirmations.
5. [05 Testing and BDD](05-testing-and-bdd.md) — test layers, the black-box boundary, conventions.
6. [06 Registry and persistence](06-registry-and-persistence.md) — single writer, runtime merge rules, slice store.
7. [07 Memory and cores](07-memory-and-cores.md) — internal SRAM versus PSRAM, stacks, core placement.
8. [08 DALI PHY and transport](08-dali-phy-and-transport.md) — the interrupt, its crate, transmit and answer paths, flash.
9. [09 DALI protocol rules](09-dali-protocol-rules.md) — the IEC 62386 rules the executor follows.
10. [10 Build, release and tooling](10-build-release-and-tooling.md) — builds, versions, knobs, partitions, merge gates.
11. [11 Extension recipes](11-extension-recipes.md) — checklists for extending the system.

[Decisions](decisions/README.md) — one ADR per locked decision.
