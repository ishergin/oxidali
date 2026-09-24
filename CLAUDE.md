# CLAUDE.md

Rules for AI agents and contributors in **dali2rust**, the Rust firmware of a DALI-2
lighting controller on the ESP32-P4. Every line is a rule, a command or a pointer; the
detail lives in the linked document.

**Authority.** This is the canonical rules file for agents. [`AGENTS.md`](AGENTS.md) only
points here. When a document and the code disagree, the code
wins and the document is fixed. Documentation map:
[`documentation/README.md`](documentation/README.md); `ADR-NNN` is a record in
[`decisions/`](documentation/architecture/decisions/README.md).

## Golden rules

1. **BDD-first.** New behaviour starts with a failing scenario and merges when the new
   and the existing scenarios pass; infrastructure-only changes (refactors, docs, CI)
   are exempt.
2. **Bus-only delivery.** HTTP, MQTT, WebSocket, HCL, rules, cluster and proxy code
   publish typed commands on the bus; they never call `DaliTransport` and never mutate
   the registry.
3. **Function size (STRICT).** Over 20 lines needs a justification in the PR; over 40 is
   forbidden — decompose. `handle_request` stays within 30 lines as validate → build →
   publish → respond; a `match` with more than 10 arms is extracted.
   `scripts/verify_fn_length.sh` (host crates) and `scripts/verify_fn_length_esp.py`
   (ESP-only code, found by content) hold baselines of 0. The crate list lives only in
   `scripts/host_crates.txt`; no gate keeps its own.
4. **Lowest test layer** that proves the behaviour without bypassing the production path.
5. **English** is canonical for code, `.feature` files, this file and `documentation/**`;
   the only Russian scopes are `documentation/product-design/**`,
   `tools/hil/STRATEGY.md` and `README.RU.md`, the translation of `README.md`.
6. **One mechanism per effect.** Retrying the one path after a detected failure is fine;
   a second parallel route to the same effect is not — it hides which route works.
7. **No comments** in source except one-line markers and tool directives; `#[allow(…)]`
   carries `reason = "…"` ([10](documentation/architecture/10-build-release-and-tooling.md)
   §Comments).
8. **Documentation is general; details live in code.** One home per fact, links
   everywhere else, no dates or history, code wins
   ([`documentation/README.md`](documentation/README.md); `scripts/verify_docs.py`).

## Crates and composition

Detail: [01](documentation/architecture/01-overview.md).

- `dali2rust-contracts` — canonical bus types (`msg/`), envelopes, `postcard` codec.
- `dali2rust-bus` — bounded typed bus: channel traits, router task, publishers.
- `dali2rust-domain` — pure DALI and registry logic, domain traits; no I/O.
- `dali2rust-platform` — HAL traits, shared lock-free primitives; no ESP-IDF types.
- `dali2rust-api` — HTTP router and handlers, confirmation bridge, all JSON building.
- `dali2rust-adapters` — composition helper, transports (ESP, mock, sim), HTTP servers.
- `dali2rust-dali-phy` — what the PHY interrupt executes and nothing else; `no_std`.
- `dali2rust-dali-codec` — Manchester codec, in task context.
- `dali2rust-gear-model` — stateful fake control gear; host-only.
- `dali2rust-dali-runtime` — `DaliWorker`, the wire's only owner, and its priority table.
- `dali2rust-registry-runtime` — `RegistryStore`, its single writer, persistence slices.
- `dali2rust-operations-runtime` — operation tracker, apply orchestrator.
- `dali2rust-fanout-runtime` — state-fanout projector, sniffer translator.
- `dali2rust-display-runtime` — OLED worker and screens.
- `dali2rust-hcl-runtime` — HCL scheduler.
- `dali2rust-poller-runtime` — background re-reads, off by default.
- `dali2rust-ws-runtime` — WebSocket hub and fan-out; carries JSON, never builds it.
- `dali2rust-mqtt-runtime` — Home Assistant bridge; owns the `HomeAssistant*` kinds.
- `dali2rust-rules-model` — typed rule graph, limits, `RuleCompiler` / `NameResolver`.
- `dali2rust-rules-lang` — the one rule compiler, source text → model; no printer.
- `dali2rust-rules-runtime` — rules store, worker and engine; never sees source text.
- `dali2rust-redundancy-runtime` — active/standby arbitration, config pull (ADR-018).
- `dali2rust-ota-runtime` — firmware update over the network (ADR-024).
- `dali2rust-bsp` — board support: spawner, stack classes, `PsramBox`, flash binding.
- `dali2rust-test-support` — shared host test helpers and port doubles.
- `dali2rust-firmware` — the binary and the only production composition root.
- `dali2rust-p4-bringup` — bring-up image, own `ESP_IDF_SYS_ROOT_CRATE`.
- `tests/dali2rust-bdd` — the black-box cucumber suite.

