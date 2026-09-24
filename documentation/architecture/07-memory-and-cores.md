# 07 Memory and Cores

Where memory comes from on the ESP32-P4, what may live where, how task stacks are sized
and gated, why the httpd stack is a budget, and which task runs on which core.

Scope: internal SRAM versus PSRAM, stacks, core placement, reading memory at runtime.
Crash triage and coredumps → [`../reference/debugging.md`](../reference/debugging.md);
flash, XIP and cache discipline → [08](08-dali-phy-and-transport.md); threading rules →
[02](02-runtime-and-threading.md); sdkconfig keys → [10](10-build-release-and-tooling.md).

## The pools

- The P4 has 768 KB of internal L2MEM, 32 MB of in-package PSRAM and 32 MB of flash. The
  image executes from PSRAM ([ADR-025](decisions/ADR-025-phy-interrupt-above-critical-sections.md)).
- **Internal SRAM is the scarce resource.** 128 KB of L2MEM is the L2 cache
  (`CONFIG_CACHE_L2_CACHE_SIZE`, and 128 KB is the smallest choice) and IRAM is carved
  from the same region, so the internal heap is roughly 575 KB. Its consumers are the
  stacks that must stay internal, ESP-IDF and its drivers (the EMAC DMA ring among
  them), and small allocations, C and Rust alike.
- The internal floor after boot is a property of each release; the bench gates the free
  internal heap at each named boot stage
  ([`tools/hil/README.md`](../../tools/hil/README.md) §Run validity).
- The failure at the bottom is not "out of memory". A plain allocation falls back to
  PSRAM; what fails is what must be internal — a task stack, DMA, TLS. On a build whose
  httpd stack is internal, `httpd_start` then fails with `ESP_ERR_HTTPD_TASK`, an abort
  and a boot loop with no network surface to diagnose it from. Every byte a change adds
  to the boot path is weighed against that.

## Placement

- `CONFIG_SPIRAM_MALLOC_ALWAYSINTERNAL` is 16 KiB: a plain C allocation below it is
  internal, above it PSRAM. On the XIP image the firmware's allocator
  (`dali2rust_bsp::rust_heap::CountingAllocator`) puts every Rust object from 256 B in
  PSRAM first and leaves smaller ones internal first
  ([ADR-028](decisions/ADR-028-task-stacks-in-psram-on-the-xip-image.md));
  `DALI2RUST_RUST_HEAP_INTERNAL=1` restores ESP-IDF's size rule for Rust too, where a
  multi-KiB record behind a plain `Box` lands in internal SRAM.
- The network stack is in PSRAM: lwIP allocates there first, and each received frame is
  copied there as it leaves the EMAC ring
  ([ADR-029](decisions/ADR-029-network-buffers-in-psram-and-a-measured-receive-ring.md)).
  The ring itself, 34.5 KB, is internal because the driver's DMA requires it.
- Multi-KiB plain-data records go to PSRAM explicitly through
  `dali2rust_bsp::psram::PsramBox`. Registry tables are pre-sized to the product limits,
  so each is one allocation above the threshold, lands in PSRAM and never rehashes.
  Capacity in PSRAM is free; capacity in internal SRAM is what runs out.
- Internal only, allocated by name: everything the PHY interrupt touches
  ([ADR-003](decisions/ADR-003-isr-owned-phy.md)) and anything a driver needs in internal
  memory. Rust atomics and mutex data may live in PSRAM on the XIP image — the P4's
  atomic operations work there from both cores, and the boot self-test
  `DALI2RUST_PSRAM_ATOMIC_SELFTEST` checks it on the device.
- An allocation that must be internal and is large asks for it with
  `heap_caps_malloc(MALLOC_CAP_INTERNAL)`; a plain allocation that size would land in
  PSRAM.
- A stack's home is chosen at spawn (`dali2rust_bsp::esp_thread::StackHome`,
  [ADR-028](decisions/ADR-028-task-stacks-in-psram-on-the-xip-image.md)): internal;
  PSRAM always, for a task that never touches flash (the UART log writer, the slice
  reload); or PSRAM on the XIP image, for every thread that never maps flash — the
  workers, the httpd task, the watchers, boot hydration. `DALI2RUST_STACKS_INTERNAL=1`
  keeps the last kind internal.
