import ast
import inspect
import textwrap

DESTRUCTIVE = "destructive"
REBOOT_FIXTURES = frozenset({"dut_reboot"})
REMOTE_SERIAL_NAMES = frozenset({"remote_serial", "remote_serial_mod"})
REMOTE_CONTROL = "control"


def _tree(source):
    try:
        return ast.parse(textwrap.dedent(source))
    except SyntaxError:
        return None


def _calls(source):
    tree = _tree(source)
    return [] if tree is None else [n for n in ast.walk(tree) if isinstance(n, ast.Call)]


def reboot_calls(source):
    found = set()
    for call in _calls(source):
        if not isinstance(call.func, ast.Attribute) or call.func.attr != REMOTE_CONTROL:
            continue
        owner = call.func.value.id if isinstance(call.func.value, ast.Name) else None
        if owner in REMOTE_SERIAL_NAMES:
            found.add("remote_serial.control()")
    return found


def called_names(source):
    return {c.func.id for c in _calls(source) if isinstance(c.func, ast.Name)}


def _source(function):
    try:
        return inspect.getsource(function)
    except (OSError, TypeError):
        return ""


def reach_sources(function):
    own = _source(function)
    scope = getattr(function, "__globals__", {})
    helpers = [scope.get(name) for name in sorted(called_names(own))]
    return [own] + [_source(h) for h in helpers
                    if inspect.isfunction(h) and h is not function
                    and h.__module__ == function.__module__]


def reboot_violation(name, fixturenames, markers, sources):
    if DESTRUCTIVE in markers:
        return None
    causes = ["the %s fixture" % f for f in sorted(REBOOT_FIXTURES & set(fixturenames))]
    causes += sorted(set().union(*(reboot_calls(s) for s in sources)))
    if not causes:
        return None
    return ("%s reboots or halts a controller through %s but is not marked destructive: "
            "a reboot is a production event, so it runs only in the destructive tier"
            % (name, ", ".join(causes)))
