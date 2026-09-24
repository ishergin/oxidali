# ADR-025: The PHY interrupt runs above the critical sections, and no flash operation stalls a task

Status: Accepted
Date: 2026-09-22

## Context

The DALI PHY is a 104 µs GPTimer tick ([ADR-003](ADR-003-isr-owned-phy.md)). At the
highest level the gptimer driver offers, three delays on it come from one fact about
interrupt levels on the ESP32-P4:

- **Critical sections.** `portENTER_CRITICAL` raises the CLIC threshold to 3, and the
  gptimer driver allocates its alarm at level 3 at most. Every kernel or heap critical
  section on the interrupt's core therefore masks the tick.
- **Flash writes on the issuing core.** ESP-IDF's IPC stall of the other CPU holds the same
  threshold-3 mask across the whole cache-off window of a program or erase, so
  `CONFIG_GPTIMER_ISR_CACHE_SAFE` buys nothing on the core that writes.
- **The stalled core.** The IPC handler on the other core runs with interrupts disabled
  outright, so no interrupt level helps there, and with the cache off no task that
  executes from flash runs anywhere. That stall is what delays the sniffer task, which
  stages arbitration answers against a Table 20 deadline.

## Decision

1. **The alarm is a CPU interrupt of our own at CLIC level 5**, above the
   critical-section threshold and above ESP-IDF's own level-4 IPC interrupt. The gptimer
   driver keeps the timer and is never asked for an interrupt. ESP-IDF refuses a handler
   above level 3 — an Xtensa-era rule; on the P4's CLIC every level enters through the
   same dispatcher — so the source is allocated with a NULL handler and ours is installed
   in its place, doing the driver's two register writes itself (mechanism:
   [08](../08-dali-phy-and-transport.md)).
2. **Composition fails by name if an assumption of that path does not hold** (the checks
   are in 08): a clear that does not clear would re-enter level 5 for ever on that core.
3. **The driver path stays compiled as the rollback** (`DALI2RUST_PHY_ISR_LEVEL=3`), and
   `hil flash` records the knob. The gear emulator stays on the driver path.
4. **The image executes from PSRAM** (`CONFIG_SPIRAM_XIP_FROM_PSRAM`). With code and
   read-only data copied to PSRAM at boot, a flash program or erase takes the SPI1 lock
   and nothing else: no cache-off window, no stalled core. What that retires and what
   replaces it (the wire-gap gate, cache-mapped slice reads) is in 08. The IRAM rule for
   the interrupt stays, for latency and for any build without XIP.
5. **One gate for flash access** (`dali2rust_platform::flash_gate`), because ESP-IDF
   5.5.3 reads `otadata` through the cache with no lock, and under XIP a cached read
   meeting an erase hangs both cores. The OTA slot is written sequentially rather than
   erased whole up front, which would hold the gate for the whole image. Who takes the
   gate and how is in 08.
6. **Delivery** of an image that changes startup behaviour goes to the other OTA slot, so
   a board that never marks itself valid rolls back
   ([ADR-024](ADR-024-ota-over-ethernet.md)); a wired flash is the fallback.

### Rejected alternatives

- **Level 4** — enough above critical sections, but no margin over what ESP-IDF places at
  level 4.
- **`CONFIG_SPI_FLASH_AUTO_SUSPEND`** — chip-dependent: a flash part outside ESP-IDF's
  table fails an assert at startup, a boot loop. The firmware prints the flash JEDEC id at
  boot so the option can be decided from a number.
- **Raising the sniffer task's priority** — with XIP there is no task latency left for it
  to recover, and a task above the worker pool that stops parking owns a core.
- **RMT capture** — would remove only the receive half of the problem and none of the
  transmit half; it stays deferred ([roadmap](../../product-design/roadmap.md)).

## Consequences

- **The handler has no right to fault.** A fault at level 5 is a reset at best, and the
  gptimer driver's own IRAM check does not cover a handler it did not install; what
  guards it is [ADR-010](ADR-010-isr-crate-and-profile-exception.md)'s crate boundary and
  the linked-binary gate.
- **The interrupt can run inside a critical section of its own core**, so nothing it
  touches may rely on one; the rule is 08's.
- **One interrupt frame deeper on every task stack.** The tick can enter inside a
  critical section, and the RISC-V entry saves its frame on the interrupted task's stack,
  so stack budgets include one interrupt frame on top of each task's own depth. The ISR
  stack gains one nesting level.
- **Boot copies the image to PSRAM** before `app_main`, which costs boot time and PSRAM,
  not internal SRAM.
