# 02 Runtime and Threading

How the composed stack runs on FreeRTOS: which tasks exist, how a worker waits, what the
single HTTP task may do, how logging stays non-blocking, and what nothing may do while
the DALI wire is live.

Scope: the task model and its blocking rules. The PHY interrupt and the task↔ISR
boundary → [08](08-dali-phy-and-transport.md); stack sizes, memory placement and core
placement → [07](07-memory-and-cores.md); bus routing → [03](03-bus-and-backpressure.md);
registry locking → [06](06-registry-and-persistence.md).

## Tasks

- Every `std::thread` is a FreeRTOS task. Each runtime crate ([01](01-overview.md))
  owns the spawn function of its workers and the composition starts them
  (`SpawnedWorkers` in `dali2rust-adapters`). The MQTT bridge, the WebSocket fan-out and
  the OTA listener are optional: absent when composition wires no client or the build
  cannot update itself.
- Beside the workers: the bus routing task, the transport's `dali-sniff` drain, the one
  `httpd` task, the UART log writer, the census thread, the IP and SNTP watchers, the
  OTA boot verifier, and per-occasion threads: `ota-run` per update and one `ws-client`
  sender per connected WebSocket client, on the device as on the host (its stack:
  [07](07-memory-and-cores.md)).
- `registry-hydrate` is a one-shot boot thread, joined before any worker starts
  ([06](06-registry-and-persistence.md)).
- The P4 is dual-core and FreeRTOS runs SMP: two tasks, or a task and the PHY interrupt,
  execute at the same instant. Correctness never rests on single-core interleaving.
- The target has no 64-bit atomics; shared counters are `AtomicU32` and wrap.

## How a worker waits

- The primary channel is read with a blocking `recv()` or a bounded `recv_timeout()`.
  Secondary inboxes are drained with `try_recv()` only after the primary wait returns;
  `dali2rust_bus::recv_then_drain` is the shape.
- Never a `try_recv()` + `thread::sleep` polling loop. Never `thread::park()` on the main
  thread; `main` ends in a coarse sleep loop.
- Idle back-off ≥ 10 ms, heartbeat or keep-alive ≥ 1 s; anything under 1 ms is a busy
  loop.
- A self-paced worker waits on its inbox with a timeout, never on a bare timer: its
  inbox disconnecting is how it learns the stack is gone (see *Bus lifetime*).
- A subscriber with nothing to do does not wake ([03](03-bus-and-backpressure.md)).

## The tick and sub-tick waits

- `CONFIG_FREERTOS_HZ` is 1000, so a tick is 1 ms.
- A sleep shorter than one tick does not sleep: newlib's `usleep` / `nanosleep` call
  `vTaskDelay` only from one tick upward and spin in `esp_rom_delay_us` below it. A
  thread in that state never yields, the idle task on its core never runs, and the task
  watchdog names a task that is not at fault.
- Decide with `dali2rust_platform::dali::sleep_parks_the_task(step_ms, tick_ms)`, with the
  tick derived from `configTICK_RATE_HZ`, never by comparing with a literal.
- A deliberate sub-tick spin is marked `// busy-wait-ok:`, never `// sleep-ok:`, so
  searching for the first marker lists every place the firmware spins on purpose (marker
  rules: [05](05-testing-and-bdd.md)).

## Spawning

- Every thread spawns through `dali2rust_bsp::esp_thread`. `spawn_named_stack` is for
  boot-time workers and panics if the spawn fails, because an incomplete stack must not
  run. `try_spawn_named_stack` is for anything a client can drive: under
  `panic = abort` a failed spawn there would be a remote reboot switch.
- The FreeRTOS task name comes from the `ThreadSpawnConfiguration` that helper sets
  (truncated to 15 characters). `std::thread::Builder::name` never reaches the task, so a
  thread spawned any other way reads `pthread` in every watchdog dump, coredump and
  census.
- Names are `&'static CStr`, so a spawn allocates nothing the process cannot reclaim.
- The helper registers each thread in the BSP task registry for its lifetime; the
  census reads that table.

## The HTTP task

