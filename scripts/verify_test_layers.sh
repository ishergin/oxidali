#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

python3 - "$REPO_ROOT" <<'PY'
import os
import re
import sys

repo_root = sys.argv[1]

sleep_allowlist = set()
harness_allowlist = {
    "crates/dali2rust-api/tests/semantic_handler_contracts.rs",
    "crates/dali2rust-dali-runtime/tests/semantic_worker_flow.rs",
    "crates/dali2rust-registry-runtime/tests/support/mod.rs",
    "crates/dali2rust-dali-runtime/tests/discovery_scan_scale.rs",
}

targets = []
for root, _, files in os.walk(os.path.join(repo_root, "crates")):
    if "/tests" not in root:
        continue
    for name in files:
        if name.endswith(".rs"):
            targets.append(os.path.join(root, name))
for root, _, files in os.walk(os.path.join(repo_root, "tests", "dali2rust-bdd", "src")):
    for name in files:
        if name.endswith(".rs"):
            targets.append(os.path.join(root, name))

errors = []

sleep_re = re.compile(r"\bthread::sleep\s*\(")
postcard_re = re.compile(r"\bpostcard::(?:to_|from_)")
temp_fs_re = re.compile(r"\bfn\s+temp_fs\s*\(")
harness_re = re.compile(r"\bstruct\s+\w+(?:Harness|TestStack)\b")
sleep_helper_re = re.compile(r"\bfn\s+sleep_worker\s*\(")

for path in sorted(targets):
    rel = os.path.relpath(path, repo_root)
    with open(path, "r", encoding="utf-8") as fh:
        content = fh.read()

    if sleep_re.search(content) and rel not in sleep_allowlist:
        errors.append(f"forbidden thread::sleep in test file: {rel}")
    if postcard_re.search(content):
        errors.append(f"manual postcard encode/decode in test file: {rel}")
    if temp_fs_re.search(content):
        errors.append(f"duplicate temp_fs helper in test file: {rel}")
    if sleep_helper_re.search(content) and rel not in sleep_allowlist:
        errors.append(f"legacy sleep helper reintroduced: {rel}")
    if harness_re.search(content) and rel not in harness_allowlist:
        errors.append(f"new ad-hoc Harness/TestStack detected: {rel}")

marker_re = re.compile(r"//\s*(sleep-ok|busy-wait-ok):")
src_targets = []
for root, _, files in os.walk(os.path.join(repo_root, "crates")):
    if os.sep + "src" not in root:
        continue
    for name in files:
        if name.endswith(".rs"):
            src_targets.append(os.path.join(root, name))

for path in sorted(src_targets):
    rel = os.path.relpath(path, repo_root)
    with open(path, "r", encoding="utf-8") as fh:
        lines = fh.readlines()
    for idx, line in enumerate(lines):
        if not sleep_re.search(line):
            continue
        prev = lines[idx - 1] if idx > 0 else ""
        if marker_re.search(line) or marker_re.search(prev):
            continue
        errors.append(
            f"thread::sleep without '// sleep-ok:' or '// busy-wait-ok:' "
            f"marker: {rel}:{idx + 1}"
        )

if errors:
    print("verify_test_layers: FAILED", file=sys.stderr)
    for err in errors:
        print(f"- {err}", file=sys.stderr)
    sys.exit(1)

print("verify_test_layers: structural checks OK")
PY

bash "$REPO_ROOT/scripts/verify_duplication.sh"
