# 11 Extension recipes

Checklists for the common ways the system grows. Each step says where the change goes;
the rules behind the steps live in the documents it links.

Scope: procedure only. Crate and trait placement → [01](01-overview.md); test layers and
BDD conventions → [05](05-testing-and-bdd.md); DALI rules → [09](09-dali-protocol-rules.md).

## A control-gear command family (16-bit, a Part 2xx device type)

1. `dali2rust-domain/src/dali/devices/<module>.rs`: a `DtXCommand` enum with `opcode`,
   `from_opcode`, `is_query` and `to_forward_frame` (indirect address byte, bit 0 set),
   and `DeviceCommandMetadata` (device type, opcode, `expects_backward`,
   `requires_repeat`). `requires_repeat` comes from the part's configuration opcode range
   (`opcode_requires_repeat`), never per variant. Register the module in `devices/mod.rs`.
2. `pres/extended.rs`: an `ExtendedCommand::DtX(DtXCommand)` variant delegating
   `from_opcode`, `to_forward_frame`, `is_query`, `requires_repeat` and
   `enable_device_type`.
3. `pres/codec.rs`: `try_decode_extended` maps the opcodes, routing by the live
   `ENABLE DEVICE TYPE` prelude where opcodes collide with another part; a query also
   joins `QUERY_OPCODES` in `pres/opcode.rs`.
4. Nothing to wire for the prelude: the controller sends `ENABLE DEVICE TYPE` inside the
   same unit before every extended command.
5. A product caller is a semantic command (below), never the raw diagnostic route.
6. Teach `dali2rust-gear-model` the commands if the host sim or a scenario must answer
   them.
7. A scenario proves the claimed profile, not only success: the indirect address byte,
   the opcode, and the transport-visible order (`ENABLE DEVICE TYPE` first, a send-twice
   pair adjacent).

## A Part 103 (24-bit) command

1. The frame lives in `dali2rust-domain::dali::dev103` (`Device103Command` or an
   instance command, `ForwardFrame24`, typed addresses and operands).
2. It executes in `DaliWorker` (`dali_worker/dev103.rs`, `executor/dev103*.rs`) through
   `DaliTransport::exchange_frame24`; never as `wire_address` + `command`.
3. Operands use the typed forms (`ShortAddressOperand`, `InitialiseScope103`), a DTR0
   operand is proved by an addressed read-back, and every read-back goes through
   `verify_readback`.
4. The command kind gets its `WIRE_CLASSES` row, like any worker command.

## A bus command or event

([ADR-005](decisions/ADR-005-declarative-contract-macro.md))

1. Append the payload at the **end** of `declare_bus_payloads!` in
   `dali2rust-contracts/src/msg/commands.rs` or `events.rs`, with a worst-case
   `budget = …;` (complex samples in `msg/payload_test_samples.rs`). `, max = N` needs a
   stated justification. The variant order is the `postcard` discriminant.
2. `cargo test -p dali2rust-contracts` fails on `FROZEN_COMMAND_ORDER` /
   `FROZEN_EVENT_ORDER`: append the name there; never reorder.
3. `cargo test -p dali2rust-adapters --test bus_payload_ownership` fails until a command
   has exactly one owner — one arm in that worker's `dispatch_bus_commands!` table, whose
   `<WORKER>_HANDLED_COMMANDS` follows — and an event has a consumer arm or an
   `OBSERVED_ONLY_EVENTS` entry with its reason.
4. An event that is the sole carrier of a fact is published with `publish_required` and
   declared in its publisher's `<PUBLISHER>_REQUIRED_EVENTS`
   ([03](03-bus-and-backpressure.md)).
5. A command the DALI worker executes needs its `WIRE_CLASSES` row
   (`dali-runtime/src/runtime/priority.rs`); a test fails without it.
6. Publish with `command_envelope(sender, correlation, adapter, origin, Payload { … })`
   or `event_envelope`; no per-payload builder.
7. A scenario, and the family's row in `product-design/bus-contracts/commands.md` or
   `events.md`.

## A worker or subscriber

1. The worker and its spawn function live in the owning runtime crate
   (`crates/dali2rust-<area>-runtime/src/runtime/<name>_worker.rs`), spawned with
   `dali2rust_bsp::esp_thread::spawn_named_stack` (or `try_spawn_named_stack` for
   anything a client can trigger) and a `std_thread_stack` class. The primary inbox is
   read with `recv` or `recv_timeout` ([02](02-runtime-and-threading.md)).