Composition:

- The firmware composes through `dali2rust_adapters::build_router_with_bus_and_transport`
  (generic over `DaliTransport`); BDD and the host dev server call it with
  `MockDaliTransport` / `SimDaliTransport`. Adapters only wire: a worker's logic and
  spawn function live in the runtime crate that owns the slice.
- The API layer never holds a mutex around the DALI transport.
- Traits: HAL → `platform`; DALI and domain → `domain`; bus channels → `bus`; HTTP →
  `api`; rule language → `rules-model`. No ESP-IDF types in `platform` or `domain`;
  ESP-only code sits behind `#[cfg(target_os = "espidf")]`.
- There is no input-device runtime: Part 103 commands execute inside `DaliWorker`. A new
  rule language is a new crate and `lang_id`, never an engine change (ADR-016).
- A new stage mirrors the boundaries of the previous one.
- One board: Waveshare ESP32-P4-ETH, wired Ethernet, no radio (ADR-011). The ESP32-S3,
  the ESP32-C6, the arbiter board, Wi-Fi, LittleFS and FlatBuffers are not part of
  the product; never describe them as live.
- Cluster and adapter proxy are designed, not composed. Built:
  [`status.md`](documentation/product-design/status.md); open work:
  [`roadmap.md`](documentation/product-design/roadmap.md).

## Bus and delivery

Detail: [03](documentation/architecture/03-bus-and-backpressure.md).

- Commands and events route to the subscribers that declared the kind
  (`<WORKER>_HANDLED_*` from `dispatch_bus_commands!` / `dispatch_bus_events!`, with no
  hand-written catch-all arm); confirmations broadcast. `bus_payload_ownership.rs` checks
  one owner per command and a consumer or `OBSERVED_ONLY_EVENTS` entry per event
  (ADR-006, ADR-015).
- A command nobody accepts becomes a synthetic `DeliveryRejected` and an HTTP `503`; an
  undeclared event is dropped silently, which is legal.
- `target_adapter_id` is filtered by the consumer; `sender_id` and `Origin` never route.
- Ingress never blocks (`Queued` / `DroppedIngressFull` / `RejectedFrameTooLarge`); a
  stuck subscriber overflows only its own inbox.
- An event that is the only carrier of a fact — a registry record, a runtime commit, an
  operation's outcome — goes through `publish_required` with one budget per unit of work
  and is declared in `<PUBLISHER>_REQUIRED_EVENTS`; `Singleton` is only the frame that
  closes the unit (ADR-021).
- A route answering `202` + operation id has the operation tracker as its listener; a
  gate's refusal travels by the route's own discipline and names its cause. A new
  answering discipline owes itself a listener.
- No producer publishes an unbounded burst; flow control belongs to the producer, never
  to a deeper queue (ADR-007).
- Coalescing may drop a frame, never a field silently; a producer that means two verbs
  to land together merges them before publishing.
- A subscriber declares facts, not numbers; a setting is read from the registry, and its
  changed-event may only wake the reader. One with nothing to do must not wake.
- A `BusHost` owns the bus and dropping it stops every worker, so a test harness keeps
  its host while it publishes. A self-paced worker waits on its inbox with a timeout,
  never on a bare timer.

## Registry, persistence and contracts

Detail: [06](documentation/architecture/06-registry-and-persistence.md),
[04](documentation/architecture/04-contracts-and-api-bridge.md).

