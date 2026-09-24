from __future__ import annotations

import hashlib
import json
import os
import subprocess
from pathlib import Path

BENCH_ENV_FILE = "bench.env"
BENCH_ENV_EXAMPLE = "bench.env.example"

REQUIRED = {
    "DALI2RUST_DALI_QUERY_CONTENTION_RETRY": "1",
    "DALI2RUST_DALI_QUERY_CONTENT_CONFIRM": "1",
}

OPTIONAL = (
    "DALI2RUST_PERSIST_DISABLE",
    "DALI2RUST_HEAP_CHECK_S",
    "DALI2RUST_DALI_RETRY_MAX_ATTEMPTS",
    "DALI2RUST_DALI_RETRY_BACKOFF_MS",
    "DALI2RUST_DALI_RETRY_JITTER_MS",
    "DALI2RUST_DALI_QUERY_CONTENT_CONFIRM_MAX_SAMPLES",
    "DALI2RUST_DALI_DISCOVERY_STEP_RETRIES",
    "DALI2RUST_DALI_TARGET_SEQUENCE_RETRIES",
    "DALI2RUST_PHY_ISR_LEVEL",
)

SECRET_PREFIXES = ("WIFI_",)


class BenchEnvError(RuntimeError):
    pass


def parse_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path.is_file():
        return values
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def resolve(hil_root: Path) -> dict[str, str]:
    env = dict(os.environ)
    env.update(parse_env_file(hil_root / BENCH_ENV_FILE))
    return env


def validate(env: dict[str, str]) -> list[str]:
    problems = []
    for key, expected in REQUIRED.items():
        actual = env.get(key)
        if actual is None:
            problems.append(f"{key} is not set (expected {expected})")
        elif actual != expected:
            problems.append(f"{key}={actual!r}, expected {expected!r}")
    return problems


def effective_knobs(env: dict[str, str]) -> dict[str, str]:
    knobs = {}
    for key in list(REQUIRED) + list(OPTIONAL):
        if key.startswith(SECRET_PREFIXES):
            continue
        if key in env:
            knobs[key] = env[key]
    return knobs


def _git(repo_root: Path, *args: str) -> str:
    try:
        out = subprocess.run(
            ["git", *args], cwd=repo_root, capture_output=True, text=True, timeout=10
        )
        return out.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def head_commit(repo_root: Path) -> str:
    return _git(repo_root, "rev-parse", "HEAD")


def record_boot_version(manifest: Path, version: str) -> None:
    try:
        doc = json.loads(manifest.read_text())
        doc.setdefault("firmware", {})["reported_version"] = version
        manifest.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
    except (OSError, ValueError) as e:
        print("manifest: could not record reported version: %s" % e)


def _sha256(path: Path) -> str:
    if not path.is_file():
        return ""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_manifest(run_dir: Path, repo_root: Path, env: dict[str, str],
                   firmware_bin: Path, target: str, bench_valid: bool,
                   board_env: dict[str, str] | None = None,
                   checks: dict[str, str] | None = None,
                   transport: str | None = None) -> Path:
    dirty = _git(repo_root, "status", "--porcelain")
    manifest = {
        "git": {
            "commit": _git(repo_root, "rev-parse", "HEAD"),
            "branch": _git(repo_root, "rev-parse", "--abbrev-ref", "HEAD"),
            "dirty": bool(dirty),
            "dirty_files": len(dirty.splitlines()) if dirty else 0,
        },
        "firmware": {
            "target": target,
            "path": str(firmware_bin),
            "sha256": _sha256(repo_root / firmware_bin),
            "size_bytes": (repo_root / firmware_bin).stat().st_size
            if (repo_root / firmware_bin).is_file()
            else 0,
        },
        "build_env": effective_knobs(env),
        "board_env": dict(board_env or {}),
        "bench_valid": bench_valid,
        "checks": dict(checks or {}),
        "flash_transport": transport or "local",
    }
    run_dir.mkdir(parents=True, exist_ok=True)
    path = run_dir / "manifest.json"
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return path
