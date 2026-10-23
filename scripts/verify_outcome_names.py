#!/usr/bin/env python3

import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path

DEFAULT_ROOT = Path(__file__).resolve().parent.parent
BOUNDED = "crates/dali2rust-contracts/src/msg/bounded.rs"
SOURCES = "crates/*/src/**/*.rs"

CAPACITY = re.compile(r"pub type FixedText32 = HeaplessString<(\d+)>;")
NAME = re.compile(r"^[a-z][a-z0-9_]*$")
LITERAL = re.compile(r'"((?:[^"\\]|\\[\s\S])*)"')
CONST_REF = re.compile(r"\b([A-Z][A-Z0-9_]{2,})\b")
CONST_DEF = re.compile(r'\bconst ([A-Z][A-Z0-9_]*): &(?:\'static )?str = "([^"]*)";')
CARRIERS = re.compile(
    r"\b(?:OperationFailed|Conflict|confirm_failed|reject_with|reject|fail_runtime_invalid"
    r"|CompactErrorPayload::new|failed|synthesize_failed_outcome|fail_operation_over_lost_fact)\("
)
MESSAGE_FN = re.compile(r"fn (?:message|outcome)\(self\) -> &'static str \{")
TUPLE_TABLE = re.compile(
    r"\(\s*(?:\w+::)*ErrorCode::\w+\s*,\s*(\"[^\"\\]*\"|[A-Z][A-Z0-9_]{2,})\s*[,)]"
)

STRING_OR_CHAR = re.compile(r'"(?:[^"\\]|\\[\s\S])*"|\'(?:[^\'\\]|\\[\s\S])\'')
FN_DEF = re.compile(r"\bfn\s+([a-z_][a-z0-9_]*)")
CALL = re.compile(r"(?<![\w])([a-z_][a-z0-9_]*)\s*\(")
LET = re.compile(r"\blet\b([^;=]*?)(?<![=!<>])=(?![=>])")
DEFINITION = re.compile(r"\bfn\s+$")
CONDITIONAL_LET = re.compile(r"\b(?:if|while)\s*$")
IDENT = re.compile(r"\b[a-z_][a-z0-9_]*\b")
BARE_IDENT = re.compile(r"(?<![\w.:])([a-z_][a-z0-9_]*)\b(?!\s*(?:\(|\.|::|!))")
SELF_PARAM = re.compile(r"^(?:&\s*(?:'\w+\s+)?)?(?:mut\s+)?self\b")
TEXT_TYPE = re.compile(r"\bstr\b|\bString\b")
OPENING, CLOSING = "([{", ")]}"


def mask_literals(text):
    def blank(match):
        whole = match.group(0)
        return whole[0] + " " * (len(whole) - 2) + whole[-1]

    return STRING_OR_CHAR.sub(blank, text).replace("->", "--").replace("=>", "==")


def close_of(mask, open_index):
    depth = 0
    for index in range(open_index, len(mask)):
        if mask[index] in OPENING:
            depth += 1
        elif mask[index] in CLOSING:
            depth -= 1
            if depth == 0:
                return index
    return len(mask)


def statement_end(mask, start):
    depth = 0
    for index in range(start, len(mask)):
        if mask[index] in OPENING:
            depth += 1
        elif mask[index] in CLOSING:
            depth -= 1
            if depth < 0:
                return index
        elif mask[index] == ";" and depth == 0:
            return index
    return len(mask)


def split_top(mask, start, end, angles=False):
    spans, begin, depth = [], start, 0
    for index in range(start, end):
        char = mask[index]
        if char in OPENING or (angles and char == "<"):
            depth += 1
        elif char in CLOSING or (angles and char == ">"):
            depth -= 1
        elif char == "," and depth == 0:
            spans.append((begin, index))
            begin = index + 1
    spans.append((begin, end))
    return [span for span in spans if mask[span[0] : span[1]].strip()]


class Function:
    def __init__(self, name, params, sinks, calls):
        self.name = name
        self.params = params
        self.sinks = sinks
        self.calls = calls