- The registry worker is the single writer (commands and evidence events through one
  funnel); never add a second thread that writes.
- Everyone else reads through the read-port traits; the full `PhysicalDeviceView` is on
  none of them.
- Runtime fields live on the physical-device record and change only through
  `RegistryRuntimeUpdateCommand`, which only the state-fanout projector publishes in
  production. Every commit publishes a `RuntimeStateChangedEvent` with the real
  observation; an entry naming an unbound virtual lamp is refused (`VlUnbound`).
- Runtime fields and operation status are not persisted. Group and scene diffs are
  expanded by the apply orchestrator, never by the registry or a handler.
- A lamp is on because its power says so; a level of 0 is a level. Whether a setpoint
  states a colour is `states_color()`, never `color.is_some()`.
- Hydration joins before HTTP mounts, so no mutating request precedes it; a boot-order
  change keeps that or adds a gate.
- Each persisted slice has its own version: bump only the slice whose shape moved and say
  what the bump resets. A physical-devices bump orphans every virtual-lamp binding.
- Bus and shared types live in `dali2rust-contracts/src/msg/`; no parallel DTOs. A
  payload is declared only in `declare_bus_payloads!` with a worst-case `budget`, and
  variant order is the wire discriminant, growing only at the end (ADR-005).
- Bus frames are `postcard`, at most `MAX_BUS_WIRE_BYTES` (128), fixed-size: no `String`,
  `Vec` or `serde_json`. The DALI byte field is `wire_address`, never a bare `address`.
- JSON exists only at the REST, WebSocket and MQTT boundaries and is built only in
  `dali2rust-api`; a WebSocket payload has the shape of the REST resource it mirrors.
- An HTTP request meets its confirmation only in the pending-confirmation slot pool, fed
  by one confirmation-bridge thread: no slot, ingress full or a rejection → `503`,
  timeout → `504`; a timeout frees the slot, not the command (ADR-002).
- A new counter goes to `/api/v1/stats`; counters are `u32` and wrap;
  `scripts/verify_counter_surface.py` keeps its spellings in step.

## ESP32 threading, httpd and memory

Detail: [02](documentation/architecture/02-runtime-and-threading.md),
[07](documentation/architecture/07-memory-and-cores.md).

- Every thread is a FreeRTOS task on a dual-core SMP chip: never rely on single-core
  interleaving. No 64-bit atomics; use `AtomicU32`.
- Spawn through `dali2rust_bsp::esp_thread`: `spawn_named_stack` at boot,
  `try_spawn_named_stack` for anything a client can trigger. The task name comes from
  there, never from `std::thread::Builder::name`.
- Wait with blocking `recv()` / `recv_timeout()` on the primary channel and drain other
  inboxes with `try_recv()` afterwards. Never a `try_recv()` + `sleep` loop, never
  `thread::park()` on main; idle back-off ≥ 10 ms, heartbeat ≥ 1 s.
- The tick is 1 ms and a shorter sleep spins instead of parking: decide with
  `sleep_parks_the_task`, and mark a deliberate spin `// busy-wait-ok:`.
- One `httpd` task serves every socket, so each millisecond a handler blocks is downtime
  for all clients. A mutating request waits at most `confirmation_timeout_ms` (2000 ms);
  a longer job answers `202` + operation id (ADR-012); several frames of one request
  share one deadline.
- The httpd stack (20 KiB) is a budget: a response is a resource, not a dump; a container
  serializes its children one at a time; view and DTO size ceilings are tests, and
  raising one is not the fix.
- WebSocket sends are queued work on that same task. The device never sends a WebSocket
  ping; client frames are parsed in `esp_ws.rs`, never through `conn.recv()`.
- Nothing holds a spinlock for more than about a tick while the wire is live: no task
  state walks and no heap walks at runtime.
