# 08 DALI PHY and transport

How frames reach the DALI wire and come back: the timer interrupt that owns the PHY, the
crate it lives in, the task↔interrupt boundary, the transmit, receive and answer paths,
and why a flash write does not stall any of it.

Scope: bit timing and wire mechanics. Which frames are sent and how answers are read →
[09](09-dali-protocol-rules.md); tasks and spinlock rules → [02](02-runtime-and-threading.md);
stacks and memory → [07](07-memory-and-cores.md); the slice store →
[06](06-registry-and-persistence.md).

## Ownership

- A 104 µs GPTimer alarm (`PHY_TICK_US`) runs `PhyIsrCore::tick`, and only the interrupt
  mutates the PHY state machine (`DaliBitbangPhy`). Task code never calls its methods
  ([ADR-003](decisions/ADR-003-isr-owned-phy.md)).
- Task and interrupt share ISR-safe primitives only: the command cell
  (`AtomicCommandCell`), the answer cell (`AtomicAnswerCell`), SPSC rings (session events,
  sniffer captures, queued frames) and atomics (idle ticks, frame-active flag, counters).
- The interrupt never allocates, logs, blocks, notifies a task or touches the bus or
  HTTP; consumers poll the rings.
- A bus edge is one volatile write to `GPIO_OUT_W1TS`/`W1TC`, computed at init.
- The Pico-DALI2 inverts TX: a high pad drives the bus dominant. The inversion lives in
  the GPIO implementation (`RegisterGpio`, any `BitbangHal`), never in the FSM. RX is
  configured floating: the adapter drives both levels, and an internal pull-up would lift
  the dominant low toward the VIL margin. GPIO18 (header pin 4) is the adapter's
  high-voltage receive tap and is never configured or driven.

## The interrupt

- The alarm is a CLIC level-5 CPU interrupt of our own: `esp_intr_alloc` with a NULL
  handler (ESP-IDF allocates above level 3 no other way) plus `intr_handler_set`. The
  gptimer driver keeps the timer and allocates no interrupt; the handler clears
  `INT_CLR` and sets `ALARM_EN` again itself (`dali2rust-dali-phy::timer_alarm`)
  ([ADR-025](decisions/ADR-025-phy-interrupt-above-critical-sections.md)).
- Composition checks at boot that the driver's alarm configuration is visible in the
  timer the handler writes, proves the two register writes from task context before
  enabling, reads the installed level back, and sees the tick counter advance within
  5 ms of enabling; any failure fails composition by name.
- Level 5 is above every critical section and above ESP-IDF's level-4 IPC interrupt, so
  the tick may run inside a critical section of its own core. Nothing it touches may be
  protected by one: atomics, SPSC cells, GPIO set/clear and the timer's registers only.
- `DALI2RUST_PHY_ISR_LEVEL=3` rebuilds the driver-owned interrupt
  (`dali_phy_alarm_isr`) as the rollback path; the boot log names the level in use.
- The raw path owns timer group 0, timer 0, and read-modify-writes `TIMG0 INT_ENA`
  without a lock (its other bits are timer 1 and the task watchdog), so nothing else in
  the image creates a gptimer or reconfigures the task watchdog at runtime.

## The crate boundary

- Everything the interrupt executes lives in `dali2rust-dali-phy`, and nothing else does
  ([ADR-010](decisions/ADR-010-isr-crate-and-profile-exception.md)). The crate is
  `no_std`, denies the panic, indexing and arithmetic lints, and builds with
  `debug-assertions = false` because those checks are calls into flash; its host tests
  keep them, and `lib.rs` asserts both at compile time. `overflow-checks` stay on, so a
  plain `+` emits a panic call the gate then finds.
- A per-package profile applies where code is instantiated, so nothing ISR-reachable is
  generic over a type defined outside the crate, and no ISR-reachable `pub fn` is
  `#[inline]`.
- What no profile removes stays out of the source: `copy_from_slice`, indexing
  (`get` + `let … else` instead), unmarked arithmetic (`wrapping_add`,
  `saturating_sub`), `esp_idf_svc::hal::cpu::core()` (the hart id is read from the CSR).
  A many-arm `match` can lower to a lookup table in `.flash.rodata`, so result reports
  are `if` chains.
- Interrupt code carries `link_section = ".iram1.dali_phy"`, and
  `scripts/verify_dali_isr_iram.py` proves the result on the linked binary: every CALL,
  LOAD or ADDRESS reference out of the interrupt's IRAM text must land in IRAM, internal
  RAM or ROM, every linked entry point must sit in IRAM, and a run that matched no
  interrupt symbol fails rather than passes. No source review or `cargo check` can see
  these calls; `hil flash` refuses a binary that fails.
