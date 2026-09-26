# ADR-011: One board — the ESP32-S3 is retired

Status: Accepted
Date: 2026-08-02

## Context

The controller runs on the Waveshare ESP32-P4-ETH: an ESP32-P4NRW32 (dual-core
RISC-V, 32 MB flash, 32 MB PSRAM) with wired Ethernet through the internal EMAC and an
IP101GRI PHY. The P4 has no radio. The ESP32-S3 board that preceded it was retired
rather than kept buildable.

Keeping a second board is not free, and the cost is not the code: every board-shaped
mechanism has to exist twice, and the two copies drift silently. Two sdkconfig
baselines need a sync gate; `[env]` in `.cargo/config.toml` is per workspace, so two
boards push the board knobs into side files every tool must read; the second
architecture's code is compiled by no gate unless a second ESP gate exists; and the
bench flash path must know about both. Each of those is the correct local answer to
"there are two boards", and each disappears when there is one.

## Decision

**The ESP32-P4-ETH is the only board.**

- **Wired only.** Ethernet is the network path. `NetworkLink` stays as the port
  (`EthLink`, and a `NoLink` stand-in when the EMAC does not start), so a future radio
  would arrive as a second implementation, not as a sweep through composition.
- **One of each board-shaped mechanism:** one sdkconfig baseline
  (`sdkconfig.p4.defaults`), one partition table (`partitions-p4.csv`), one target
  triple (`riscv32imafc-esp-espidf`), one BSP board module (`dali2rust-bsp::esp32p4`,
  which holds the pin map), and one ESP gate (`just esp-check`) that compiles the board
  that ships.
- **The board knobs live in `.cargo/config.toml [env]` with `force = true`.** `MCU` and
  `ESP_IDF_SDKCONFIG_DEFAULTS` apply to every build, including `cargo fw` and
  `cargo flash`. `force` matters because Cargo's `[env]` does not override a variable
  the shell already holds, so a stale export would otherwise produce an image for the
  wrong die. `ESP_IDF_SYS_ROOT_CRATE` is deliberately not forced: it names the crate
  being built, not the board.
- **The bench flash path asserts, it does not set.** `hil flash` reads the knobs from
  `.cargo/config.toml`, checks them against its board table and records them in the run
  manifest, so a repointed `[env]` with a stale table fails before a binary reaches the
  chip.

### Rejected alternatives

- **Keep the S3 buildable but untested** — pays the full doubling cost while the second
  half rots unexercised.
- **Keep it and add a gate against drift** — every doubled mechanism already had a gate
  or a promise; the answer to drifting copies is one copy.
- **Archive it on a branch** — git history already holds it; a branch is one more thing
  to keep mergeable.

## Consequences

- Adding a second board means reintroducing the doubling deliberately: the board table
  in `hil flash`, the `force` decision and every mechanism above must change together.
- `scripts/verify_dali_isr_iram.py` parses RISC-V only, which is the only architecture
  there is.
- Build and flashing details are described in
  [10-build-release-and-tooling.md](../10-build-release-and-tooling.md); the memory
  budget of this board is in [07-memory-and-cores.md](../07-memory-and-cores.md).
