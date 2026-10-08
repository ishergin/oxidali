from pathlib import Path

LIST = Path(__file__).resolve().parent / "host_crates.txt"


def host_crates():
    names = [line.split("#", 1)[0].strip() for line in LIST.read_text(encoding="utf-8").splitlines()]
    names = [name for name in names if name]
    if not names:
        raise SystemExit(f"{LIST} lists no crates")
    return names
