#!/usr/bin/env python3

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BOUNDED = ROOT / "crates/dali2rust-contracts/src/msg/bounded.rs"
SOURCES = sorted(ROOT.glob("crates/*/src/**/*.rs"))

CAPACITY = re.compile(r"pub type FixedText32 = HeaplessString<(\d+)>;")
NAME = re.compile(r"^[a-z][a-z0-9_]*$")
LITERAL = re.compile(r'"([^"\\]*)"')
CONST_REF = re.compile(r"\b([A-Z][A-Z0-9_]{2,})\b")
CONST_DEF = re.compile(r'\bconst ([A-Z][A-Z0-9_]*): &(?:\'static )?str = "([^"]*)";')
CARRIERS = re.compile(
    r"\b(?:OperationFailed|Conflict|confirm_failed|reject_with|reject|fail_runtime_invalid"
    r"|CompactErrorPayload::new|failed|synthesize_failed_outcome|fail_operation_over_lost_fact)\("
)
MESSAGE_FN = re.compile(r"fn (?:message|outcome)\(self\) -> &'static str \{")


def call_body(text, open_paren):
    depth = 0
    for index in range(open_paren, len(text)):
        if text[index] == "(":
            depth += 1
        elif text[index] == ")":
            depth -= 1
            if depth == 0:
                return text[open_paren + 1 : index]
    return ""


def block_body(text, open_brace):
    depth = 0
    for index in range(open_brace, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return text[open_brace + 1 : index]
    return ""


def carried(text):
    for match in CARRIERS.finditer(text):
        yield call_body(text, match.end() - 1)
    for match in MESSAGE_FN.finditer(text):
        yield block_body(text, match.end() - 1)


def outcome_names(texts, consts):
    names = {}
    for path, text in texts:
        for body in carried(text):
            found = LITERAL.findall(body)
            found += [consts[ref] for ref in CONST_REF.findall(body) if ref in consts]
            for name in found:
                if NAME.match(name):
                    names.setdefault(name, path.relative_to(ROOT))
    return names


def main():
    capacity = int(CAPACITY.search(BOUNDED.read_text()).group(1))
    texts = [(path, path.read_text()) for path in SOURCES]
    consts = {name: value for _, text in texts for name, value in CONST_DEF.findall(text)}
    names = outcome_names(texts, consts)
    long = sorted((name, path) for name, path in names.items() if len(name) > capacity)
    for name, path in long:
        print(f"verify_outcome_names: {name} ({len(name)} B, {path}) is cut by a {capacity}-byte message")
    if long:
        return 1
    print(f"verify_outcome_names: OK ({len(names)} outcome names fit {capacity} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
