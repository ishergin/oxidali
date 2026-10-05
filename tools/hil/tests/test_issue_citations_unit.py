import re
from pathlib import Path

HIL_ROOT = Path(__file__).resolve().parent.parent
REPO_ROOT = HIL_ROOT.parent.parent
REGISTRY = REPO_ROOT / "documentation" / "product-design" / "issue-ids-registry.md"
ISR_GATE = REPO_ROOT / "scripts" / "verify_dali_isr_iram.py"
REGISTRY_ROW = re.compile(r"^\| ISSUE-([0-9]+) \| ([^|]+) \|", re.M)
CLOSED = "закрыт"
CITATION = re.compile(r"(?i:issue)-?([0-9]+)")
SOURCE_SUFFIXES = frozenset({".py", ".txt", ".sh", ".toml", ".example"})
LOCAL_TREES = frozenset({".venv", "state", "runs", "corpus", "vendor", "__pycache__",
                         ".pytest_cache", "hil.egg-info"})


def _open_issues():
    return {int(number) for number, status in REGISTRY_ROW.findall(REGISTRY.read_text())
            if not status.strip().startswith(CLOSED)}


def _sources():
    found = [path for path in HIL_ROOT.rglob("*")
             if path.is_file() and path.suffix in SOURCE_SUFFIXES
             and not LOCAL_TREES & set(path.relative_to(HIL_ROOT).parts)]
    return sorted(found) + [ISR_GATE]


def _citations(path):
    named = str(path.relative_to(REPO_ROOT))
    yield from ((named, int(number)) for number in CITATION.findall(path.name))
    for row, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
        yield from (("%s:%d" % (named, row), int(number)) for number in CITATION.findall(line))


def test_the_registry_is_read():
    assert _open_issues(), "no open issue was read out of %s" % REGISTRY


def test_the_toolkit_cites_only_open_issues():
    open_issues = _open_issues()
    closed = ["%s cites ISSUE-%d" % (where, number) for path in _sources()
              for where, number in _citations(path) if number not in open_issues]
    assert closed == [], "\n".join(closed)
