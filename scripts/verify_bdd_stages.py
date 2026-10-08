#!/usr/bin/env python3
import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FEATURES = ROOT / "tests" / "dali2rust-bdd" / "features"
STATUS = ROOT / "documentation" / "product-design" / "status.md"
STAGE_TAG = re.compile(r"@stage-([A-Z][0-9]{1,2})")
ANY_STAGE_TAG = re.compile(r"@stage-\S*")
STATUS_ROW = re.compile(r"^\| ([FRIX])(\d{1,2})(?:-[A-Z])?(?:–[FRIX](\d{1,2}))? [^|]*\| ([^|]+?) \|")
DONE = "готово"
KNOWN_STATUSES = (DONE, "частично", "запланировано")
SCENARIO = re.compile(r"^\s*Scenario(?: Outline| Template)?:")


def stage_statuses():
    statuses = {}
    for number, line in enumerate(STATUS.read_text(encoding="utf-8").splitlines(), 1):
        row = STATUS_ROW.match(line)
        if not row:
            continue
        letter, first, last, status = row.groups()
        if status not in KNOWN_STATUSES:
            sys.exit(f"verify_bdd_stages: {STATUS.relative_to(ROOT)}:{number}: unknown status '{status}'")
        for value in range(int(first), int(last or first) + 1):
            statuses.setdefault(f"{letter}{value}", []).append(status)
    if not statuses:
        sys.exit(f"verify_bdd_stages: no stage row parsed from {STATUS.relative_to(ROOT)}")
    return {stage: all(status == DONE for status in rows) for stage, rows in statuses.items()}


def own_stage(tags):
    return next((m.group(1) for m in map(STAGE_TAG.fullmatch, tags) if m), None)


def scenarios(path):
    feature_tags, pending, in_feature = [], [], False
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if line.startswith("@"):
            pending.extend(line.split())
        elif line.startswith("Feature:"):
            feature_tags, pending, in_feature = pending, [], True
        elif in_feature and SCENARIO.match(raw):
            tags = pending + feature_tags
            yield number, own_stage(pending) or own_stage(feature_tags), "@wip" in tags
            pending = []
        elif line and not line.startswith("#"):
            pending = []


def all_scenarios():
    for path in sorted(FEATURES.rglob("*.feature")):
        rel = path.relative_to(ROOT)
        for number, stage, wip in scenarios(path):
            yield f"{rel}:{number}", stage, wip


def malformed_stage_tags():
    for path in sorted(FEATURES.rglob("*.feature")):
        rel = path.relative_to(ROOT)
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for tag in ANY_STAGE_TAG.findall(line):
                if not STAGE_TAG.fullmatch(tag):
                    yield f"{rel}:{number}: {tag} is not a stage row's letter and number"


def check_stage(stage):
    if stage not in stage_statuses():
        print(f"ERROR: {stage} names no stage row in {STATUS.relative_to(ROOT)}", file=sys.stderr)
        return 1
    wip = [where for where, own, is_wip in all_scenarios() if own == stage and is_wip]
    for where in wip:
        print(f"  @wip: {where}", file=sys.stderr)
    if wip:
        print(f"ERROR: @stage-{stage} has {len(wip)} @wip scenario(s) — stage not clean", file=sys.stderr)
        return 1
    print(f"Stage {stage} is clean (no @wip)")
    return 0


def check_all():
    done = stage_statuses()
    errors = list(malformed_stage_tags())
    for where, stage, wip in all_scenarios():
        if stage not in done:
            errors.append(f"{where}: @stage-{stage} names no stage row in {STATUS.relative_to(ROOT)}")
        elif wip and done[stage]:
            errors.append(f"{where}: @wip in @stage-{stage}, which status.md marks {DONE}")
    for error in errors:
        print(f"  {error}", file=sys.stderr)
    if errors:
        print(f"verify_bdd_stages: {len(errors)} problem(s)", file=sys.stderr)
        return 1
    finished = sorted(stage for stage, is_done in done.items() if is_done)
    print(f"verify_bdd_stages: OK (no @wip in a finished stage: {' '.join(finished)})")
    return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--stage")
    args = parser.parse_args()
    return check_stage(args.stage) if args.stage else check_all()


if __name__ == "__main__":
    sys.exit(main())