- What rules a PSRAM stack out is mapping flash, not writing it. On the XIP image a
  program, erase or read over SPI1 leaves the cache on; `esp_mmu_map` and `esp_mmu_unmap`
  turn it off on both cores even there and assert that the caller's stack is internal.
  `esp_partition_mmap` maps, and so do `esp_ota_begin`, `esp_ota_end`,
  `esp_ota_set_boot_partition`, `esp_ota_mark_app_valid_*` and
  `esp_ota_get_{state,boot}_partition`. The OTA adapter refuses those calls by name on a
  PSRAM stack; the update and boot-verify threads keep internal stacks for them.
- Internal stacks stay for what carries the wire's timing — the DALI worker, the DALI
  sniffer, the bus task — and for `main` and every ESP-IDF task.
- Stream rather than buffer: HTTP responses have no full-body buffer, and the
  persistence flush reuses one small chunk buffer ([06](06-registry-and-persistence.md)).

## Building large values

- Allocate first, initialise in place: a multi-KiB record is never built on a worker
  stack. `PsramBox::new(T)` takes `T` by value, so the caller's frame holds two copies
  before a byte reaches PSRAM. Records are built with `PsramBox::new_with` and an
  in-place initialiser (`PhysicalDeviceRecord::init_empty`); the by-value constructor
  exists only in tests, and a section above `MAX_INLINE_INIT_TEMP_BYTES` (2 KiB) owes its
  own in-place default. Stack-footprint tests hold all three.
- On a worker stack a multi-KiB field is moved with `mem::swap`, never cloned into
  place.
- A helper that builds a large value for a caller on a tight stack (route wiring on
  `main`, the diagnostics and stats DTOs for the WebSocket worker) is `#[inline(never)]`:
  inlined, its frame is added to the caller's for the caller's whole body. The two DTOs
  are filled in place in a `Box` (`*_dto_into`), never returned by value.
- The standard slice sorts keep their scratch in their own frame: 4 KiB for `sort*`
  (driftsort), up to 48 elements for `sort_unstable*`. Firmware code orders its lists,
  which product limits bound, with the in-place, stable
  `dali2rust_platform::small_sort::insertion_sort_by`. A `BTreeMap` (a `serde_json::Map`
  included) collected from an array or an iterator runs the same stable sort, so one is
  filled with `insert`.

## Stack classes

A class is chosen by what the thread does, not by inheritance; where its stack lives is
the spawn's `StackHome` (§Placement). Constants live in `dali2rust_bsp::std_thread_stack`
unless named.

| Class | Size | For |
| --- | --- | --- |
| `HYDRATION_WORKER` | 48 KiB | One-shot boot hydration, joined; the standby's slice reload |
| `HTTPD_TASK_STACK_BYTES` (adapters) | 20 KiB | The one httpd task |
| `OTA_UPDATE_STACK` | 16 KiB | One update, spawned fallibly when it starts |
| `COMMAND_WORKER_STACK` | 12 KiB | Deep executors: DALI worker, rules worker |
| `STATE_WORKER_STACK` | 10 KiB | Workers that own state and build views: registry, operation tracker |
| `EVENT_WORKER_STACK` | 8 KiB | Workers that route small payloads: projector, sniffer translator, apply orchestrator, HCL, poller, WebSocket, MQTT, replication; one `ws-client` sender per connected client, so the four-client cap holds 32 KiB |
| `RING_CONSUMER_STACK` | 8 KiB | Ring and driver pollers: `dali-sniff`, IP watcher |
| `DISPLAY_WORKER` | 6 KiB | Display worker, SNTP watcher |
| `ARBITRATION_WORKER_STACK` | 4 KiB | Arbitration probe worker |
| `OTA_LISTENER_STACK`, `ARBITRATION_SUPERVISOR_STACK`, `CENSUS_STACK` | 3 KiB | Permanently idle or once-a-minute tasks |

- A task that idles for the whole uptime — a listener, a supervisor, a feature that is
  off by default — gets the smallest class; the work it triggers runs on a thread
  created for the occasion.
- The bus task and the confirmation bridge use 8 KiB constants local to their crates;
  `main` has the sdkconfig's 12 KiB for the composition boot and returns afterwards, so
  that stack is freed. The census thread carries the heartbeat. `mqtt_task` is sized by
  the bridge (`task_stack`); ESP-IDF allocates it internal.

## The stack census is a gate

- Every task's high-water mark is logged once a minute (the census, [02](02-runtime-and-threading.md)),
  and the bench gates each task against its stack budget
  ([`tools/hil/README.md`](../../tools/hil/README.md) §Run validity).