- `esp_http_server` runs **one** task that `select()`s over every open socket and runs
  each handler inline. `max_open_sockets` (10) sizes the descriptor set, not a pool; its
  bound against the lwIP socket count is in
  [10](10-build-release-and-tooling.md#sdkconfigp4defaults).
- Every millisecond a handler blocks, no other client is served. A mutating request that
  waits for its confirmation holds the task up to `confirmation_timeout_ms` (2000 ms).
  Work that would wait longer moves the wait off the task — accept, answer `202` with an
  operation id ([ADR-012](decisions/ADR-012-async-chunked-config-writes.md)) — rather
  than raising a timeout.
- A handler that publishes several frames shares **one** deadline across them; a fresh
  timeout per frame is forbidden. `publish_batch_and_wait_for_success` is the shape.
- Responses are built on this task's stack, which is a budget
  ([07](07-memory-and-cores.md)).

## WebSocket on the HTTP task

- `httpd_ws_send_frame_async` queues the send as work for the httpd task: the caller
  does not block, but the send occupies the task every socket shares. `esp-idf-svc`
  fixes the send timeout at 5 s, so the design case is one 5 s stall of every socket.
- Bounds that keep it there: sniffer records batched into one frame per 100 ms under a
  2048-byte payload budget (a byte cap, enforced where the bytes exist; shed records are
  reported in `dropped_since`); a per-client queue bounded by 32 frames and 16 KiB,
  whichever comes first, that drops and tells the client to refetch rather than
  accumulate; a client reaped on its first send error; at most four clients, each
  holding a socket for its session.
- The device never sends a WebSocket ping.
- On the device a client's frames go out on its `ws-client` thread, which queues each
  send to the httpd task and waits for it. Code already running on the httpd task (the
  upgrade handler, a PONG, a CLOSE) sends through the connection itself, never through
  that path: queued work waiting on its own task never runs.
- `/api/v1/ws` is registered before the wildcard routes (`esp_http_server` matches URI
  handlers in registration order, and `/*` would take the upgrade) by
  `dali2rust-adapters/src/http/esp_ws.rs`, with ESP-IDF's `handle_ws_control_frames`
  enabled, and every frame is parsed there: the payload read
  happens only when the frame length is non-zero, the opcode is classified by our code,
  and anything unparseable closes the socket. Client frames never go back through
  `esp-idf-svc`'s `conn.recv()`. In exchange the handler answers PING with PONG and CLOSE
  with CLOSE, and a client slot is released by the CLOSE frame. The external defects this
  avoids are recorded under the workarounds in
  [`known-issues.md`](../product-design/known-issues.md).

## Logging never waits

- A producer reserves one slot of a bounded PSRAM queue with a single `compare_exchange`,
  formats into that slot and publishes it with one release store. Some slots are held
  back for WARN and ERROR. A full queue, or a reservation lost to other producers after
  a bounded number of attempts, drops the line and counts it.
- One writer task on core 1, with its stack in PSRAM, is the only caller of the blocking
  `uart_write_bytes`; it parks on its inbox and feeds the UART driver's TX ring. The
  VFS UART path is not used: it costs a lock and a ring send per byte.
- A Rust panic drains the queue through `esp_rom_printf`.
- If the queue or the writer cannot be created, neither the hook nor the facade is
  installed and the board keeps the ROM console: degraded, never aborted. The
  `console_log_unavailable_total` counter says so.
- Queue, truncation, unavailable and UART-error counters are on `/api/v1/stats`.
- The capture path reaches the log history ring (the WebSocket `logs` channel) only
  through `log_ring::try_global`: `global()` builds the ring through `PsramBox`, which
  logs when PSRAM falls back to internal, so a capture there would re-enter the
  initializer and deadlock.

## No long spinlock holds while the wire is live

- A spinlock held longer than about one tick delays every task that wants it, the
  sniffer staging an arbitration answer included.
- So nothing at runtime walks the kernel or the heap: no `uxTaskGetSystemState`, no
  `heap_caps_get_info`, `heap_caps_get_largest_free_block` or `multi_heap_get_info`.
  Largest-block figures are taken at boot stages only
  (`EspHeapStats::boot_snapshot`) and later snapshots repeat them.
- The task census reads the BSP task registry and asks `uxTaskGetStackHighWaterMark` per
  handle. Threads register as they start and leave as they exit; the kernel's own
  process-lifetime tasks are re-resolved by name on every census; transient tasks
  (`httpd`, `mqtt_task`) publish their own watermark with an instance id and a timestamp,
  so no handle is dereferenced after its task may have died. The table is released
  before the census line is formatted.
- The heap line and the census run once a minute on core 1, away from the interrupt.

## Bus lifetime

- A `BusHost` owns the bus. Dropping it sets a shutdown flag and joins the routing task,
  which drops the subscriber senders it owns; every worker's `recv` then reports
  `Disconnected` and the worker exits.
- A test harness therefore owns its `BusHost` for as long as it publishes.
- `dali2rust-adapters/tests/stack_teardown.rs` counts threads across repeated
  build-and-drop cycles.

## Panics on the device

- The firmware builds `std` with `panic_abort`: a panic resets the chip.
- The flashed image is the dev profile, with debug assertions on in every crate but
  `dali2rust-dali-phy`, so a `debug_assert!` on a device path resets the chip like a
  panic. An outcome the wire can produce is mapped to a defined value, never asserted.
- Lock poisoning needs an unwinding panic, so it cannot happen on the device.
  `PoisonError` recovery helpers run only in host builds, where they keep one panicking
  test thread from cascading into unrelated failures. Poisoning is not a device failure
  mode.
- Code reachable from a client fails soft: fallible spawns, a closed socket on a bad
  frame, an error response on bad input.