- The gate finds interrupt code by the crate name in the mangled symbol, so interrupt
  functions keep Rust mangling (no `#[no_mangle]`): an unmangled one escapes it. A call
  through a function pointer is invisible to it too, so whatever the late-tick probe
  installs as its clock must be IRAM-resident, checked with `nm` on the linked image.
- Decoding stays out of the interrupt: `dali2rust-dali-codec` (Manchester, best-phase
  search) runs in task context with its checks on.

## Transmit

- The task encodes a frame into half-bits and submits it through the command cell with
  the settling it needs (`min_idle_ticks`); the interrupt arms it on the tick the idle
  counter crosses that gate. The counter is preloaded with the line's high run when a
  frame ends, so it measures from the last rising edge, as Tables 20 and 22 do.
- Arming and the opening edge are different ticks: the first level is written
  `TX_ARM_LEAD_TICKS` (3) after the arming tick, whose work would otherwise delay the
  opening edge and shorten the start bit. Work on the transmit-start path is
  timing-critical; move it off that tick rather than make it faster.
- The lead listens: a dominant level during it means another master took the slot, and
  the frame steps back unwritten (`TxPollResult::Yielded`) and re-arms from the interrupt
  once the bus is idle. A line inside its start-bit debounce counts as busy.
- A held or yielded frame is voided when another master's forward frame completes first
  — that command spends the `ENABLE DEVICE TYPE` the frame may depend on (102 §11.7.14)
  and may have rewritten its DTR — and is reported that tick as `SessionEvent::TxVoided`
  (→ `BusBusy`).
- A collision is detected while we drive recessive; the interrupt drives the Table 25
  break, then waits out the recovery time with the line released before it returns to
  idle. The controller retries a collision with no delay of its own, through the ordinary
  settling gate (the reduced t_RECOVER restart is a
  [conformance gap](../reference/iec62386-conformance-gaps.md)).
- A cancel (an exchange the task abandons) drains the held frame, the queued tail and the
  command cell: a frame left in the cell on a busy bus would go out later, outside its
  transaction. A foreign reception drains the held frame and the tail but never the cell,
  because a frame submitted during a foreign frame is held and goes out after it. Neither
  touches the answer cell.
- The cancel-request word gates new submissions: the interrupt reads it, runs the
  discard, clears it and only then acknowledges — clearing first would let a newer
  submission be erased. A cancel whose acknowledgement the task did not see keeps
  exchange admission closed until the interrupt has applied it, and the verdict already
  decided (`BusBusy`, `Collision`) is reported rather than replaced by a transport error.
- Each exchange opens a new epoch and drains the session ring at its start, so a late
  event of a timed-out exchange is never read as the current outcome. A session event
  carries the epoch its frame was armed under (read at arming, never at push), and
  production code submits with the exchange id it holds (`submit_tx_for`,
  `queue_tx_for`), never with `current_exchange()` read back.
- The interrupt takes the batch queue before the single-frame cell, so a single frame
  never lands inside a queued transaction's tail.
- A unit whose frames are all known up front can be handed over whole
  (`DaliTransport::exchange_transaction`): all or nothing, a foreign forward frame voids
  the tail, and the caller re-runs the unit. The controller does not use it yet.
- Task-side waits sleep the intervals in which nothing can complete — the frame's own
  time on the wire — instead of polling through them.

## Receive

- The interrupt oversamples the line; a capture completes after 16 recessive ticks.
- A capture is routed by its length, never by what the session waits for (101 §7.4.4: a
  backward frame is 8 bits). Below `FORWARD_LENGTH_MIN_SAMPLES` it is a backward frame or
  a §8.2.5 violation, and goes to the session if one waits, else to the sniffer. From that
  length up it is a forward frame: it goes to the sniffer, voids a held tail, and reaches
  an open session only as a `ForeignForward` marker.
- The backward window closes at Table 20's 13,4 ms, judged from the interrupt's own
  pre-idle reading; a later frame is foreign. The task's polling deadline is longer and
  separate.
- The `dali-sniff` task drains captures, decodes, logs and publishes. It polls every
  10 ms, and every 1 ms while the arbitration answer table is armed.

## Answering inside Table 20

