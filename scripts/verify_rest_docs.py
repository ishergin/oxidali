#!/usr/bin/env python3
import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
APP = ROOT / "crates/dali2rust-api/src/http/app.rs"
API_SRC = ROOT / "crates/dali2rust-api/src"
ERRORS_RS = ROOT / "crates/dali2rust-contracts/src/msg/errors.rs"
REST_DOCS = ROOT / "documentation/product-design/rest-api"
RESOURCES = REST_DOCS / "resources"

ROUTE_ROW = re.compile(r'\(\s*RouteKey::\w+,\s*HttpMethod::(\w+),\s*"([^"]+)",?\s*\)')
TABLE_HEADER_BASE = re.compile(r"^\|[^|]*\|\s*Путь(?:\s*\(`([^`]*)`\))?")
DOC_ROW = re.compile(r"^\| `(GET|POST|PUT|PATCH|DELETE)` \| `([^`]*)`")
SPEC_ROUTE = re.compile(r'RouteSpec::(get|post|put|patch|delete)\(\s*"(/api/[^"]+)"')
WS_ROUTE = re.compile(r'WS_PATH: &str = "(/api/[^"]+)"')
WS_SOURCES = (
    "crates/dali2rust-adapters/src/http/host_ws.rs",
    "crates/dali2rust-adapters/src/http/esp_ws.rs",
)
INLINE_ROUTE = re.compile(r"`(GET|POST|PUT|PATCH|DELETE) (/api/v1/[^`\s]+)`")
PARAM = re.compile(r"\{[^}]*\}")
OPTIONAL = re.compile(r"\[[^\]]*\]")
ERROR_LITERAL = re.compile(r'\(\s*[0-9]{3}\s*,\s*"([a-z_]+)"')
REST_NAME = re.compile(r'=> "([a-z_]+)"')
BACKTICKED = re.compile(r"`([^`]+)`")
IDENTIFIER = re.compile(r"[a-z][a-z0-9_]*")

errors: list[str] = []


def normalise(path: str) -> str:
    path = OPTIONAL.sub("", path).split("?", 1)[0].rstrip("/")
    return PARAM.sub("{}", path)


def code_routes() -> set[tuple[str, str]]:
    app = APP.read_text(encoding="utf-8")
    routes = {(m.upper(), normalise(p)) for m, p in ROUTE_ROW.findall(app) if p.startswith("/api/")}
    routes |= {(m.upper(), normalise(p)) for m, p in SPEC_ROUTE.findall(app)}
    for source in WS_SOURCES:
        text = (ROOT / source).read_text(encoding="utf-8")
        routes |= {("GET", normalise(p)) for p in WS_ROUTE.findall(text)}
    return routes


def table_base(line: str) -> str | None:
    found = TABLE_HEADER_BASE.match(line)
    if not found:
        return None
    return (found.group(1) or "").replace("…", "").rstrip("/")


def doc_routes_in(path: Path) -> set[tuple[str, str]]:
    routes = set()
    base = None
    for line in path.read_text(encoding="utf-8").splitlines():
        header = table_base(line)
        if header is not None:
            base = header
            continue
        if not line.startswith("|"):
            base = None
        row = DOC_ROW.match(line)
        if row and base is not None:
            method, route = row.groups()
            full = route if route.startswith("/") else f"{base}/{route}" if route else base
            routes.add((method, normalise(full)))
        for method, route in INLINE_ROUTE.findall(line):
            routes.add((method, normalise(route)))
    return routes


def documented_routes() -> set[tuple[str, str]]:
    routes = set()
    for path in sorted(REST_DOCS.rglob("*.md")):
        routes |= doc_routes_in(path)
    return routes


def code_error_names() -> set[str]:
    names = set(REST_NAME.findall(ERRORS_RS.read_text(encoding="utf-8")))
    for path in API_SRC.rglob("*.rs"):
        names |= set(ERROR_LITERAL.findall(path.read_text(encoding="utf-8")))
    return names


def documented_names() -> set[str]:
    names = set()
    for path in REST_DOCS.rglob("*.md"):
        for line in path.read_text(encoding="utf-8").splitlines():
            for span in BACKTICKED.findall(line.replace("``", "")):
                names |= set(IDENTIFIER.findall(span))
    return names


def main() -> int:
    code = code_routes()
    docs = documented_routes()
    for method, path in sorted(code - docs):
        errors.append(f"route {method} {path} is served but no REST document names it")
    for method, path in sorted(docs - code):
        errors.append(f"route {method} {path} is documented but the router does not serve it")
    for name in sorted(code_error_names() - documented_names()):
        errors.append(f"error code `{name}` is answered but no REST document names it")
    for line in errors:
        print(f"verify_rest_docs: {line}")
    if errors:
        print(f"verify_rest_docs: FAILED ({len(errors)} finding(s))")
        return 1
    print(f"verify_rest_docs: OK ({len(code)} routes, {len(code_error_names())} error codes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