- A log call never blocks: a bounded PSRAM queue and one UART writer; never the VFS UART.
- Internal SRAM is the scarce pool — at its bottom a task stack, DMA or TLS fails, and a
  build with an internal httpd stack boot-loops with no network. A plain C allocation
  under 16 KiB lands in it; on the XIP image a Rust object from 256 B goes to PSRAM first.
  Multi-KiB records go to PSRAM through `PsramBox::new_with` with in-place initialisation,
  never built on a worker stack; move large fields with `mem::swap`, not a clone.
- Everything the PHY interrupt touches, and whatever a driver needs internal, is allocated
  internal by name. A stack's home is
  the spawn's `StackHome`: on the XIP image a thread that never maps flash
  (`esp_partition_mmap`, the `esp_ota_*` calls that read `otadata` or check an image) and
  carries no wire timing has its stack in PSRAM (ADR-028). lwIP and the frames it holds
  are in PSRAM; the EMAC ring is internal and sized against
  `network.rx_ring_overruns_total` (ADR-029).
- The bench gates task stacks and the boot heap ladder
  ([`tools/hil/README.md`](tools/hil/README.md) §Run validity): answer a stack breach with
  a shallower path, never a bigger stack, and measure frame depth on the linked image
  (`scripts/measure_stack_frames.py`) before flashing.
- The display reads a small `Copy` sample of counts, never a registry view.
- A panic resets the chip; code a client can reach fails soft.

## DALI PHY and interrupt

Detail: [08](documentation/architecture/08-dali-phy-and-transport.md).

- Only the GPTimer interrupt mutates the PHY state machine; tasks reach it through
  ISR-safe cells, SPSC rings and atomics. The interrupt never allocates, logs, blocks,
  notifies a task or touches the bus or HTTP (ADR-003).
- Everything the interrupt executes lives in `dali2rust-dali-phy`, and nothing else does.
  Nothing ISR-reachable is generic over an outside type or an `#[inline] pub fn`; no
  `copy_from_slice`, indexing or unchecked arithmetic there (ADR-010).
- The interrupt runs at CLIC level 5, above every critical section, so nothing it touches
  may rely on one (ADR-025).
- `scripts/verify_dali_isr_iram.py` on the linked binary proves the interrupt reaches no
  flash; source review cannot. `hil flash` refuses a red binary.
- The task decides and the interrupt transmits: decoding and the arbitration answer's
  decision stay in task context; the interrupt only sends a staged answer.
- Arming a frame and writing its opening edge are different ticks; new work on the
  transmit-start path moves off that tick rather than getting faster.
- A cancel drains the command cell; a batch abort on foreign reception must not; neither
  touches the answer cell.
- A capture is routed by its length, never by what the session is waiting for.
- Every flash program or erase and every `otadata` read holds `flash_gate` (`try_hold` on
  the httpd task). NVS is never initialised.
- Per-tick work in the interrupt costs timing no host test measures, and the firmware
  cannot judge its own transmit timing; no independent receiver is installed.

## DALI protocol

Detail: [09](documentation/architecture/09-dali-protocol-rules.md); what is not
implemented: [conformance gaps](documentation/reference/iec62386-conformance-gaps.md).

- Product code publishes semantic commands and never assembles opcodes or frames; it
  depends on `DaliProductController`, not `DaliApplicationController`. The raw path
  (`/api/v1/dali/*`) is diagnostic and never mapped onto devices or virtual lamps.
- A Part 103 (24-bit) frame has its own payload, worker branch and `exchange_frame24`,
  never the 16-bit `wire_address` + `command` pair. Part 103 and 102 encode addresses
  and operands differently; use the typed operands.
- Priority comes from the command kind (`WIRE_CLASSES`: exhaustive, no default), never
  the opcode, except on the raw diagnostic path (ADR-013).
- Background work never delays an operator, and that is not a setting; a preempted read
  (`Preempted`) is routine, never shown as a failure (ADR-009).
- A multi-frame unit is a transaction (`controller.transaction`); nothing yields inside a
  started one; a retry repeats the whole unit, prelude included, and only after
  `Collision` or `BusBusy` (ADR-017).
- A violating backward frame is an answer — terminal, `is_yes()`, no `value()` — never a
  collision (ADR-020).