class Source:
    def __init__(self, path, text):
        self.path = path
        self.text = text
        self.mask = mask_literals(text)
        self.fn_starts = [match.start() for match in FN_DEF.finditer(self.mask)]
        self.lets = self.bindings()
        self.functions = list(self.function_definitions())

    def bindings(self):
        found = defaultdict(list)
        for match in LET.finditer(self.mask):
            if CONDITIONAL_LET.search(self.mask[max(0, match.start() - 8) : match.start()]):
                continue
            span = (match.start(), match.end(), statement_end(self.mask, match.end()))
            for ident in IDENT.findall(match.group(1)):
                found[ident].append(span)
        return found

    def function_definitions(self):
        for match in FN_DEF.finditer(self.mask):
            opening = self.parameter_list_start(match.end())
            if opening is None:
                continue
            closing = close_of(self.mask, opening)
            body = self.body_span(closing + 1)
            if body is not None:
                yield self.function(match.group(1), self.parameters(opening, closing), body)

    def function(self, name, params, body):
        start, end = body
        sinks = []
        for carrier in CARRIERS.finditer(self.mask, start, end):
            sinks += self.arguments(carrier.end() - 1)
        calls = [(callee, self.arguments(opening)) for callee, opening in self.calls(start, end)]
        return Function(name, params, sinks, calls)

    def parameter_list_start(self, index):
        while self.mask[index : index + 1].isspace():
            index += 1
        if self.mask[index : index + 1] == "<":
            depth = 0
            while index < len(self.mask):
                depth += {"<": 1, ">": -1}.get(self.mask[index], 0)
                index += 1
                if depth == 0:
                    break
        while self.mask[index : index + 1].isspace():
            index += 1
        return index if self.mask[index : index + 1] == "(" else None

    def body_span(self, index):
        depth = 0
        for at in range(index, len(self.mask)):
            char = self.mask[at]
            if char in "([":
                depth += 1
            elif char in ")]":
                depth -= 1
            elif char == "{" and depth == 0:
                return (at, close_of(self.mask, at))
            elif char == ";" and depth == 0:
                return None
        return None

    def parameters(self, opening, closing):
        params, position = [], 0
        for start, end in split_top(self.mask, opening + 1, closing, angles=True):
            declaration = self.mask[start:end].strip()
            if SELF_PARAM.match(declaration):
                continue
            pattern, kind = (re.split(r"(?<!:):(?!:)", declaration, maxsplit=1) + [""])[:2]
            names = [word for word in IDENT.findall(pattern) if word not in ("mut", "ref")]
            if names and TEXT_TYPE.search(kind):
                params.append((position, names[-1]))
            position += 1
        return params

    def enclosing_fn_start(self, position):
        before = [start for start in self.fn_starts if start <= position]
        return before[-1] if before else 0

    def calls(self, start, end):
        for match in CALL.finditer(self.mask, start, end):
            if not DEFINITION.search(self.mask[max(0, match.start() - 8) : match.start()]):
                yield match.group(1), match.end() - 1

    def arguments(self, opening):
        return split_top(self.mask, opening + 1, close_of(self.mask, opening))

    def names_in(self, start, end, consts):
        region = self.text[start:end]
        found = LITERAL.findall(region)
        found += [consts[ref] for ref in CONST_REF.findall(region) if ref in consts]
        return found

    def names_bound_in(self, start, end, consts):
        found, fn_start = [], self.enclosing_fn_start(start)
        for ident in BARE_IDENT.findall(self.mask[start:end]):
            for let_start, rhs_start, rhs_end in self.lets.get(ident, ()):
                if fn_start <= let_start < start:
                    found += self.names_in(rhs_start, rhs_end, consts)
        return found

    def names_reaching(self, start, end, consts):
        return self.names_in(start, end, consts) + self.names_bound_in(start, end, consts)


def reaches_a_message(source, function, closers, param):
    word = re.compile(rf"\b{re.escape(param)}\b")
    regions = list(function.sinks)
    for callee, arguments in function.calls:
        regions += [arguments[i] for i in closers.get(callee, ()) if i < len(arguments)]
    return any(word.search(source.mask[start:end]) for start, end in regions)


def closer_parameters(sources):
    closers = defaultdict(set)
    changed = True
    while changed:
        changed = False
        for source in sources:
            for function in source.functions:
                for index, param in function.params:
                    if index in closers[function.name]:
                        continue
                    if reaches_a_message(source, function, closers, param):
                        closers[function.name].add(index)
                        changed = True
    return {name: indices for name, indices in closers.items() if indices}


def carried(source):
    for match in CARRIERS.finditer(source.mask):
        opening = match.end() - 1
        yield opening + 1, close_of(source.mask, opening)
    for match in MESSAGE_FN.finditer(source.text):
        opening = match.end() - 1
        yield opening + 1, close_of(source.mask, opening)


def closed_arguments(source, closers):
    for name, opening in source.calls(0, len(source.mask)):
        arguments = source.arguments(opening)
        for index in closers.get(name, ()):
            if index < len(arguments):
                yield arguments[index]


def tabled(source, consts):
    for match in TUPLE_TABLE.finditer(source.text):
        value = match.group(1)
        yield value[1:-1] if value.startswith('"') else consts.get(value)


def outcome_names(sources, consts, closers):
    names = {}

    def keep(source, found):
        for name in found:
            if name and NAME.match(name):
                names.setdefault(name, source.path)

    for source in sources:
        for start, end in carried(source):
            keep(source, source.names_reaching(start, end, consts))
        for start, end in closed_arguments(source, closers):
            keep(source, source.names_reaching(start, end, consts))
        keep(source, tabled(source, consts))
    return names


def main(argv=None):
    parser = argparse.ArgumentParser(description="Outcome names fit the 32-byte error message.")
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    root = parser.parse_args(argv).root.resolve()
    capacity = int(CAPACITY.search((root / BOUNDED).read_text()).group(1))
    paths = sorted(root.glob(SOURCES))
    sources = [Source(path.relative_to(root), path.read_text()) for path in paths]
    consts = {name: value for s in sources for name, value in CONST_DEF.findall(s.text)}
    closers = closer_parameters(sources)
    names = outcome_names(sources, consts, closers)
    long = sorted((name, path) for name, path in names.items() if len(name) > capacity)
    for name, path in long:
        print(f"verify_outcome_names: {name} ({len(name)} B, {path}) is cut by a {capacity}-byte message")
    if long:
        return 1
    print(f"verify_outcome_names: OK ({len(names)} outcome names fit {capacity} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
