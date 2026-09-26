# 10 Build, release and tooling

How the workspace is built, versioned, configured and flashed, and which gates a change
passes before it merges.

Scope: toolchain, cargo configuration, the justfile, the firmware image (version, knobs,
sdkconfig, partitions), the web UI chain, merge gates and the comment policy. Test and
BDD conventions → [05](05-testing-and-bdd.md); updates over the network →
[ADR-024](decisions/ADR-024-ota-over-ethernet.md); the bench runbook →
[`tools/hil/README.md`](../../tools/hil/README.md).

## Toolchain and cargo

- `rust-toolchain.toml` pins the `esp` toolchain installed by `espup` for every build:
  a nightly-channel rustc with `rust-src`, which `[unstable] build-std` needs to build
  `std` for the tier-3 target `riscv32imafc-esp-espidf`. It lists no `components`;
  rustup rejects them for a custom toolchain.
- No workspace `[build] target`, and every build and alias names its triple — an alias
  without one builds a second copy of the workspace into `target/debug` (the cargo
  limitations behind this: [ISSUE-147](../product-design/known-issues.md)). The aliases
  are `cargo fw` (build), `cargo flash` (build, flash with `espflash` and
  `partitions-p4.csv`, monitor) and `cargo bdd`. The host triple is literal in
  `[alias]` and in the justfile's `default_host`; another host edits both.
- `[env]` forces `MCU = esp32p4` and `ESP_IDF_SDKCONFIG_DEFAULTS = sdkconfig.p4.defaults`
  (`force = true`): otherwise a variable already in the shell wins, and a stale one builds
  for the wrong die. `ESP_IDF_SYS_ROOT_CRATE` (the crate being built),
  `ESP_IDF_VERSION` and `ESP_IDF_TOOLS_INSTALL_DIR` stay overridable. `hil flash` reads
  that block line by line, so each entry stays on one line as `KEY = "value"` or
  `KEY = { value = "…", force = true }` with `value` first.
- `--cfg espidf_time64` matches the `libc` crate's `time_t` to ESP-IDF 5's 64-bit one.
- ESP-IDF is pinned to v5.5.3: `esp-idf-hal` 0.46.2 does not compile against v5.5.5, and
  `embuild` below 0.33.2 cannot parse its tool list. Move it between bench experiments,
  never inside one.

## justfile

| Group | Recipes |
| --- | --- |
| Host | `check`, `test` (`--all-targets` over `scripts/host_crates.txt`, so no doc-tests), `quick <crates…>` (check all, test the named crates and BDD), `clippy` (`-D warnings`), `clippy-pedantic-advisory`, `fmt` (manual; rustfmt is no gate — wrap only lines you touch) |
| BDD | `bdd`, `bdd-check`, `bdd-stage`, `check-stage-clean` |
| Gates | `verify`; `ci` = clippy, test, bdd, verify, `esp-check`, `gear-sim-check`; `contracts-check` |
| Firmware | `esp-check` (`cargo check` on the P4 triple), `p4-fw-build`, `p4-isr-iram-check`, `p4-fw-flash` |
| Other | emulator `gear-sim-check` / `gear-sim-isr-iram-check`; `gc`, `gc-status`; `hil-preflight`, `hil-smoke`, `hil-default`, `hil-slow` |

- `just ci` before a commit lands and after anything that moves a contract, a wire order
  or ESP-only code; `just quick` is the inner loop. `ci` never links the firmware.
- `check` compiles test targets too, because test doubles are what breaks unseen;
  `clippy` covers production targets only. There is no destructive HIL recipe: that tier
  runs one test at a time, by hand.
- Host test time is launch time, not test time: `just gc` sweeps stale `.o` files and
  `just gc-status` counts them (the macOS mechanism: ISSUE-147).
