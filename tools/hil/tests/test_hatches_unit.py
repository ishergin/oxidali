import ast
from pathlib import Path

TESTS = Path(__file__).resolve().parent


def _trees():
    return [(path.name, ast.parse(path.read_text())) for path in sorted(TESTS.glob("test_*.py"))]


def _named(node, owner, attr):
    return isinstance(node, ast.Attribute) and node.attr == attr and (
        isinstance(node.value, ast.Name) and node.value.id == owner
        or isinstance(node.value, ast.Attribute) and node.value.attr == owner)


def _strict(call):
    return any(kw.arg == "strict" and isinstance(kw.value, ast.Constant)
               and kw.value.value is True for kw in call.keywords)


def test_no_test_hides_a_failure_behind_an_imperative_xfail():
    found = ["%s:%d" % (name, node.lineno) for name, tree in _trees()
             for node in ast.walk(tree)
             if isinstance(node, ast.Call) and _named(node.func, "pytest", "xfail")]
    assert found == [], "an imperative xfail never reports XPASS: %s" % found


def test_every_xfail_marker_is_strict():
    loose = []
    for name, tree in _trees():
        markers = [node for node in ast.walk(tree) if _named(node, "mark", "xfail")]
        strict = {id(node.func) for node in ast.walk(tree)
                  if isinstance(node, ast.Call) and _strict(node)}
        loose += ["%s:%d" % (name, node.lineno) for node in markers if id(node) not in strict]
    assert loose == [], "an xfail without strict=True hides its own fix: %s" % loose