- The task decides, the interrupt transmits. The sniffer decodes a capture, looks it up in
  the reflex table (`dali2rust-platform::arbitration`, filled by the redundancy
  supervisor) and stages the answer in the answer cell under the capture's bus epoch.
  The interrupt sends it on the tick the idle counter reaches the Table 20 target
  (`answer_gate`), ahead of any forward frame of ours, only while no later bus event has
  moved the epoch, and discards it once the window has passed.
- The sniffer stages the answer before it logs, forwards or dumps the capture;
  everything after staging is diagnostic.
- The window constants live in `dali-phy::backward_window` beside `TX_ARM_LEAD_TICKS`,
  bounded at compile time; `tools/dali-gear-sim` keeps copies of its own.

## Transports

- `EspIdfDaliTransport` is the interrupt-backed one. `MockDaliTransport` scripts outcomes
  for tests and models order, not timing. `SimDaliTransport` runs the gear model for the
  dev server and implements the observed-frame seam.
- 24-bit frames are a capability of the one transport (`exchange_frame24`); the trait's
  default refuses, and a Part 103 operation on such a transport fails with
  `transport_unsupported_24bit`. `honours_settle()` says whether `min_idle_us` is
  enforced rather than dropped.

## Wire counters and the late-tick probe

- Each tick books occupancy — total, active (not idle: ours, another master's or a
  collision) and ours. The percentage is computed once, in task context, by
  `dali2rust_platform::dali::WireLoadWindow` over a one-second window and published as
  `load_permille` / `load_own_permille`; every surface reads that gauge.
  `wire_ticks_total == 0` means not measured (a host transport), not a quiet bus;
  `SimDaliTransport` charges nominal ticks per exchange.
- A low line for 45 ms is bus power down and for 550 ms system failure (101 §4.11
  Table 4); the interrupt flags both, counts entries, and a down bus releases the
  frame-active flag.
- The interrupt times its own ticks with an IRAM µs clock. A gap over two periods
  (`LATE_TICK_US`) means alarms coalesced while interrupts were masked; it records the
  task it found running and, at nesting depth one, the displaced `mepc` and `ra`. The
  sniffer prints them; `addr2line` on the image names the holder.
- Per-tick work added to the interrupt costs timing that no host test can measure.

## Flash does not stall the wire

- The image executes from PSRAM (`CONFIG_SPIRAM_XIP_FROM_PSRAM`), so a flash program or
  erase stalls neither core
  ([ADR-025](decisions/ADR-025-phy-interrupt-above-critical-sections.md)).
- Changing a flash mapping still does: on the ESP32-P4, `esp_mmu_map` and `esp_mmu_unmap`
  turn the cache off on both cores for the change, even under XIP. Everything that maps —
  `esp_partition_mmap`, the `otadata` reads, `esp_ota_end`'s image check — is short,
  gated, and runs only on an internal stack ([07](07-memory-and-cores.md) §Placement).
- Every flash program or erase and every `otadata` read (`esp_ota_get_*_partition`,
  `mark_app_valid`, the state check in `esp_ota_begin`) holds
  `dali2rust_platform::flash_gate` (why: ADR-025). The running slot is read under the gate
  at boot and after every update or validation, and every other caller — the HTTP task
  included — answers from that reading; with none taken yet, the query waits for the gate
  rather than answer a default, because the boot verifier reads `pending_verify` there. Other writers (registry flush, replication
  import) defer while `firmware_write_open()`; the update raises that flag before it takes
  the gate and lowers it only after `esp_ota_end` and the boot selection, whether they
  succeeded or not. The update slot is opened for sequential writes, so no hold outlasts
  one firmware chunk. A new flash writer or `esp_ota_get_*_partition` caller takes the
  gate.
- NVS is never initialised; initialising it would make lwIP's last-IP store
  (`CONFIG_LWIP_DHCP_RESTORE_LAST_IP`) an ungated writer.
- Slice reads go through the flash driver into DRAM (a 1 KiB DRAM bounce for a PSRAM
  destination), never through a cache mapping that nothing would order against an erase;
  writes go through a 1 KiB DRAM bounce.
- A build without XIP maps slice reads through the cache instead and holds each program
  or erase for a quiet gap on the wire (`await_wire_gap`, bounded at 50 ms); on the
  shipping image that gate is compiled out.

## What the firmware cannot measure

The controller's edge and its observation of that edge come from one clock and one
interrupt, so a stall moves both: the firmware cannot judge its own transmit timing. That
takes an independent receiver; its design and measurement method are in
[`tools/dali-arbiter`](../../tools/dali-arbiter/README.md). No such receiver is installed.
