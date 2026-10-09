#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FEATURES_DIR="$REPO_ROOT/tests/dali2rust-bdd/features"
STEPS_DIR="$REPO_ROOT/tests/dali2rust-bdd/src/steps"

echo "=== BDD Coverage Verification ==="

bash "$REPO_ROOT/scripts/verify_bdd_tree_policy.sh"
bash "$REPO_ROOT/scripts/verify_bdd_ids.sh"
python3 "$REPO_ROOT/scripts/verify_bdd_stages.py"

python3 - "$REPO_ROOT" "$FEATURES_DIR" "$STEPS_DIR" <<'PY'
import os
import re
import sys

ID_PATTERN = r"[A-Z]+(?:-[A-Z]+)*-[0-9]{3}[a-z]?"
SCENARIO_ID_TAG = re.compile(r"@id:(" + ID_PATTERN + r")")
BARE_ID = re.compile(r"\b(" + ID_PATTERN + r")\b")
NON_SCENARIO_PREFIXES = ("ADR-", "ISSUE-")
STEP_KEYWORDS = {"Given": "given", "When": "when", "Then": "then"}
CONTINUATION_KEYWORDS = ("And", "But", "*")
STEP_ATTRIBUTE = re.compile(
    r'#\[(?P<kind>given|when|then)\(\s*(?:(?P<mode>regex|expr)\s*=\s*)?'
    r'(?:r(?P<hashes>#*)"(?P<raw>(?:[^"]|"(?!(?P=hashes)\s*,?\s*\)))*)"(?P=hashes)'
    r'|"(?P<plain>(?:[^"\\]|\\.)*)")\s*,?\s*\)\]',
    re.S,
)
STEP_ATTRIBUTE_START = re.compile(r"#\[(?:given|when|then)\(")
EXPRESSION_PARAMETERS = (
    ("{int}", r"-?\d+"),
    ("{float}", r"-?\d*\.?\d+"),
    ("{word}", r"\S+"),
    ("{string}", r'"[^"]*"'),
    ("{}", r".*"),
)
RUST_ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "0": "\0"}
OUTLINE_PLACEHOLDER = re.compile(r"<([^<>]+)>")
DOC_STRING_FENCES = ('"""', "```")


def is_scenario_id(ident):
    return not ident.startswith(NON_SCENARIO_PREFIXES)


def files_under(root, suffix):
    found = []
    for directory, _, names in os.walk(root):
        found.extend(os.path.join(directory, name) for name in names if name.endswith(suffix))
    return sorted(found)


def step_definition_files(steps_dir):
    return [path for path in files_under(steps_dir, ".rs") if os.path.basename(path) != "mod.rs"]


def read_lines(path):
    with open(path, encoding="utf-8") as fh:
        return fh.read().splitlines()


def tags_on(line):
    stripped = line.strip()
    return stripped.split() if stripped.startswith("@") else []


def feature_tags(path):
    return [tag for line in read_lines(path) for tag in tags_on(line)]


def executable_scenario_ids(features):
    ids = set()
    for path in features:
        for tag in feature_tags(path):
            match = SCENARIO_ID_TAG.fullmatch(tag)
            if match:
                ids.add(match.group(1))
    return ids


def empty_feature_directories(features_dir):
    errors = []
    for entry in sorted(os.listdir(features_dir)):
        path = os.path.join(features_dir, entry)
        if os.path.isdir(path) and not files_under(path, ".feature"):
            errors.append(f"empty feature directory: {path}")
    return errors


def scenario_ids_using_exempt_prefixes(ids):
    return [
        f"scenario id {ident} uses a prefix step comments may cite freely "
        f"({', '.join(NON_SCENARIO_PREFIXES)}); rename the scenario"
        for ident in sorted(ids)
        if not is_scenario_id(ident)
    ]


def step_comment_ids_without_scenario(step_files, ids):
    errors = []
    for path in step_files:
        for number, line in enumerate(read_lines(path), start=1):
            if not line.lstrip().startswith("//"):
                continue
            for ident in BARE_ID.findall(line):
                if is_scenario_id(ident) and ident not in ids:
                    errors.append(f"step comment names {ident}, which no feature tags: {path}:{number}")
    return errors


def rust_string_value(literal):
    joined = re.sub(r"\\\n\s*", "", literal)
    return re.sub(r'\\(["\\\'nrt0])', lambda m: RUST_ESCAPES.get(m.group(1), m.group(1)), joined)


def expression_regex(expression):
    parts, index = [], 0
    while index < len(expression):
        for token, pattern in EXPRESSION_PARAMETERS:
            if expression.startswith(token, index):
                parts.append(pattern)
                index += len(token)
                break
        else:
            parts.append(re.escape(expression[index]))
            index += 1
    return "^" + "".join(parts) + "$"


def step_matcher(match):
    text = match.group("raw")
    if text is None:
        text = rust_string_value(match.group("plain"))
    if match.group("mode") == "regex":
        return re.compile(text).search
    if match.group("mode") == "expr":
        return re.compile(expression_regex(text)).match
    return lambda value: value == text