- MASK in an answer is never a value.
- Arm, prove, act once: a DTR operand is proved by read-back before use, and a read-back
  tells a wrong answer, silence and a contended window apart
  ([ADR-027](documentation/architecture/decisions/ADR-027-dtr-operand-proof-and-readback-outcomes.md)).
- A boolean broadcast query needs a positive control; silence alone proves nothing.
- DT8 colour activation follows
  [ADR-026](documentation/architecture/decisions/ADR-026-dt8-colour-activation.md).
  RGBWAF channels are sRGB above the wire and linear on it, compared through the encoder
  (ADR-022).
- A device type is a set; an override may narrow it, never widen it.
- Identification is the gear's own `IDENTIFY DEVICE` procedure; nothing else is sent.
- When our path and a known-good foreign master drive the same gear differently, diff
  the two frame sequences before guessing.

## Testing & BDD

Detail: [05](documentation/architecture/05-testing-and-bdd.md).

- Layers: a unit test for logic, mapping and codecs; a crate integration test
  (`crates/*/tests/`) for one runtime slice on the real bus path; BDD for behaviour of
  the composed host stack.
- BDD is black-box: drive through HTTP and the WebSocket; assert responses, DTOs,
  WebSocket frames, `MockDaliTransport` frames and effects at mocked ports. Never reach
  into `BusStackRuntime`, the bus, `RegistryStore`, slots or counters (ADR-004); a
  behaviour that needs internals is tested in its owning crate.
- Tests use production builders, codecs and composition; mocks sit only at port
  boundaries.
- At least one test takes each field a consumer depends on from its real producer.
- Synchronise on predicates and deadlines, never on sleeps.
- Seed state through the production path: no `bdd` feature, no `bdd_*` hooks, no
  registry-seed reach-in.
- A scenario proves the claimed IEC profile — address byte, opcode, frame order — not
  only success.
- The `.feature` files under `tests/dali2rust-bdd/features/` are the only BDD canon.
  Each scenario has a unique `@id:<PREFIX><NNN>` with a prefix registered in
  [`ids-registry.md`](documentation/product-design/bdd/ids-registry.md); a file has one
  `@stage-*`; unfinished work is `@wip`; each step carries its `// <ID list>`; the
  runner fails skipped steps.
- Features live in `features/<resource>/`, `diagnostic/`, `system/` or `contracts/`;
  `dali/`, `bus/`, `display/` and `registry/` are forbidden.
- Shared helpers live in `dali2rust-test-support`; no per-crate copies.
- A test proves a mechanism only if it exercises it: a counter the run never moved makes
  the result inconclusive, not green.
- A bench fixture neutralises state it does not own (running schedules, for example) and
  restores it; a test that changes gear configuration puts it back.
- A red test is fixed in the code, never by loosening its assert.

## Extending

A device-type command family, a Part 103 command, a bus command or event, a worker, an
HTTP endpoint or a platform backend: follow its checklist in
[11](documentation/architecture/11-extension-recipes.md).

## Build, test, flash

Detail: [10](documentation/architecture/10-build-release-and-tooling.md).

```bash
just check               # every host crate, test targets included
just test                # host tests (--all-targets)
just quick <crates…>     # inner loop: check all, test the named crates, then BDD
just bdd                 # the BDD suite (= cargo bdd)
just clippy              # -D warnings plus allow reasons and SAFETY comments
just verify              # every merge gate
just ci                  # clippy, test, bdd, verify, esp-check, gear-sim-check
just esp-check           # cargo check of the firmware on the P4 triple
just gc                  # sweep stale .o files; just gc-status counts them
cargo fw                 # build the firmware image
cargo run --target aarch64-apple-darwin -p dali2rust-adapters --example host_dev_server
```

- There is no workspace `build.target`, and every alias names its triple, the host one
  included. `MCU` and `ESP_IDF_SDKCONFIG_DEFAULTS` are forced in `.cargo/config.toml`.
- `DALI2RUST_*` knobs are read with `option_env!`, and the build script tracks every name
  it finds in the firmware's `src`. Pass knobs to `hil flash`, which builds for itself,
  and check that the flip reached the binary.