- The `p4-fw-flash` recipe names a local USB port; the installed boards are flashed with
  `hil flash` ([runbook](../../tools/hil/README.md#flashing)).

## Firmware version

- `crates/dali2rust-firmware/build.rs` derives `<major>.<minor>.<commit count>+<8-char
  sha>`, adding `.dirty.<MMDDThhmm>` (UTC) when tracked files differ from the commit and
  `.unknown` when `git status` fails; without git it is `0.1.0+unknown`. A clean build
  has no stamp, so a commit always rebuilds to the same version.
- `/api/v1/health`, `/api/v1/controller`, Home Assistant's `sw_version` and the boot log
  report it, threaded from the composition root through
  `build_router_with_bus_and_transport`; `hil flash` checks it against the image it wrote
  ([runbook](../../tools/hil/README.md#flashing)).
- Any `rerun-if-*` line replaces cargo's default, so the script declares its inputs: the
  firmware `src`; git `HEAD`, branch ref, `packed-refs` and `logs/HEAD` (via
  `git rev-parse --git-path` for worktrees, and only if they exist — a missing path
  counts as changed); and `crates`, `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`,
  `sdkconfig.p4.defaults`, which `.dirty` speaks for.

## Build-time knobs

Read with `option_env!`. The build script declares every `DALI2RUST_*` name under the
firmware's `src` as `rerun-if-env-changed`, so a knob needs no build-script edit.
`hil flash` builds for itself: pass knobs to it, and check the flip reached the binary.

| Knob | Effect |
| --- | --- |
| `DALI2RUST_DALI_RETRY_MAX_ATTEMPTS`, `_BACKOFF_MS`, `_JITTER_MS` | Unit retries and backoff after `Collision` / `BusBusy` / `ForeignInWindow` |
| `DALI2RUST_DALI_QUERY_CONTENTION_RETRY` | Retry a query's `NoAnswer` when foreign activity was seen; `hil flash` requires `1` |
| `DALI2RUST_DALI_QUERY_CONTENT_CONFIRM`, `_MAX_SAMPLES` | Bounded re-reads of contended idempotent queries; `hil flash` requires `1` |
| `DALI2RUST_DALI_DISCOVERY_STEP_RETRIES` | Discovery verify and random-address retries |
| `DALI2RUST_DALI_TARGET_SEQUENCE_RETRIES` | Re-runs of a target-state sequence after `bus_contended` |
| `DALI2RUST_PHY_ISR_LEVEL` | `3`, `4`, `5` (default): PHY interrupt level; `3` is the driver rollback path |
| `DALI2RUST_PERSIST_DISABLE` | `1`: no slice store — no flash writes, no hydration; diagnosis only |
| `DALI2RUST_HEAP_CHECK_S` | Heap-integrity sweep period from the heartbeat; absent or `0` = off |
| `DALI2RUST_STACKS_INTERNAL` | `1`: every stack that would be in PSRAM on the XIP image stays internal, httpd reserve included ([ADR-028](decisions/ADR-028-task-stacks-in-psram-on-the-xip-image.md)) |
| `DALI2RUST_ETH_RX_INTERNAL` | `1`: received frames stay in the EMAC driver's internal buffer instead of a PSRAM copy ([ADR-029](decisions/ADR-029-network-buffers-in-psram-and-a-measured-receive-ring.md)) |
| `DALI2RUST_RUST_HEAP_INTERNAL` | `1`: Rust allocations keep ESP-IDF's internal-first placement instead of PSRAM from 256 B ([ADR-028](decisions/ADR-028-task-stacks-in-psram-on-the-xip-image.md)) |
| `DALI2RUST_PSRAM_ATOMIC_SELFTEST` | `1`: at boot both cores race atomic increments on one PSRAM word and log `psram atomics self-test: ok` or `FAILED` |
| `DALI2RUST_ESP_VERBOSE` | Verbose ESP-IDF component logs |

## sdkconfig.p4.defaults

The only baseline. The generated
`target/riscv32imafc-esp-espidf/debug/build/esp-idf-sys-*/out/sdkconfig` is the authority
(Kconfig may clamp silently), and a change is proven only by a flash: a wrong value can
abort the boot where no `cargo check` sees it. Experiments layer on top
([`sdkconfig.experiments/`](../../sdkconfig.experiments/README.md)).

| Key | Value | Why |
| --- | --- | --- |
| `PARTITION_TABLE_CUSTOM_FILENAME` | `../../../../../../partitions-p4.csv` | relative to esp-idf-sys's CMake project dir |
| `ESP32P4_SELECTS_REV_LESS_V3` | `y` | the dies are revision v1.3; without it the image will not boot below v3.0 |
| `SPIRAM_MALLOC_ALWAYSINTERNAL` | `16384` | smaller allocations stay internal ([07](07-memory-and-cores.md)) |
| `SPIRAM_XIP_FROM_PSRAM` | `y` | flash writes open no cache-off window ([ADR-025](decisions/ADR-025-phy-interrupt-above-critical-sections.md)); costs a boot copy and ~4,5 MB of PSRAM |
| `SPIRAM_TRY_ALLOCATE_WIFI_LWIP` | `y` | lwIP's pbufs, segments and control blocks come from PSRAM first; the EMAC DMA ring is allocated internal by the driver whatever this says ([ADR-029](decisions/ADR-029-network-buffers-in-psram-and-a-measured-receive-ring.md)) |
| `MBEDTLS_EXTERNAL_MEM_ALLOC` | `y` | a TLS session's buffers (16 KiB in, 4 KiB out) live in PSRAM; ESP-IDF's default puts them in internal SRAM |
| `MQTT_USE_CUSTOM_CONFIG`, `MQTT_OUTBOX_DATA_ON_EXTERNAL_MEMORY` | `y` | messages the broker has not acknowledged wait in PSRAM; the custom block's other defaults equal the built-in ones |
| `ETH_RMII_CLK_INPUT`, `_CLK_IN_GPIO` | `y`, `50` | the PHY feeds REF_CLK into GPIO50 |
| `ETH_DMA_BUFFER_SIZE`, `ETH_DMA_RX_BUFFER_NUM`, `ETH_DMA_TX_BUFFER_NUM` | `768`, `30`, `15` | a full frame in two descriptors: 15 full frames or 30 small ones on receive, 34.5 KB internal; sized against `network.rx_ring_overruns_total` ([ADR-029](decisions/ADR-029-network-buffers-in-psram-and-a-measured-receive-ring.md)). Kconfig takes 3–30 buffers per ring and 256–1600 bytes, 64-aligned on the P4, and replaces a value out of range with its default without a word |
| `ETH_TRANSMIT_MUTEX` | `y` | one mutex against silent frame corruption when several tasks transmit |
| `LWIP_STATS` | `y` | the httpd slow-socket line reads lwIP's TCP and link counters to tell "sent and lost" from "never sent" |
| `LWIP_TCP_SND_BUF_DEFAULT`, `_WND_DEFAULT` | `23040` | 16 × MSS keeps fast retransmit reachable; no window scaling on a wired LAN; the buffers are in PSRAM ([ADR-029](decisions/ADR-029-network-buffers-in-psram-and-a-measured-receive-ring.md)) |
| `LWIP_TCP_RECVMBOX_SIZE` | `18` | window / MSS + 2: a smaller mailbox refuses the rest of the window and the sender waits for a retransmit — three concurrent uploads ran at a tenth of one |
| `LWIP_TCP_OOSEQ_MAX_PBUFS` | `4` | out-of-order segments held per connection; with lwIP in PSRAM ESP-IDF's default would be unbounded |
| `LWIP_MAX_SOCKETS` | `16` | ≥ httpd's `max_open_sockets` ([02](02-runtime-and-threading.md)) + its 3 internal descriptors, or `httpd_start` fails and the boot aborts |
| `FREERTOS_HZ` | `1000` | short sleeps park instead of spinning ([02](02-runtime-and-threading.md)) |
| `ESP_MAIN_TASK_STACK_SIZE` | `12288` | `main` runs the composition boot and returns; the heartbeat runs on the census thread, and `main`'s stack is freed |
| `ESP_SYSTEM_EVENT_TASK_STACK_SIZE` | `3072` | the event loop task runs the Ethernet and IP handlers, well under a kilobyte deep |
| `ESP_COREDUMP_ENABLE_TO_UART`, `FREERTOS_WATCHPOINT_END_OF_STACK` | `y` | post-mortem ([debugging](../reference/debugging.md)) |
| `HTTPD_WS_SUPPORT` | `y` | without it `/api/v1/ws` is not a WebSocket route |
| `HTTPD_QUEUE_WORK_BLOCKING` | unset (off) | the WebSocket handler queues socket closes from the httpd task itself; a blocking queue would wait on its own task |
| `GPTIMER_ISR_CACHE_SAFE` | `y` | the level-3 rollback's driver alarm keeps firing with the cache off |
| `ESP_PANIC_HANDLER_IRAM` | `y` | a panic with the cache off still prints instead of a bare watchdog reset |
| `BOOTLOADER_APP_ROLLBACK_ENABLE` | `y` | an image that never marks itself valid is rolled back; a DEBUG-level bootloader does not fit below `0x8000` |

## Partitions (`partitions-p4.csv`)

| Name | Offset | Size | Rule |
| --- | --- | --- | --- |
| `nvs` | `0x9000` | 64 KB | never initialised ([08](08-dali-phy-and-transport.md)) |
| `phy_init` | `0x19000` | 4 KB | kept without a radio: its absence fails a boot-time lookup |
| `otadata` | `0x1A000` | 8 KB | below 16 MB, or the bootloader ignores it |
| `ota_0`, `ota_1` | `0x20000`, `0x620000` | 6 MB each | below 16 MB, or an update started from the slot faults; empty `otadata` boots `ota_0` |
| `storage` | `0xC20000` | 4 MB | persistence slices; never moves (that orphans every virtual-lamp binding) |

Nothing executable or selectable goes above `0x1020000`. `esp_ota_begin` refuses an image
larger than a slot. Changing the table takes a wired flash.

## Embedded web UI

- The UI ships in flash: `bash scripts/build_web_ui.sh` builds and gzips `web/app` into
  `crates/dali2rust-firmware/assets/web/` (committed, so `cargo fw` needs no Node). A pull
  request carries sources only; the maintainer rebuilds the bundle when syncing a release.
  `scripts/verify_web_mirror_fresh.sh` compares it with `web/app`: `hil flash` refuses a
  stale bundle (`--allow-stale-ui` builds with the previous one), and `just
  verify-release` and an advisory CI job on `main` report it. The product-name table (`dali-products.json`) ships empty; a gitignored
  `crates/dali2rust-firmware/assets/local/dali-products.json.gz` replaces it in the image
  when present. A new bundle file needs a row in the firmware's `src/web_assets.rs` (or it
  is a silent 404), in `MANIFEST_FILES` of `build_web_ui.sh` and in the host dev server's
  `asset_route`; no gate checks the dev server's copy.
- `/assets/*` is served immutable for a year and `index.html` no-cache, so the
  `?v=<bundle hash>` that `build_web_ui.sh` stamps on every `/assets/` reference is the
  bundle's only freshness mechanism.
- Every bundle file is stored gzipped and always served as stored, with
  `Content-Encoding: gzip`, whatever the request's `Accept-Encoding`.
- `build_web_ui.sh` and `verify_web_mirror_fresh.sh` fingerprint the same `web/app` source set
  (sources, `public/`, the entry HTML, the lockfile, the TypeScript and Vite configs);
  changing one list without the other breaks the mirror-freshness gate.
- Every screen and component has a card in `web/design-system/`, written in the same
  change as the UI, with the shared `:root` block spliced from an existing card. A change
  to screens, components or `app.css` that leaves the look as it was carries the commit
  trailer `UI-Design: unchanged` instead. The
  design language the cards and the app share is in
  [`web-ui/README.md`](../product-design/web-ui/README.md).
- The maintainer pushes the cards, when syncing, to the Claude Design project "dali2rust Web UI"
  (`0f3fcd66-9619-445b-b9bc-51bc578eefd9`) with `DesignSync`: `list_files` / `get_file`
  first (the owner edits there; never replace the project), `finalize_plan` (`deletes:
  []`), `write_files` with `localDir = web/design-system`, then
  `scripts/verify_design_system_pushed.sh --stamp` — only after the write succeeded.

## Merge gates (`just verify`)

A missing tool fails its gate (`DALI2RUST_SKIP_JSCPD=1`, `DALI2RUST_SKIP_TSC=1` are the
explicit opt-outs), and no gate keeps its own crate list. The `scripts/*budget*.txt`
files only go down; the bench's own budgets are in the
[HIL runbook](../../tools/hil/README.md).

- A new host-buildable crate is added to `scripts/host_crates.txt` only — the one list
  that `just check` / `test` / `clippy`, `verify_fn_length.sh` and the pedantic advisory
  read. The ESP-only firmware crate and the BDD crate stay out of it.
- The 40-line rule is measured on production code: `verify_fn_length.sh` runs clippy on
  `--lib` targets, so unit-test modules, integration tests and the BDD crate are outside
  it. `verify_fn_length_esp.py` finds ESP-only code by the literal
  `target_os = "espidf"` in a file, by `#[cfg(target_os = "espidf")] mod name;` on its
  declaration, by the `esp_idf.rs` / `esp_ws.rs` file names, or by the firmware and BSP
  paths; a module gated any other way is measured by neither gate.

| Script | Holds |
| --- | --- |
| `verify_contracts_codegen.sh` → `verify_native_contracts.sh` | `msg/` + `postcard` only; no FlatBuffers or byte round-trip builders |
| `verify_docs.py` | every markdown link resolves; size caps (`scripts/doc_size_caps.txt`); no dated text; no passage of 40 words repeated between documents |
| `verify_issue_ids.py` | every `ISSUE-NN` resolves to one issue-registry row |
| `verify_bdd_ids.sh`, `verify_bdd_coverage.sh` (+ tree policy), `verify_bdd_layers.sh`, `verify_no_bdd_production_hooks.sh`, `verify_test_layers.sh` (+ `verify_duplication.sh`) | [05](05-testing-and-bdd.md) |
| `verify_runtime_boundaries.sh` | no `#[path]` in runtime crates; fixed composition file set |
| `verify_fixed_bus_guardrails.sh` | no `String`/`Vec`/JSON in bus messages or registry state |
| `verify_comments.py` | no comment outside the one-line markers; budget `scripts/comment_budget.txt` |
| `verify_web_assets.sh` | every embedded file present, `tsc -b`, UI tests |
| `verify_web_classes_styled.py` | every `web/app` class has a CSS rule |
| `verify_design_vocabulary.py` | one `:root` per card; `web/app` speaks card vocabulary |
| `verify_ui_follows_design.sh` | a visual `web/app` change since `origin/main` changes a card, or declares `UI-Design: unchanged` |
| `verify_fn_length.sh`, `verify_fn_length_esp.py` | no function over 40 lines, host and ESP-only code |
| `verify_counter_surface.py` | counter names agree across spellings ([04](04-contracts-and-api-bridge.md)) |
| `verify_read_surface.py` | every read-payload block reaches a screen |
| `verify_dali_isr_iram.py` | the PHY interrupt reaches no flash; soft here, hard in `hil flash` |

## Comments

- Source comments are banned except one-line markers: `// SAFETY:` (and `/// # Safety`
  plus one line on a public `unsafe fn`), `// sleep-ok:`, `// busy-wait-ok:`, a BDD
  step's `// <ID list>`, and a citation such as `// IEC 62386-102 §9.4`.
- Rationale lives in this directory and the ADRs; a workaround for an external defect in
  [`known-issues.md`](../product-design/known-issues.md).
- An `#[allow(…)]` carries `reason = "…"` instead of a comment; `just clippy` denies one
  without it, and an `unsafe` block without `// SAFETY:`.
- `scripts/verify_comments.py` enforces it; `--strip <paths>` removes violations and
  leaves a file alone if its tokens would change. A branch that conflicts with the strip
  takes its own version and re-runs `python3 scripts/verify_comments.py --strip <paths>`.

## Beside the workspace

- [`tools/dali-gear-sim`](../../tools/dali-gear-sim/README.md), the gear emulator, is
  its own cargo workspace; its README states what `gear-sim-check` does and does not
  prove ([ADR-014](decisions/ADR-014-gear-model-and-second-dali-endpoint.md)).
- `hardware/enclosure/` models the DIN-rail enclosure; the `Params` spreadsheet in
  `dali2rust-case.FCStd` is its source of dimensions.