2. Subscribe in `service_channels()` (`adapters/src/runtime/bus_host.rs`) with the
   declared kind set — `subscribe_commands(capacity, <WORKER>_HANDLED_COMMANDS)`,
   `subscribe_events_named(capacity, <WORKER>_HANDLED_EVENTS, "<name>")`, the name being
   what the overflow counters report
   ([ADR-023](decisions/ADR-023-named-subscribers-and-shared-coalescing.md)) — and add
   the receiver to `ServiceChannels` (`workers.rs`).
3. Spawn it from `spawn_service_workers` (`workers.rs`); the compiler then leads through
   `SpawnedWorkers` and `collect_spawned`, and for counters through
   `RuntimeCounterHandles`, the DTO and the counter surface
   ([04](04-contracts-and-api-bridge.md)). External dependencies arrive through
   `WorkerSpawnInputs`.
4. Its stack is measured on the bench and held by `tools/hil/stack_budget.txt`
   ([07](07-memory-and-cores.md)).
5. Composed behaviour is a scenario; counters or internal bus access are a crate
   integration test; bus-runtime behaviour is a test in `crates/dali2rust-bus/tests/`.

## An HTTP endpoint

1. A handler in `dali2rust-api/src/http/handlers/<name>.rs` implementing `ApiHandler`
   (`Send + Sync`). `handle_request` stays within 30 lines as validate → build →
   publish → respond. It publishes on the bus; it never calls the transport or mutates
   the registry, and reads go through read ports ([06](06-registry-and-persistence.md)).
2. Choose the answering discipline. A short wait uses the dispatcher helpers
   (`dispatch_and_wait_for_success`; `publish_batch_and_wait_for_success` for several
   frames under one deadline): `400` invalid input, `503` slots or ingress full, `504`
   timeout. Anything longer answers `202` + operation id through
   `handlers/operation_dispatch.rs`, with the operation tracker as its listener
   ([03](03-bus-and-backpressure.md)).
3. Add a `RouteKey` variant and its `ROUTE_TABLE` row (method, path) in `http/app.rs`,
   and update the row count the table test pins.
4. Register it in `dali2rust-adapters/src/runtime/app_router.rs` with
   `.with_handler(RouteKey::…, …)`; `AppBuilder::build` panics on a handler without a row.
5. The response is a resource sized for the HTTP task's stack
   ([07](07-memory-and-cores.md)); JSON is built only in `dali2rust-api`
   ([04](04-contracts-and-api-bridge.md)).
6. A scenario under the resource's feature directory.
7. The route and every error code it can answer are named in the resource's document
   under `documentation/product-design/rest-api/`; `verify_rest_docs.py` fails a route
   or a code that no document names.

## A platform backend

1. The trait goes in `dali2rust-platform` with `type Error: core::fmt::Debug` (no
   `std::error::Error`, so it stays `no_std`-friendly) and no ESP-IDF type.
2. The ESP implementation sits behind `#[cfg(target_os = "espidf")]` beside the slice
   that owns it (`adapters/src/dali/transport/esp_idf.rs`,
   `display-runtime/src/display/esp_idf.rs`). Anything that runs in the PHY interrupt
   belongs in `dali2rust-dali-phy` ([08](08-dali-phy-and-transport.md)).
3. A host implementation for tests (`MockDaliTransport`, `HardwareDisplay::None`).
4. Wiring: `dali2rust-firmware/src/composition/esp_idf.rs` for hardware, `host.rs` for
   the host build, and the BDD harness; composition stays generic over the trait
   (`build_router_with_bus_and_transport<T: DaliTransport>`).
5. Proof through the mock at the port boundary.

## Smaller changes

- **A counter**: an `AtomicU32` in the runtime, then DTO, mapping, worst-case sample and
  the web UI's type mirror; it goes to `/api/v1/stats` unless the periodic diagnostics
  frame is argued for ([04](04-contracts-and-api-bridge.md)).
- **A persisted field**: bump only its own slice's version and say what the bump resets;
  a physical-device slice bump costs every virtual-lamp binding
  ([06](06-registry-and-persistence.md)).
- **A build-time knob**: an `option_env!` read in the firmware crate; add its row to
  [10](10-build-release-and-tooling.md).
- **A UI change**: the design card first, pushed ([10](10-build-release-and-tooling.md)).
