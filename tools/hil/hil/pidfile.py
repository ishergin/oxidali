import os
from pathlib import Path
from typing import Optional


def pid_alive(pidfile: Path) -> bool:
    try:
        os.kill(int(Path(pidfile).read_text().strip()), 0)
        return True
    except (OSError, ValueError):
        return False


def sidecar_log(pidfile: Path) -> Optional[Path]:
    pidfile = Path(pidfile)
    try:
        return Path((pidfile.parent / (pidfile.name + ".log"))
                    .read_text().strip())
    except OSError:
        return None