- A breach is answered by making the path shallower, not by growing the stack.
- `StackLowWater` (`dali2rust-bsp::stack_probe`) names the path the census cannot: the
  registry worker samples once per turn, tagged with the payload kind that just ran.
- The census line (`task stack hwm (B free…): <task>=<bytes> …`) and the boot-stage lines
  (`boot heap [<stage>]: internal free=…`, `internal heap before httpd: free=…`) are
  parsed by the bench's validity section and `hil corpus`: reword one, or rename a stage,
  together with `tools/hil/hil/validity.py` and `boot_heap_budget.txt`.
- Measure a change's frame depth on the linked image before flashing, not after
  (`scripts/measure_stack_frames.py` reads every stack adjustment of a function, not only
  the first `addi sp`, and adds a prologue the size-optimised build outlined into an
  `OUTLINED_FUNCTION_*` fragment).

## The httpd stack is a budget

- Every handler runs on the one httpd task and builds its response there.
- A response is a resource, not a dump. When a view grows, the resource splits: physical
  devices are read as a summary list, a device core, attribute sections chosen with
  `?sections=`, and bank coverage.
- A container serializes its children one at a time inside `serialize` rather than
  holding them all.
- The full physical-device view is off every read-port trait
  ([06](06-registry-and-persistence.md)), so a handler cannot reach it: a compile error,
  not a review convention.
- Size ceilings on views and DTOs are tests. Raising a ceiling is not the fix; moving
  the stack size needs a bench high-water measurement first.
- On the XIP image the stack is in PSRAM (`task_caps`) and nothing is reserved; the task
  never maps flash — the running OTA slot comes from the reading taken at boot and after
  each update ([08](08-dali-phy-and-transport.md)).
- On a build without XIP the stack must be one contiguous internal block.
  `HttpdStackReserve` takes it, plus headroom for `httpd_start`'s own allocations, early
  in boot before the worker fleet fragments the heap, and releases it immediately before
  `httpd_start`.

## Workers read narrow views

- A worker that reads the registry on its own stack does so through a narrow port of
  small `Copy` views (poll targets, lamp capabilities, policy cells, Home Assistant
  publish views); the REST-shaped views, which allocate owned strings per row, are never
  in that port's trait bounds.
- The display worker's stack is 6 KiB, so it never holds a registry view; a fat view is
  not made safe by being read quickly. It reads a small `Copy` sample: the registry's
  counts, the poller's settings and the adapter gate come from `display_sample_parts`
  under one read lock, so one screen shows one instant; everything else on the screen is
  an atomic load, assembled in `dali2rust-adapters/src/runtime/display_source.rs`, so
  the display crate depends on no other runtime.

## Cores

- Two cores, FreeRTOS SMP ([02](02-runtime-and-threading.md)).
- The DALI PHY interrupt is allocated from the main task and runs on core 0.
- Pinned to core 1, away from the interrupt: the heartbeat, census and heap diagnostics
  thread and the UART log writer.
- Everything else, httpd and lwIP included, is unpinned. The heartbeat reports where
  the interrupt and httpd actually run (`cores: dali_isr=… httpd=…`).

## Reading memory at runtime

- `GET /api/v1/stats` reports internal free, largest block, minimum ever, total and
  allocated-block count, and the PSRAM-dominated free heap. The serial heartbeat reads
  the same `HeapStatsPort` (`dali2rust-platform::heap`, ESP implementation in
  `dali2rust-bsp::heap_stats`), so the two cross-check. Host builds report `null`, and
  the web UI hides the block.
- The firmware's global allocator (`dali2rust_bsp::rust_heap::CountingAllocator`) counts
  the Rust heap by placement: live and peak bytes in internal SRAM and in PSRAM on
  `/api/v1/stats`, and the internal bytes by size class on the heartbeat's `rust heap:`
  line. Everything else on the internal heap is C: IDF, lwIP, drivers, task stacks.
- Largest block, total and block count are boot-stage figures from one heap walk per
  stage; nothing walks the heap at runtime ([02](02-runtime-and-threading.md)).
- The internal minimum falls at runtime as well as at boot — an update, WebSocket
  clients and bursts of requests each take internal SRAM while they last. The heartbeat
  prints `internal SRAM low-water` with the Rust heap's share each time the minimum
  drops below 12 KiB, and the bench gates the minimum at session end
  (`runtime_heap_budget.txt`).
- Boot cost is logged per stage (composition start, around hydration, around the OTA
  worker, after the workers, before httpd); those are the stages
  `boot_heap_budget.txt` gates.
