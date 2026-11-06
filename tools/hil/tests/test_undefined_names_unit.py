import ast
import builtins
import pathlib

TESTS = pathlib.Path(__file__).resolve().parent
BUILTINS = frozenset(dir(builtins)) | {"__file__", "__name__", "__doc__"}
BINDERS = (ast.Name, ast.arg, ast.alias, ast.ExceptHandler, ast.Global, ast.Nonlocal,
           ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)


def _module_level(tree):
    bound = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            bound.add(node.name)
        elif isinstance(node, ast.alias):
            bound.add((node.asname or node.name).split(".")[0])
        elif isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
            bound.add(node.id)
    return bound


def _bound_inside(func):
    bound = set()
    for node in ast.walk(func):
        if not isinstance(node, BINDERS):
            continue
        if isinstance(node, ast.Name):
            if isinstance(node.ctx, (ast.Store, ast.Del)):
                bound.add(node.id)
        elif isinstance(node, ast.arg):
            bound.add(node.arg)
        elif isinstance(node, ast.alias):
            bound.add((node.asname or node.name).split(".")[0])
        elif isinstance(node, ast.ExceptHandler):
            if node.name:
                bound.add(node.name)
        elif isinstance(node, (ast.Global, ast.Nonlocal)):
            bound.update(node.names)
        else:
            bound.add(node.name)
    return bound


def _unknown(func, module_level):
    known = module_level | BUILTINS | _bound_inside(func)
    return sorted({node.id for node in ast.walk(func)
                   if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load)
                   and node.id not in known})


def test_no_test_module_names_a_fixture_it_never_took():
    problems = []
    for path in sorted(TESTS.glob("*.py")):
        tree = ast.parse(path.read_text())
        module_level = _module_level(tree)
        for node in tree.body:
            if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            unknown = _unknown(node, module_level)
            if unknown:
                problems.append("%s::%s uses %s" % (path.name, node.name,
                                                    ", ".join(unknown)))
    assert not problems, (
        "a name a test body uses but never takes as a fixture or binds is a rename left "
        "half-done, and only the bench would have said so: %s" % "; ".join(problems))
