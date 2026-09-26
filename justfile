default_host := "aarch64-apple-darwin"

host_crates := `sed 's/#.*//' scripts/host_crates.txt | tr -d ' \t' | grep . | sed 's/^/-p /' | tr '\n' ' '`

[doc("Type-check every host crate, test targets included.")]
check:
    cargo check --target {{default_host}} --all-targets {{host_crates}}

[doc("Run the host tests (all targets, no doc-tests).")]
test:
    cargo test --target {{default_host}} --all-targets {{host_crates}}

[doc("Delete .o files older than DAYS from target/*/deps.")]
gc DAYS='7':
    #!/usr/bin/env bash
    set -euo pipefail
    for d in target/*/debug/deps target/*/release/deps; do
      [ -d "$d" ] || continue
      n=$(find "$d" -name '*.o' -mtime +{{DAYS}} | wc -l | tr -d ' ')
      echo "$d: sweeping $n stale .o (older than {{DAYS}} d)"
      find "$d" -name '*.o' -mtime +{{DAYS}} -delete
    done
    just gc-status

[doc("Count entries in each target/*/debug/deps; past 50k, test launches slow down.")]
gc-status:
    #!/usr/bin/env bash
    for d in target/*/debug/deps; do
      [ -d "$d" ] || continue
      n=$(ls "$d" | wc -l | tr -d ' ')
      printf '%s: %s entries, %s\n' "$d" "$n" "$(du -sh "$d" 2>/dev/null | cut -f1)"
      [ "$n" -gt 50000 ] && echo "  ^ past 50k — run \`just gc\`; launches out of this directory get slow long before the disk fills"
    done
    exit 0

[doc("Inner loop: check every host crate, test the named crates and BDD.")]
quick +CRATES:
    cargo check --target {{default_host}} --all-targets {{host_crates}}
    cargo test --target {{default_host}} --all-targets {{ prepend('-p ', CRATES) }}
    cargo test -p dali2rust-bdd --target {{default_host}} --test bdd

[doc("Verify BDD IDs and coverage, then run the scenarios.")]
bdd-check:
    @bash scripts/verify_bdd_ids.sh
    @bash scripts/verify_bdd_coverage.sh
    cargo test --target {{default_host}} -p dali2rust-bdd --test bdd

bdd:
    cargo test --target {{default_host}} -p dali2rust-bdd --test bdd

[doc("Run only the scenarios of one stage.")]
bdd-stage STAGE:
    cargo test --target {{default_host}} -p dali2rust-bdd --test bdd -- "{{STAGE}}"

[doc("Fail if a stage still has @wip scenarios.")]
check-stage-clean STAGE:
    @if grep -r "@stage-{{STAGE}}" tests/dali2rust-bdd/features/ | grep -q "@wip"; then \
        echo "ERROR: @stage-{{STAGE}} has @wip scenarios — stage not clean"; \
        exit 1; \
    fi
    @echo "Stage {{STAGE}} is clean (no @wip)"

[doc("Format the workspace by hand; rustfmt is not a merge gate.")]
fmt:
    cargo fmt --all

clippy:
    cargo clippy --target {{default_host}} --all-targets {{host_crates}} -- -D warnings -D clippy::allow_attributes_without_reason -D clippy::undocumented_unsafe_blocks

[doc("Advisory pedantic clippy over the host crates, minus the allowlist.")]
clippy-pedantic-advisory:
    bash scripts/run_clippy_pedantic_advisory.sh

[doc("Run every merge-gate verification script.")]
verify:
    bash scripts/verify_contracts_codegen.sh
    python3 scripts/verify_docs.py
    python3 scripts/verify_issue_ids.py
    bash scripts/verify_bdd_ids.sh
    bash scripts/verify_bdd_coverage.sh
    bash scripts/verify_bdd_layers.sh
    bash scripts/verify_no_bdd_production_hooks.sh
    bash scripts/verify_runtime_boundaries.sh
    bash scripts/verify_test_layers.sh
    bash scripts/verify_fixed_bus_guardrails.sh
    python3 scripts/verify_comments.py
    bash scripts/verify_web_assets.sh
    python3 scripts/verify_web_classes_styled.py
    python3 scripts/verify_design_vocabulary.py
    bash scripts/verify_ui_follows_design.sh
    bash scripts/verify_fn_length.sh
    python3 scripts/verify_fn_length_esp.py
    python3 scripts/verify_counter_surface.py
    python3 scripts/verify_read_surface.py
    python3 scripts/verify_dali_isr_iram.py

ci: clippy test bdd verify esp-check gear-sim-check

[doc("Maintainer sync state: the embedded UI bundle and the pushed design cards.")]
verify-release:
    bash scripts/verify_web_mirror_fresh.sh
    bash scripts/verify_design_system_pushed.sh

contracts-check:
    bash scripts/verify_contracts_codegen.sh

[doc("Type-check the firmware for the ESP32-P4 target.")]
esp-check:
    cargo check -p dali2rust-firmware --target {{p4_target}}

[doc("Type-check the gear emulator against the crates it shares.")]
gear-sim-check:
    cd tools/dali-gear-sim && cargo check --target riscv32imac-esp-espidf

[doc("Build the gear emulator and check its interrupt reaches no flash.")]
gear-sim-isr-iram-check:
    cd tools/dali-gear-sim && cargo build
    python3 scripts/verify_dali_isr_iram.py --require-tools \
        tools/dali-gear-sim/target/riscv32imac-esp-espidf/debug/dali-gear-sim

p4_target := "riscv32imafc-esp-espidf"

[doc("Build the firmware image for the ESP32-P4.")]
p4-fw-build:
    cargo build -p dali2rust-firmware --bin dali2rust --target {{p4_target}}

[doc("Build the firmware and check the DALI PHY interrupt reaches no flash.")]
p4-isr-iram-check: p4-fw-build
    python3 scripts/verify_dali_isr_iram.py --require-tools target/{{p4_target}}/debug/dali2rust

p4-fw-flash PORT="/dev/cu.usbmodem5B901574541": p4-fw-build
    espflash flash --port {{PORT}} \
        --bootloader target/{{p4_target}}/debug/bootloader.bin \
        --partition-table partitions-p4.csv \
        target/{{p4_target}}/debug/dali2rust

hil-preflight:
    cd tools/hil && .venv/bin/python3 -m hil.cli preflight

hil-smoke:
    cd tools/hil && .venv/bin/python3 -m pytest -m smoke

hil-default:
    cd tools/hil && .venv/bin/python3 -m pytest

hil-slow:
    cd tools/hil && .venv/bin/python3 -m pytest -m slow
