#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FEATURES_DIR="$REPO_ROOT/tests/dali2rust-bdd/features"

echo "=== BDD Tree Policy Verification ==="

python3 - "$FEATURES_DIR" <<'PY'
import os
import sys

features_dir = sys.argv[1]

allowed_top_level = {
    "adapters",
    "commissioning",
    "contracts",
    "diagnostic",
    "groups",
    "hcl",
    "input_devices",
    "rules",
    "mqtt_home_assistant",
    "operations",
    "persistence",
    "physical_devices",
    "poller",
    "redundancy",
    "config_transfer",
    "policies",
    "scenes",
    "settings_home_assistant",
    "settings_dali",
    "settings_poller",
    "stats",
    "system",
    "virtual_lamps",
    "web_ui",
    "websocket",
}

forbidden_top_level = {
    "dali",
    "bus",
    "display",
    "registry",
}

errors = []

if not os.path.isdir(features_dir):
    errors.append(f"missing features directory: {features_dir}")
else:
    for entry in sorted(os.listdir(features_dir)):
        path = os.path.join(features_dir, entry)
        if not os.path.isdir(path):
            continue
        if entry in forbidden_top_level:
            errors.append(
                f"forbidden legacy top-level feature directory: {path} "
                f"(migrate to resource-first tree; see tests/dali2rust-bdd/README.md)"
            )
            continue
        if entry not in allowed_top_level:
            errors.append(
                f"unknown top-level feature directory: {path} "
                f"(allowed: {', '.join(sorted(allowed_top_level))})"
            )

legacy_dali = os.path.join(features_dir, "dali")
if os.path.isdir(legacy_dali):
    remaining = []
    for root, _, files in os.walk(legacy_dali):
        for name in files:
            if name.endswith(".feature"):
                remaining.append(os.path.join(root, name))
    if remaining:
        errors.append(
            "legacy features/dali/ still contains executable features: "
            + ", ".join(sorted(remaining))
        )
    else:
        errors.append(
            "legacy features/dali/ directory still exists (remove empty directory)"
        )

if errors:
    print("BDD Tree Policy Verification FAILED", file=sys.stderr)
    for err in errors:
        print(f"- {err}", file=sys.stderr)
    sys.exit(1)

print("=== BDD Tree Policy Verification PASSED ===")
PY