def line_of(text, offset):
    return text.count("\n", 0, offset)


def declared_ids(lines, first_line, attribute_lines):
    ids = set()
    index = first_line - 1
    while index >= 0:
        stripped = lines[index].strip()
        if index in attribute_lines or stripped.startswith("#["):
            index -= 1
            continue
        if not stripped.startswith("//"):
            break
        ids.update(ident for ident in BARE_ID.findall(stripped) if is_scenario_id(ident))
        index -= 1
    return ids


def step_definitions(repo_root, step_files):
    steps, unparsed = [], 0
    for path in step_files:
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
        lines = text.splitlines()
        matches = list(STEP_ATTRIBUTE.finditer(text))
        unparsed += len(STEP_ATTRIBUTE_START.findall(text)) - len(matches)
        attribute_lines = {
            number
            for match in matches
            for number in range(line_of(text, match.start()), line_of(text, match.end()) + 1)
        }
        for match in matches:
            try:
                matcher = step_matcher(match)
            except re.error:
                unparsed += 1
                continue
            first_line = line_of(text, match.start())
            steps.append({
                "where": f"{os.path.relpath(path, repo_root)}:{first_line + 1}",
                "kind": match.group("kind"),
                "matches": matcher,
                "declared": declared_ids(lines, first_line, attribute_lines),
                "reached": set(),
            })
    return steps, unparsed


def table_cells(line):
    return [cell.strip() for cell in line.strip("|").split("|")]


def parse_feature(repo_root, path):
    background, scenarios, steps, kind = [], [], None, None
    in_doc, header = False, None
    for number, raw in enumerate(read_lines(path), start=1):
        line = raw.strip()
        if line.startswith(DOC_STRING_FENCES):
            in_doc = not in_doc
            continue
        ids = [m.group(1) for m in map(SCENARIO_ID_TAG.fullmatch, tags_on(line)) if m]
        if in_doc or not line:
            continue
        if ids:
            scenarios.append({"id": ids[0], "steps": [], "rows": []})
            steps, header = scenarios[-1]["steps"], None
            continue
        if line.startswith("Background:"):
            steps = background
            continue
        if line.startswith("Examples:"):
            steps, header = None, []
            continue
        if header is not None and line.startswith("|") and scenarios:
            if header:
                scenarios[-1]["rows"].append(dict(zip(header, table_cells(line))))
            else:
                header = table_cells(line)
            continue
        keyword, _, value = line.partition(" ")
        if keyword in STEP_KEYWORDS:
            kind = STEP_KEYWORDS[keyword]
        elif keyword not in CONTINUATION_KEYWORDS or kind is None:
            continue
        if steps is not None:
            steps.append((f"{os.path.relpath(path, repo_root)}:{number}", kind, value.strip()))
    return background, scenarios


def expanded_steps(background, scenario):
    for row in scenario["rows"] or [{}]:
        for where, kind, text in background + scenario["steps"]:
            yield where, kind, OUTLINE_PLACEHOLDER.sub(lambda m: row.get(m.group(1), m.group(0)), text)


def record_reach(repo_root, features, steps):
    unresolved = set()
    for path in features:
        background, scenarios = parse_feature(repo_root, path)
        for scenario in scenarios:
            for where, kind, text in expanded_steps(background, scenario):
                hits = [step for step in steps if step["kind"] == kind and step["matches"](text)]
                for step in hits:
                    step["reached"].add(scenario["id"])
                if not hits:
                    unresolved.add(f"feature step matches no step pattern: {where}: {text}")
    return sorted(unresolved)


def step_comment_completeness(repo_root, features, step_files):
    steps, unparsed = step_definitions(repo_root, step_files)
    errors = [f"{unparsed} step attribute(s) this check cannot parse"] if unparsed else []
    errors.extend(record_reach(repo_root, features, steps))
    for step in steps:
        missing = sorted(step["reached"] - step["declared"])
        if missing:
            errors.append(f"{step['where']}: the ID list omits {' '.join(missing)}")
    return errors


def run_check(title, errors, sink):
    print(f"--- {title} ---")
    sink.extend(errors)
    print("  FAILED" if errors else "  OK")


def main(repo_root, features_dir, steps_dir):
    features = files_under(features_dir, ".feature")
    step_files = step_definition_files(steps_dir)
    ids = executable_scenario_ids(features)
    errors = []
    run_check("Feature directories are not empty", empty_feature_directories(features_dir), errors)
    run_check("Scenario ids avoid the exempt prefixes", scenario_ids_using_exempt_prefixes(ids), errors)
    run_check("Step comment IDs name executable scenarios", step_comment_ids_without_scenario(step_files, ids), errors)
    run_check("Step comment ID lists are complete", step_comment_completeness(repo_root, features, step_files), errors)
    if errors:
        sys.stdout.flush()
        print("BDD Coverage Verification FAILED", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        sys.exit(1)
    print("=== BDD Coverage Verification PASSED ===")


main(sys.argv[1], sys.argv[2], sys.argv[3])
PY
