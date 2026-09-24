# ADR-028: Task stacks and the Rust heap live in PSRAM on the XIP image

Status: Accepted
Date: 2026-09-24

## Context

Internal SRAM is the scarce pool ([07](../07-memory-and-cores.md)), and task stacks were
its largest single consumer: every worker's stack was internal for the life of the task,
most of it reserved headroom. The internal minimum does not only fall at boot. An update,
WebSocket clients and bursts of requests take internal SRAM while they last, and what
must be internal at that moment — a new task stack, the httpd task, DMA, TLS — is what
fails.

The rule that kept stacks internal was that a PSRAM stack is behind the cache and a flash
operation turns the cache off. Read against ESP-IDF 5.5.3 it is narrower than that:

- During a cache-off window of a flash operation both cores run tasks with internal
  stacks — the task issuing the operation, whose stack ESP-IDF asserts is internal, and
  the IPC task that stalls the other core. No other task runs, so no other task's stack
  is touched, and an interrupt entered in the window saves its frame on an internal
  stack.
- On the image that executes from PSRAM
  ([ADR-025](ADR-025-phy-interrupt-above-critical-sections.md)), a flash program, erase
  or read over SPI1 takes a lock and leaves the cache on.
- On the ESP32-P4, changing a flash mapping (`esp_mmu_map`, `esp_mmu_unmap`) still turns
  the cache off on both cores, even from PSRAM code, and asserts that the caller's stack
  is internal. `esp_partition_mmap` maps; so do the `esp_ota_*` calls that read `otadata`
  or verify an image.

## Decision

1. **A stack's home is chosen when the thread is spawned** (`dali2rust_bsp::esp_thread::StackHome`):
   internal; PSRAM always (a task that never touches flash); or PSRAM on the XIP image and
   internal on a build without it.
2. **On the XIP image every thread that never maps flash and is not on the wire's timing
   path has its stack in PSRAM**: the state and event workers, the rules worker,
   replication, the confirmation bridge, the WebSocket and MQTT workers, the display,
   supervisors, the census, the IP and SNTP watchers, WebSocket senders, the OTA listener
   and boot hydration.
3. **The httpd task's stack is in PSRAM on the XIP image** (`task_caps`), and the boot
   reserve that kept a contiguous internal block for it is not taken. The HTTP task never
   maps flash: the running OTA slot is read under the flash gate at boot and after every
   update or validation, and requests answer from that reading.
4. **Internal stays internal**: the DALI worker and sniffer and the bus task, which carry
   the wire's timing; `main`; the update and boot-verify threads, which map flash; and
   every ESP-IDF task.
5. **A mapping call on a PSRAM stack is refused by name** in the OTA adapter instead of
   reaching ESP-IDF's assert, which is a reboot.
6. **One knob puts everything back**: `DALI2RUST_STACKS_INTERNAL=1` keeps every
   conditional stack internal, so a rollback or an A/B is one flash.
7. **The Rust heap is counted by placement** in the firmware's global allocator, and the
   bench gates the internal minimum at the end of a session as well as the boot ladder.
8. **On the XIP image the allocator puts every Rust object from 256 B in PSRAM first**
   (`PSRAM_FIRST_FROM_BYTES`); smaller ones stay internal first. The count showed the
   Rust heap holding over 100 KB of internal SRAM, most of it objects of 256 B to 1 KiB.
   Atomics and mutex data move with their objects: the P4 performs atomic operations on
   PSRAM correctly from both cores, which a boot self-test
   (`DALI2RUST_PSRAM_ATOMIC_SELFTEST`) checks on the device. What must be internal is
   allocated by name — the PHY interrupt's state, the flash-write bounce — and C code
   keeps ESP-IDF's placement. `DALI2RUST_RUST_HEAP_INTERNAL=1` restores internal-first.

### Rejected alternatives

- **Lowering `CONFIG_SPIRAM_MALLOC_ALWAYSINTERNAL`** — moves every C allocation below
  16 KiB, drivers' included; the Rust allocator places only Rust objects, and the network
  stack moved by its own mechanism
  ([ADR-029](ADR-029-network-buffers-in-psram-and-a-measured-receive-ring.md)).
- **Pinning PSRAM-stack tasks to core 1**, away from the PHY interrupt — the interrupt's
  code and data stay internal and only its entry frame lands on an interrupted stack,
  through the cache; pinning is held back until the interrupt's timing counters move.
- **Trimming stacks further** — sizes follow the census and its margins; the depth a task
  needs does not change with where its stack lives.

## Consequences

- Most of the internal SRAM that stacks held returns to the pool, and the httpd task no
  longer depends on a contiguous internal block surviving the boot.
- A build without XIP gets every conditional stack back in internal SRAM, reserve
  included, with the ladder that goes with it.
- An interrupt that lands on a PSRAM-stack task writes its entry frame through the cache;
  the late-tick and answer-staging counters are the watch on it.
- Stack budgets are unchanged: the census reads a high-water mark wherever the stack
  lives, and a breach is still answered with a shallower path.
- A new thread that maps flash, or a new mapping call on an existing PSRAM-stack thread,
  is refused at runtime with its name; it spawns with an internal stack.
- Rust objects from 256 B, their atomics and mutex data included, live in PSRAM on the
  XIP image; FreeRTOS objects behind a mutex stay internal because ESP-IDF allocates them.
  A Rust buffer handed to something that needs internal memory — DMA, the PHY interrupt,
  a flash write without a bounce — must be allocated internal by name.