- ESP-IDF is pinned to v5.5.3; move it only between bench experiments.
- The version is `<major>.<minor>.<commit count>+<sha>[.dirty.<stamp>]`, derived by the
  firmware's `build.rs` and reported by `/api/v1/health`, `/api/v1/controller`, Home
  Assistant and the boot log.
- The host dev server runs the composed stack on `SimDaliTransport`; its stdin console
  (`press`, `release`, `foreign`, `occupancy`) plays a foreign master (ADR-014).
- A change to `sdkconfig.p4.defaults` is proven only by a flash; only a wired flash
  changes `partitions-p4.csv`. Everything executable or selectable stays below 16 MB, and
  `storage` never moves.
- Run `just ci` before a commit lands and after anything touching a contract, a wire
  order or ESP-only code; `.github/workflows/ci.yml` does not replace the local run
  (it lags `just ci`: [roadmap](documentation/product-design/roadmap.md)).
- [`tools/dali-gear-sim`](tools/dali-gear-sim/README.md) is a separate workspace pinned to
  the ESP32-C6 and not ported; `just gear-sim-check` proves only that it
  type-checks.
- `hardware/enclosure/` holds the DIN-rail enclosure; the `Params` spreadsheet in
  `dali2rust-case.FCStd` is its only source of dimensions.

## HIL bench

Detail: [`tools/hil/README.md`](tools/hil/README.md) (runbook),
[`STRATEGY.md`](tools/hil/STRATEGY.md) (safety rules, oracles, coverage, watchlist).

- A bench may be production lighting. Anything that visibly changes a lamp needs the
  go-ahead of whoever owns that light for that run; without it, run with
  `HIL_LAMPS_READ_ONLY=1` and deselect every test that drives light (STRATEGY §4).
- Tests drive only `HIL_LAMP_SHORTS`; commissioning never runs on production lighting.
  Bench-specific rules live in `CLAUDE.local.md`, which is not tracked.
- Every session saves the bench and restores it; recover a killed session with
  `hil state restore`.
- Flash only with `hil flash`. A controller reboot is a production event, and so is
  starting a serial bridge that is down.
- Classify a red run from its `HIL validity` section, and validate the instrument before
  believing a measurement (STRATEGY §3).

## Web UI

Detail: [10](documentation/architecture/10-build-release-and-tooling.md) §Embedded web UI
(build chain, cards), [`web-ui/`](documentation/product-design/web-ui/README.md) (screens,
UI rules).

- A UI change reaches the device only through `bash scripts/build_web_ui.sh`, the
  committed mirror and a reflash.
- Every screen and component has a card in `web/design-system/`, written in the same
  change and pushed to the Claude Design project before
  `scripts/verify_design_system_pushed.sh --stamp`.

## Code style

- No magic number without a named constant; imports at module level, never in a
  function body.
- Every `unsafe` block and `Box::leak` carries `// SAFETY:`.
- An identical function in two files moves to a shared module; two identical impl blocks
  are parameterised; three identical blocks must be extracted. Prefer the shared
  `dali2rust-contracts::bus` helpers to local builders.

## Merge gates

`just verify` runs every gate; what each holds is in
[10](documentation/architecture/10-build-release-and-tooling.md) §Merge gates.

- A gate that passes when its tool is missing is not a gate: `DALI2RUST_SKIP_JSCPD=1`
  and `DALI2RUST_SKIP_TSC=1` are the only, explicit opt-outs (in a fresh worktree run
  `npm ci` in `web/app`).
- Budgets and allowlists move only in their direction; widening one to turn a gate green
  is forbidden.
- rustfmt is not a gate: never reformat a whole file; wrap only the lines you touch.

## Where facts live

- Which tree owns which kind of fact, and the rules every document follows:
  [`documentation/README.md`](documentation/README.md).
- A status or stage-completion claim holds only if every cited `@id` exists and passes.
- An `ISSUE-NN` number is claimed in the
  [registry](documentation/product-design/issue-ids-registry.md) first, as its maximum
  plus one, before it appears in a record, code or a commit message; cite one only to
  point at an open issue.
