#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FEATURES="$ROOT/tests/dali2rust-bdd/features"
REGISTRY_DOC="$ROOT/documentation/product-design/bdd/ids-registry.md"

python3 - "$FEATURES" "$REGISTRY_DOC" <<'PY'
import os
import re
import sys
from collections import Counter

features_root = sys.argv[1]
registry_doc = sys.argv[2]

scenario_re = re.compile(r'^\s*Scenario(?: Outline)?:')
id_re = re.compile(r'@id:([A-Z]+(?:-[A-Z]+)*-[0-9]{3}[a-z]?)\b')
stage_re = re.compile(r'@stage-([A-Z][0-9]{1,2})\b')
allowed_stage_re = re.compile(r'^(F[0-6]|R(?:[1-9]|1[0-6])|I(?:[1-9]|10)|X[1-4])$')
prefix_re = re.compile(r'^([A-Z]+(?:-[A-Z]+)*-)[0-9]{3}[a-z]?$')

registered_prefixes = set()
with open(registry_doc, "r", encoding="utf-8") as fh:
    for line in fh:
        match = re.match(r'^\|\s*`([A-Z]+(?:-[A-Z]+)*-)`\s*\|', line)
        if not match:
            continue
        if "*(removed)*" in line:
            continue
        registered_prefixes.add(match.group(1))

if not registered_prefixes:
    print(
        f"ERROR: no active BDD prefixes parsed from registry {registry_doc}",
        file=sys.stderr,
    )
    sys.exit(1)

feature_files = []
for root, _, files in os.walk(features_root):
    for name in files:
        if name.endswith(".feature"):
            feature_files.append(os.path.join(root, name))
feature_files.sort()

if not feature_files:
    print("ERROR: no executable .feature files found", file=sys.stderr)
    sys.exit(1)

errors = []
ids = []
total_scenarios = 0

for path in feature_files:
    with open(path, "r", encoding="utf-8") as fh:
        lines = fh.readlines()

    feature_line = next(
        (idx for idx, line in enumerate(lines, start=1) if line.lstrip().startswith("Feature:")),
        None,
    )
    if feature_line is None:
        errors.append(f"MISSING Feature: header: {path}")
        continue

    stages = []
    for idx, line in enumerate(lines, start=1):
        for match in stage_re.finditer(line):
            stage = match.group(1)
            stages.append(stage)
            if not allowed_stage_re.match(stage):
                errors.append(f"INVALID @stage tag: {path}:{idx}: {stage}")

    feature_header_stages = []
    for idx in range(1, feature_line):
        for match in stage_re.finditer(lines[idx - 1]):
            feature_header_stages.append((match.group(1), idx))

    if not feature_header_stages:
        errors.append(f"MISSING feature-level @stage tag: {path}")
    elif len(feature_header_stages) > 1:
        locations = ", ".join(
            f"{stage}@{path}:{line}" for stage, line in feature_header_stages
        )
        errors.append(
            "FEATURE must declare exactly one default @stage tag before Feature: "
            f"{locations}"
        )

    for idx, line in enumerate(lines, start=1):
        if not scenario_re.search(line):
            continue
        total_scenarios += 1
        found = False
        for prev in range(max(0, idx - 4), idx - 1):
            if id_re.search(lines[prev]):
                found = True
                break
        if not found:
            errors.append(f"MISSING @id: {path}:{idx}")

    for idx, line in enumerate(lines, start=1):
        for match in id_re.finditer(line):
            ids.append((match.group(1), path, idx))

id_values = [value for value, _, _ in ids]
counts = Counter(id_values)
for value, count in sorted(counts.items()):
    if count > 1:
        locations = [f"{path}:{line}" for ident, path, line in ids if ident == value]
        errors.append(f"DUPLICATE @id {value}: " + ", ".join(locations))

for value, path, line in ids:
    if not id_re.fullmatch(f"@id:{value}"):
        errors.append(f"MALFORMED @id: {path}:{line}: {value}")
    prefix_match = prefix_re.match(value)
    if prefix_match is None:
        errors.append(f"MALFORMED @id prefix: {path}:{line}: {value}")
        continue
    prefix = prefix_match.group(1)
    if prefix not in registered_prefixes:
        errors.append(
            f"UNREGISTERED @id prefix {prefix}: {path}:{line}: {value}"
        )

if errors:
    print("BDD ID verification FAILED", file=sys.stderr)
    for err in errors:
        print(f"- {err}", file=sys.stderr)
    sys.exit(1)

print(f"BDD IDs OK ({len(counts)} unique IDs across {total_scenarios} scenarios)")
PY
