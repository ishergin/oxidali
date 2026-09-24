# ADR-010: The interrupt boundary is a crate boundary

Status: Accepted, amended by ADR-025
Date: 2026-08-02

## Context

[ADR-003](ADR-003-isr-owned-phy.md) settles who may touch the DALI PHY state machine.
This ADR settles what the compiler may emit into it. The interrupt must keep running
while the flash cache is off (`CONFIG_GPTIMER_ISR_CACHE_SAFE`), so every instruction it
executes must be in IRAM and every call it makes must stay there.

`link_section` places a function in IRAM but says nothing about where it calls. The dev
profile inserts calls that appear in no source file: `ptr::add` and `write_volatile`
precondition checks, raw-pointer null and alignment checks, bounds and overflow panics,
and out-of-line `libcore` helpers. At `opt-level = "z"` those checks can keep even the
GPIO store that writes a bus edge out of line, in flash. Two facts decide the shape:

- Some of the checks come from MIR passes on any raw-pointer dereference and cannot be
  avoided in source; only `debug-assertions = false` removes them.
- A per-package Cargo profile applies at the **instantiation site**: generic or inlined
  code is compiled under the flags of the crate that instantiates it.

## Decision

1. **Everything the DALI PHY interrupt executes lives in `dali2rust-dali-phy`, and
   nothing else does.** The crate is `#![no_std]`, so "the interrupt never allocates,
   logs or blocks" is a compiler error rather than a review rule.
2. **That crate alone gives up `debug-assertions` in firmware** — the one switch that
   removes the MIR-inserted checks — and keeps them in its tests.
3. **`overflow-checks` stay on and `opt-level` is not overridden.** A future plain `+`
   still emits a panic path the gate catches, and the transmit timing is calibrated
   against current codegen, which does not belong in a cache-safety change.
4. **What the profile cannot reach becomes a source rule of the crate**: the two
   instantiation-site rules (no ISR-reachable code generic over an outside type, no
   ISR-reachable `#[inline] pub fn`) and the constructs that emit a flash call whatever
   the profile. Breaking any of them silently brings the flash calls back; the list is
   in [08](../08-dali-phy-and-transport.md).
5. **The linked binary is the proof**, read by `scripts/verify_dali_isr_iram.py`. Source
   review and `cargo check` are not substitutes: neither reaches codegen. What the gate
   checks is in 08, where it runs in [10](../10-build-release-and-tooling.md).

## Consequences

- Task-side code inside the PHY crate (the task halves of the cells and rings) runs
  without debug checks in firmware. The test profile restores them for the host tests
  that cover it; the reduction in firmware is real and accepted.
- A per-package build profile is a precedent in this workspace. It is scoped to one crate
  whose purpose is to be that scope, and it is checked on the artifact; any further use
  must meet the same two conditions.
- `just ci` does not link the firmware, so the hard gate is `hil flash`, which builds
  anyway. The gear emulator's binary is checked with the same script.
- `persist_commit_overlap` counts persistence commits that land while a DALI frame is on
  the wire, so a clean soak can be told apart from one that never exercised the hazard.
