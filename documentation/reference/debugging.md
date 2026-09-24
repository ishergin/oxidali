# Debugging on-device crashes (ESP32-P4)

Triage runbook for panics, stack overflows and core dumps on the ESP32-P4
(RISC-V, dual-core). Host-side debugging is ordinary Rust and out of scope.

**Scope:** the crash-evidence pipeline and how to read it. Memory limits live
in [`07-memory-and-cores.md`](../architecture/07-memory-and-cores.md); the
serial channel and flashing live in [`tools/hil/README.md`](../../tools/hil/README.md).

## What the build arms

Set in `sdkconfig.p4.defaults`; keep them on, they are the whole post-mortem:

- `CONFIG_FREERTOS_WATCHPOINT_END_OF_STACK` — a stack overflow hits a
  watchpoint that names the overflowing task.
- `CONFIG_ESP_COREDUMP_ENABLE_TO_UART` + `CONFIG_ESP_COREDUMP_DATA_FORMAT_ELF` —
  on panic the chip prints an ELF core dump as base64 to the UART.
- `CONFIG_ESP_PANIC_HANDLER_IRAM` — the panic handler runs with the flash
  cache off.
- `panic = abort` — every Rust panic becomes an abort and a core dump.

## Where the evidence lands

- The persistent HIL serial monitor log, `tools/hil/state/persist/serial.log`
  (the port is reached through the Wiren Board bridge; see the HIL README).
- The ELF to symbolize against is `target/riscv32imafc-esp-espidf/debug/dali2rust`.
  It must come from the **same build** that crashed. The version string
  printed at boot (`0.1.<count>+<sha>`) tells you which commit that was.

## Decode a core dump

```bash
python3 tools/debug/coredump_decode.py --list     # dumps found in the serial log
python3 tools/debug/coredump_decode.py            # decode the most recent one
python3 tools/debug/coredump_decode.py --index 0 --out crash.elf
```

The script prints the crashed task, every task's TCB and PC, and writes the
ELF core. Each line between the `CORE DUMP START/END` markers is an
independent base64 block, and the crashed task's note comes first.

## Read a register dump

For a panic without a usable dump, the register block is enough:

- `MEPC` is the faulting instruction, `RA` the caller, `MCAUSE` the exception
  class, `MTVAL` the offending address or instruction.
- Symbolize with the ESP RISC-V toolchain:
  `riscv32-esp-elf-addr2line -pfiaC -e <elf> <MEPC> <RA>`.
- Disassemble around the PC before forming a theory:
  `riscv32-esp-elf-objdump -d --start-address=<pc-0x40> --stop-address=<pc+0x40> <elf>`.

## Method

- A dump can describe the panic path failing rather than the defect. Read the
  instructions at the reported PC before trusting the signature.
- A stack-overflow watchpoint names the task. The fix is the depth, not a
  bigger stack: task stacks are internal SRAM, which is the scarce pool.
- A dump whose task pointer is stale (a freed and reused TCB) points at
  whatever took its place. Check that the handle was still alive.

## Log markers

| Marker | Meaning |
| --- | --- |
| `CORE DUMP START` | A dump follows; feed the log to `coredump_decode.py`. |
| `heap: free=… largest_free_block=…@boot min_free_ever=…` | Heartbeat. `largest_free_block` is sampled at boot only; watch `min_free_ever`. |
| `internal: free=… largest=… min_ever=…` | Internal-SRAM heartbeat, the pool that runs out first. |
| `task stack hwm` | Per-task stack census, gated by `tools/hil/stack_budget.txt`. |
| `alloc-failure probe armed` / `alloc failures: count=…` | Allocation-failure hook. Any `alloc failures` line is a signal; `last_caps` says which pool refused. |
| `HTTP slow/failed` | Per-URI timing and heap diagnostics for slow or failed responses. |
| `registry flush: slow` | A persistence flush exceeded its budget. |
| `DALI ISR late ticks` | The PHY interrupt found a gap between alarms; names the task and return address that held it off. |
| `registry persistence DISABLED` | Built with `DALI2RUST_PERSIST_DISABLE=1`; diagnosis builds only. |
| `heap integrity sweeps armed` / `heap integrity check FAILED` | Built with `DALI2RUST_HEAP_CHECK_S`; on corruption it panics so the dump lands near the event. |
