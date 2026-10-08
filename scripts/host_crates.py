import sys
from pathlib import Path

if sys.version_info < (3, 11):
    raise SystemExit("the merge gates need Python 3.11 or newer (tomllib)")

import tomllib

SCRIPTS = Path(__file__).resolve().parent
LIST = SCRIPTS / "host_crates.txt"
WORKSPACE = SCRIPTS.parent / "Cargo.toml"
NOT_HOST = {"crates/dali2rust-firmware", "tests/dali2rust-bdd"}


def host_crates():
    names = [line.split("#", 1)[0].strip() for line in LIST.read_text(encoding="utf-8").splitlines()]
    names = [name for name in names if name]
    if not names:
        raise SystemExit(f"{LIST} lists no crates")
    members = tomllib.loads(WORKSPACE.read_text(encoding="utf-8"))["workspace"]["members"]
    expected = sorted(member.removeprefix("crates/") for member in members if member not in NOT_HOST)
    if sorted(names) != expected:
        missing = sorted(set(expected) - set(names))
        stray = sorted(set(names) - set(expected))
        raise SystemExit(f"{LIST} and the workspace members disagree: missing {missing}, not members {stray}")
    return names
