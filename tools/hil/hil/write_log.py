import json
import os
import re
from urllib.parse import unquote

ALL = "*"
READ_METHODS = ("GET", "HEAD")
POLICY_FIELDS = frozenset({"power_on_level", "system_failure_level"})
EVERY = frozenset({ALL})
FIELDWISE = (
    (re.compile(r"settings/(poller|dali|redundancy|home-assistant)"), "settings/%s"),
    (re.compile(r"adapters/([0-9]+)"), "adapter/%s"),
    (re.compile(r"adapters/[0-9]+/physical-devices/([0-9]+)"), "device/%s"),
    (re.compile(r"adapters/[0-9]+/physical-devices/([0-9]+)/write-attributes"), "gear/%s"),
    (re.compile(r"adapters/[0-9]+/virtual-lamps/([0-9]+)"), "vl/%s"),
    (re.compile(r"adapters/[0-9]+/groups/([0-9]+)"), "group/%s"),
    (re.compile(r"adapters/[0-9]+/scenes/([0-9]+)"), "scene/%s"),
    (re.compile(r"hcl-schedules/([^/]+)"), "hcl/%s"),
    (re.compile(r"rules/([^/]+)"), "rule/%s"),
    (re.compile(r"(policies|time)"), "%s"),
)
WHOLE = (
    (re.compile(r"(rules)"), "%s", EVERY),
    (re.compile(r"hcl-schedules/([^/]+)/override"), "hcl_override/%s", EVERY),
    (re.compile(r"adapters/[0-9]+/physical-devices/([0-9]+)/target-state"), "shown/%s", EVERY),
    (re.compile(r"adapters/[0-9]+/(?:groups/[0-9]+/target-state|scenes/[0-9]+/recall)"),
     "shown/*", EVERY),
    (re.compile(r"adapters/[0-9]+/virtual-lamps/([0-9]+)/binding"), "vl/%s",
     frozenset({"binding"})),
    (re.compile(r"(policies)/apply"), "gear/*", POLICY_FIELDS),
)
GROUP_MATRIX = re.compile(r"adapters/[0-9]+/group-membership-matrix")
SCENE_MATRIX = re.compile(r"adapters/[0-9]+/scenes/([0-9]+)/matrix")
CREATED = ((re.compile(r"hcl-schedules"), "hcl/%s", "schedule_id"),
           (re.compile(r"adapters/[0-9]+/virtual-lamps"), "vl/%s", "virtual_lamp_id"))
INDIRECT = (re.compile(r"rules/[^/]+/run"), re.compile(r"hcl-schedules(/[^/]+(/override)?)?"),
            re.compile(r"time"))
ANY_LAMP = ("shown/" + ALL, EVERY)
COMMAND_TOPIC = "/set"


def request_keys(method, path, body):
    method = method.upper()
    if method in READ_METHODS:
        return []
    path = unquote(path.lstrip("/").split("?", 1)[0])
    body = body if isinstance(body, dict) else {}
    keys = []
    for find in (_matrix_keys, _whole_keys, _created_keys, _fieldwise_keys):
        found = find(method, path, body)
        if found is not None:
            keys = found
            break
    if any(pattern.fullmatch(path) for pattern in INDIRECT):
        keys = keys + [ANY_LAMP]
    return keys


def topic_keys(topic):
    return [ANY_LAMP] if topic.endswith(COMMAND_TOPIC) else []


def _key(template, match):
    return template % match.group(1) if "%s" in template else template


def _matrix_keys(method, path, body):
    scene = SCENE_MATRIX.fullmatch(path)
    if not GROUP_MATRIX.fullmatch(path) and scene is None:
        return None
    family = "group_matrix" if scene is None else "scene_matrix/%s" % scene.group(1)
    if method != "PATCH":
        return [(family + "/" + ALL, EVERY)]
    return [("%s/%s" % (family, row.get("virtual_lamp_id")), frozenset({"desired"}))
            for row in body.get("rows") or []]


def _whole_keys(method, path, body):
    for pattern, template, fields in WHOLE:
        match = pattern.fullmatch(path)
        if match:
            return [(_key(template, match), fields)]
    return None


def _created_keys(method, path, body):
    for pattern, template, name in CREATED:
        if pattern.fullmatch(path) and method == "POST":
            return [(template % body.get(name, ALL), EVERY)]
    return None


def _fieldwise_keys(method, path, body):
    for pattern, template in FIELDWISE:
        match = pattern.fullmatch(path)
        if match:
            fields = frozenset(body) if method != "DELETE" and body else EVERY
            return [(_key(template, match), fields)]
    return None


def frame_keys(target, visible=True):
    return [("gear/%s" % target, EVERY)] + ([("shown/%s" % target, EVERY)] if visible else [])


def _candidates(key):
    parts = key.split("/")
    return [key] + ["/".join(parts[:i] + [ALL]) for i in range(len(parts) - 1, 0, -1)] + [ALL]


class WriteLog:
    def __init__(self, session, base, path=None, touched=None):
        self.session, self.base, self.path = session, base, path
        self.touched = {k: set(v) for k, v in (touched or {}).items()}

    @classmethod
    def everything(cls, session, base):
        return cls(session, base, touched={ALL: {ALL}})

    @classmethod
    def load(cls, path, session):
        try:
            held = json.loads(path.read_text())
        except (OSError, ValueError):
            return None
        if held.get("session") != session:
            return None
        return cls(session, held.get("base"), path, held.get("touched"))

    def note(self, keys):
        grown = False
        for key, fields in keys:
            have = self.touched.setdefault(key, set())
            grown = grown or not set(fields) <= have
            have.update(fields)
        if grown:
            self.save()

    def changed(self, key, field=None, exact=False):
        for candidate in [key] if exact else _candidates(key):
            fields = self.touched.get(candidate, ())
            if ALL in fields or (field in fields if field is not None else fields):
                return True
        return False

    def save(self):
        if self.path is None:
            return
        tmp = self.path.with_name(self.path.name + ".tmp")
        tmp.write_text(json.dumps({"session": self.session, "base": self.base,
                                   "touched": {k: sorted(v) for k, v in self.touched.items()}},
                                  indent=1, sort_keys=True, ensure_ascii=False))
        os.replace(tmp, self.path)


_SESSION = {"log": None}


def start(log):
    _SESSION["log"] = log


def stop():
    _SESSION["log"] = None


def note(base, keys):
    log = _SESSION["log"]
    if log is not None and keys and (base is None or base == log.base):
        log.note(keys)


def writes_path(snapshot_path):
    return snapshot_path.with_name(snapshot_path.stem + ".writes.json")
