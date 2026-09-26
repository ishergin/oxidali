import collections
import contextlib
import time

import requests

from urllib.parse import quote

from hil import lamp_guard
from hil.config import HilConfig
from hil.lamp_guard import LampGuard, LampNotAllowed

OP_POLL_S = 0.7
OP_TIMEOUT_S = 30.0
ATTR_GROUPS_DEFAULT = "runtime_status,common_102,dt8_color,dt6_led"

BUS_CONTENDED = "bus_contended"

HTTP_GATEWAY_TIMEOUT = 504
RULES_RECONCILE_S = 10.0
RULES_LANDED, RULES_OVERTAKEN, RULES_UNCHANGED = "landed", "overtaken", "unchanged"


def rules_put_outcome(doc, source, base_revision):
    if doc.get("revision") == base_revision:
        return RULES_UNCHANGED
    return RULES_LANDED if doc.get("source") == source else RULES_OVERTAKEN


def op_contended(view) -> bool:
    if not isinstance(view, dict) or view.get("status") != "failed":
        return False
    error = view.get("error") or {}
    return str(error.get("message", "")) == BUS_CONTENDED


class ApiError(RuntimeError):
    def __init__(self, status, body, url):
        super().__init__("HTTP %s %s: %s" % (status, url, str(body)[:200]))
        self.status = status
        self.body = body


class CapabilityUnsupported(ApiError):
    pass


def _cause_of(exc):
    names = [type(exc).__name__]
    cause = exc
    seen = 0
    while seen < 6:
        nxt = getattr(cause, "__cause__", None) or getattr(cause, "__context__", None)
        if nxt is None:
            nxt = getattr(cause, "reason", None)
        if nxt is None or nxt is cause:
            break
        cause = nxt
        seen += 1
        name = type(cause).__name__
        errno_name = getattr(cause, "errno", None)
        if errno_name is not None:
            import errno as _errno
            name = "%s/%s" % (name, _errno.errorcode.get(errno_name, errno_name))
        if name not in names:
            names.append(name)
    return " < ".join(names)


class Client:
    context = ""

    reboot_generation = 0
    _seen_reboot_generation = 0

    def __init__(self, cfg: HilConfig, timeout_s: float = 10.0):
        self.cfg = cfg
        self.base = cfg.base.rstrip("/")
        self.adapter = cfg.adapter
        self.timeout_s = timeout_s
        self.http = requests.Session()
        self._seen_reboot_generation = Client.reboot_generation
        self.raw_touched = set()
        self.groups = _Groups(self)
        self.scenes = _Scenes(self)
        self.vlamps = _VirtualLamps(self)
        self.hcl = _HclSchedules(self)
        self.poller = _PollerSettings(self)
        self.firmware = _Firmware(self)
        self.ha = _HomeAssistantSettings(self)
        self.dali_settings = _DaliSettings(self)
        self.redundancy = _Redundancy(self)
        self.config = _ConfigSlices(self)
        self.guard = LampGuard.for_config(cfg, segment=self.segment_shorts,
                                          binding=self._bound_short)
        self.init_ledger()
        self._rebooting = False

    @contextlib.contextmanager
    def expect_reboot(self):
        previous = self._rebooting
        self._rebooting = True
        try:
            yield
        finally:
            self._rebooting = previous
            Client.reboot_generation += 1

    def _drop_pool_after_reboot(self):
        if self._seen_reboot_generation == Client.reboot_generation:
            return
        self._seen_reboot_generation = Client.reboot_generation
        self.http.close()

    def _where(self):
        return " in %s" % self.context if self.context else ""

    def init_ledger(self):
        self.retries = collections.Counter()
        self.retry_events = []

    def count_retry(self, kind, detail):
        self.retries[kind] += 1
        self.retry_events.append("%s %s%s" % (kind, detail, self._where()))

    def _url(self, path):
        return "%s/api/v1/%s" % (self.base, path.lstrip("/"))

    IDEMPOTENT = ("GET", "PUT")

    def _http(self, method, path, body=None, conditional=False):
        url = self._url(path)
        self.guard.check_request(method, path, body)
        self._drop_pool_after_reboot()
        self._note_diagnostic_write(method, path, body)
        attempts = 3 if method in self.IDEMPOTENT and not conditional else 1
        for attempt in range(attempts):
            try:
                resp = self.http.request(method, url, json=body,
                                         timeout=self.timeout_s)
                break
            except requests.RequestException as exc:
                if attempt == attempts - 1:
                    raise
                kind = "http_reboot_race" if self._rebooting else "http_transport"
                self.retries[kind] += 1
                self.retry_events.append(
                    "%s %s %s (%s)%s"
                    % (kind, method, path, _cause_of(exc), self._where()))
                print("hil: %s %s failed (%s), retry %d/%d"
                      % (method, path, type(exc).__name__,
                         attempt + 1, attempts - 1))
                time.sleep(1.5)
        try:
            payload = resp.json() if resp.content else {}
        except ValueError:
            payload = {"raw": resp.text}
        return resp.status_code, payload, url

    def _req(self, method, path, body=None, _retry_503=True, _retry_504=2,
             conditional=False):
        status, payload, url = self._http(method, path, body, conditional)
        if status >= 400:
            err = str(payload.get("error", ""))
            if status == 503 and _retry_503:
                self.retries["http_503"] += 1
                self.retry_events.append(
                    "http_503 %s %s%s" % (method, path, self._where()))
                time.sleep(1.2)
                return self._req(method, path, body, _retry_503=False,
                                 conditional=conditional)
            if (status == 504 and _retry_504 > 0
                    and method in self.IDEMPOTENT and not conditional):
                self.retries["http_504"] += 1
                self.retry_events.append(
                    "http_504 %s %s%s" % (method, path, self._where()))
                time.sleep(1.5)
                return self._req(method, path, body,
                                 _retry_503=_retry_503,
                                 _retry_504=_retry_504 - 1)
            if status == 404 or (
                    status == 422 and err.startswith("unsupported")):
                raise CapabilityUnsupported(status, payload, url)
            raise ApiError(status, payload, url)
        return payload

    def raw_request(self, method, path, body=None):
        status, payload, _ = self._http(method, path, body)
        return status, payload

    def raw_response(self, method, path, body=None):
        self.guard.check_request(method, path, body)
        return self.http.request(method, self._url(path), json=body,
                                 timeout=self.timeout_s)

    def health(self):
        return self._req("GET", "health")

    def devices(self):
        body = self.devices_unfiltered()
        wanted = self.cfg.gear_short_set()
        if wanted is not None and isinstance(body, dict):
            body = dict(body, physical_devices=[
                d for d in body.get("physical_devices", [])
                if d.get("short_address") in wanted])
        return body

    def devices_unfiltered(self):
        return self._req("GET", "adapters/%d/physical-devices" % self.adapter)

    def addrs(self):
        return sorted(d["short_address"] for d in self.devices()["physical_devices"])

    def present_addrs(self):
        return sorted(d["short_address"] for d in self.devices()["physical_devices"]
                      if d.get("present", True))

    def segment_shorts(self):
        return sorted(d["short_address"] for d in self.devices_unfiltered()["physical_devices"]
                      if d.get("present", True))

    def lamp_addrs(self):
        allowed = self.cfg.lamp_short_set()
        return [a for a in self.addrs() if a in allowed]

    def _bound_short(self, lamp_id):
        try:
            lamp = self.vlamps.get(lamp_id)
        except ApiError:
            return None
        return (lamp.get("binding") or {}).get("physical_short_address")

    def optical_addrs(self):
        wanted = self.cfg.optical_short_set()
        return [a for a in self.addrs() if a in wanted]

    def present_optical_addrs(self):
        present = set(self.present_addrs())
        return [a for a in self.optical_addrs() if a in present]

    def state(self, short):
        return self._req("GET", "adapters/%d/physical-devices/%d" % (self.adapter, short))

    def attributes(self, short, sections=None):
        path = "adapters/%d/physical-devices/%d/attributes" % (self.adapter, short)
        if sections:
            path += "?sections=%s" % ",".join(sections)
        return self._req("GET", path)

    def memory_banks(self, short):
        return self._req("GET", "adapters/%d/physical-devices/%d/memory-banks"
                         % (self.adapter, short))

    def device_full(self, short):
        dev = dict(self.state(short))
        dev["attributes"] = self.attributes(short).get("attributes", {})
        dev["memory_banks"] = self.memory_banks(short).get("memory_banks", [])
        return dev

    def controller(self) -> dict:
        return self._req("GET", "controller")

    def adapters(self) -> dict:
        return self._req("GET", "adapters")

    def adapter_info(self, aid: int = None) -> dict:
        return self._req("GET", "adapters/%d"
                         % (self.adapter if aid is None else aid))

    def adapter_patch(self, patch: dict, aid: int = None) -> dict:
        return self._req("PATCH", "adapters/%d"
                         % (self.adapter if aid is None else aid), patch)

    def operations(self) -> list:
        return self._req("GET", "operations")["operations"]

    def diagnostics(self) -> dict:
        return self._req("GET", "diagnostics")

    def stats(self) -> dict:
        return self._req("GET", "stats")

    def time_get(self) -> dict:
        return self._req("GET", "time")

    def time_set(self, unix_ms: int = None, timezone: str = None) -> dict:
        body = {}
        if unix_ms is not None:
            body["unix_ms"] = int(unix_ms)
        if timezone is not None:
            body["timezone"] = timezone
        return self._req("PUT", "time", body)


    def device_patch(self, short: int, patch: dict) -> dict:
        return self._req("PATCH", "adapters/%d/physical-devices/%d"
                         % (self.adapter, short), patch)

    def device_forget(self, short: int):
        return self._req("DELETE", "adapters/%d/physical-devices/%d"
                         % (self.adapter, short))

    def write_attrs(self, short: int, attrs: dict) -> dict:
        return self._req("POST", "adapters/%d/physical-devices/%d/write-attributes"
                         % (self.adapter, short), attrs)

    def ts(self, short: int, setpoint: dict) -> dict:
        return self._req("PUT", "adapters/%d/physical-devices/%d/target-state"
                         % (self.adapter, short), setpoint)

    def group_ts(self, group_id: int, setpoint: dict) -> dict:
        return self.groups.ts(group_id, setpoint)

    def off(self, short):
        return self.ts(short, {"power": "off"})

    def off_many(self, addrs):
        for a in addrs:
            self.off(a)
            time.sleep(0.35)

    def off_all(self):
        self.off_many(self.lamp_addrs())

    TOUCHED_ALL = lamp_guard.TARGET_SEGMENT

    def _note_diagnostic_write(self, method, path, body):
        if method != "POST":
            return
        frame = lamp_guard.diagnostic_frame(path, body)
        if frame is not None and lamp_guard.frame_visible(*frame):
            self.raw_touched.add(lamp_guard.wire_target(frame[0]))

    def dapc(self, short: int, level: int) -> dict:
        return self._req("POST", "dali/level",
                         {"wire_address": short << 1, "level": level})

    def cmd(self, short: int, opcode: int, repeat: int = 1) -> dict:
        return self._req("POST", "dali/command",
                         {"wire_address": (short << 1) | 1, "command": opcode,
                          "repeat_count": repeat})

    def cmd_wire(self, wire_address: int, opcode: int, repeat: int = 1) -> dict:
        return self._req("POST", "dali/command",
                         {"wire_address": wire_address, "command": opcode,
                          "repeat_count": repeat})

    def raw(self, frame: int, expects_backward: bool = False) -> dict:
        body = self._req("POST", "dali/raw",
                         {"frame": frame, "expects_backward": expects_backward})
        self.retries["raw_exchanges"] += 1
        if isinstance(body, dict) and body.get("success") is False:
            self.retries["raw_unanswered"] += 1
        return body

    COLOUR_VALUE_TEMPORARY_COLOUR_TYPE = 208
    DT8_MASK = 0xFF

    def temporary_colour_type(self, short: int):
        dtr0 = 0xA300 | self.COLOUR_VALUE_TEMPORARY_COLOUR_TYPE
        enable_dt8 = 0xC108
        query = ((short << 1) | 1) << 8 | 0xFA
        self.raw(dtr0)
        self.raw(enable_dt8)
        body = self.raw(query, expects_backward=True)
        if not isinstance(body, dict) or body.get("success") is not True:
            return None
        return body.get("backward_frame")

    def colour_is_activated(self, short: int):
        answer = self.temporary_colour_type(short)
        if answer is None:
            return None
        return answer == self.DT8_MASK

    DT8_QUERY_COLOUR_TYPE_FEATURES = 249

    def colour_type_features(self, short: int):
        self.raw(0xC108)
        body = self.raw(((short << 1) | 1) << 8 | self.DT8_QUERY_COLOUR_TYPE_FEATURES,
                        expects_backward=True)
        if not isinstance(body, dict) or body.get("success") is not True:
            return None
        return body.get("backward_frame")

    def rgbwaf_channel_count(self, short: int):
        features = self.colour_type_features(short)
        return None if features is None else (features >> 5) & 0x07

    def input_devices(self):
        return self._req("GET", "adapters/%d/input-devices" % self.adapter)

    def input_device(self, short):
        return self._req("GET", "adapters/%d/input-devices/%d" % (self.adapter, short))

    def input_scan(self):
        return self._req("POST", "adapters/%d/input-devices/scan" % self.adapter, {})

    def input_instance_patch(self, short, instance, body):
        return self._req(
            "PATCH",
            "adapters/%d/input-devices/%d/instances/%d" % (self.adapter, short, instance),
            body)

    def input_feedback_patch(self, short, instance, body):
        return self._req(
            "PATCH",
            "adapters/%d/input-devices/%d/instances/%d/feedback"
            % (self.adapter, short, instance),
            body)

    def input_commission(self, include_addressed=False):
        return self._req("POST", "adapters/%d/input-devices/commission" % self.adapter,
                         {"include_addressed": bool(include_addressed)})

    def rules_get(self) -> dict:
        return self._req("GET", "rules")

    def rules_replace(self, source: str, base_revision: int) -> dict:
        body = {"source": source, "base_revision": base_revision}
        try:
            accepted = self._req("PUT", "rules", body, conditional=True)
        except requests.RequestException as exc:
            return self._reconcile_rules(body, _cause_of(exc))
        except ApiError as exc:
            if exc.status != HTTP_GATEWAY_TIMEOUT:
                raise
            return self._reconcile_rules(body, "HTTP %d" % exc.status)
        return self.wait_op(accepted)

    def _reconcile_rules(self, body, cause):
        base = body["base_revision"]
        self.count_retry("rules_put_reconciled",
                         "PUT rules base_revision=%d lost its answer (%s)" % (base, cause))
        deadline = time.monotonic() + RULES_RECONCILE_S
        doc = self.rules_get()
        while doc.get("revision") == base and time.monotonic() < deadline:
            time.sleep(OP_POLL_S)
            doc = self.rules_get()
        outcome = rules_put_outcome(doc, body["source"], base)
        if outcome == RULES_LANDED:
            return {"status": "succeeded", "reconciled": outcome,
                    "revision": doc.get("revision")}
        if outcome == RULES_OVERTAKEN:
            return {"status": "failed", "reconciled": outcome,
                    "error": {"code": "conflict", "message": "rule_set_conflict"}}
        return self.wait_op(self._req("PUT", "rules", body, conditional=True))

    def rules_run(self, name: str, dry: bool = False) -> dict:
        return self._req("POST", "rules/%s/run%s"
                         % (quote(name, safe=""), "?dry=1" if dry else ""), {})

    def attr_read(self, short, groups=ATTR_GROUPS_DEFAULT, banks="none"):
        return self._req("POST", "adapters/%d/physical-devices/%d/attribute-reads"
                         % (self.adapter, short),
                         {"attribute_groups": groups.split(","), "memory_banks": banks})

    def attr_read_checked(self, short, groups=ATTR_GROUPS_DEFAULT,
                          banks="none", retries=2):
        view = {}
        for attempt in range(retries + 1):
            view = self.wait_op(self.attr_read(short, groups=groups, banks=banks))
            if view.get("status") == "succeeded":
                if attempt:
                    self.retries["attr_read"] += attempt
                    self.retry_events.append(
                        "attr_read short=%d groups=%s after %d attempt(s)%s"
                        % (short, groups, attempt, self._where()))
                return view
            time.sleep(0.5)
        self.retries["attr_read"] += retries
        self.retries["attr_read_exhausted"] += 1
        self.retry_events.append(
            "attr_read short=%d groups=%s EXHAUSTED (last status %r)%s"
            % (short, groups, view.get("status"), self._where()))
        raise ApiError(500, {"error": "attr_read_not_succeeded",
                             "last": view.get("status"), "view": view},
                       "attribute-reads")

    def identify(self, short):
        return self._req("POST", "adapters/%d/commissioning/identify" % self.adapter,
                         {"short_address": short})

    def discovery(self, mode):
        return self._req("POST", "adapters/%d/discovery-runs" % self.adapter,
                         {"mode": mode})

    def _config_write(self, method: str, path: str, body: dict) -> dict:
        res = self._req(method, path, body)
        if isinstance(res, dict) and "operation_id" in res:
            done = self.wait_op(res)
            status = done.get("status")
            if status not in ("succeeded", "evicted_or_unknown"):
                raise AssertionError(
                    "config write %s %s did not commit: %s" % (method, path, done)
                )
            return {**res, **done}
        return res

    def op(self, op_id):
        return self._req("GET", "operations/%s" % op_id)

    def wait_op(self, op_or_id, timeout_s=OP_TIMEOUT_S):
        op_id = op_or_id["operation_id"] if isinstance(op_or_id, dict) else op_or_id
        deadline = time.monotonic() + timeout_s
        last = None
        seen = False
        while time.monotonic() < deadline:
            try:
                last = self.op(op_id)
                seen = True
            except CapabilityUnsupported:
                if seen:
                    return {"operation_id": op_id, "status": "evicted_or_unknown"}
                time.sleep(OP_POLL_S)
                continue
            if last.get("status") in ("succeeded", "failed", "timed_out", "cancelled"):
                return last
            time.sleep(OP_POLL_S)
        if not seen:
            return {"operation_id": op_id, "status": "never_registered"}
        return last or {"operation_id": op_id, "status": "poll_timeout"}

    def config_snapshot(self) -> dict:
        return {
            "vl": self.vlamps.list(),
            "group_matrix": self.groups.matrix(),
            "scenes": self._scene_snapshot(),
        }

    def _scene_snapshot(self) -> list:
        out = []
        for scene in self.scenes.list()["scenes"]:
            scene_id = scene["scene_id"]
            rows = [{"virtual_lamp_id": r["virtual_lamp_id"], "desired": r["desired"]}
                    for r in self.scenes.matrix(scene_id)["rows"]
                    if (r.get("desired") or {}).get("included")]
            out.append({"scene_id": scene_id,
                        "name": scene.get("name") or "",
                        "ha_select_enabled": scene.get("ha_select_enabled", True),
                        "rows": rows})
        return out

    def _scene_restore(self, scenes: list) -> None:
        for scene in scenes:
            scene_id = scene["scene_id"]
            patch = {}
            if scene.get("name"):
                patch["name"] = scene["name"]
            if "ha_select_enabled" in scene:
                patch["ha_select_enabled"] = scene["ha_select_enabled"]
            if patch:
                self.scenes.patch(scene_id, patch)
            rows = [{"virtual_lamp_id": r["virtual_lamp_id"],
                     "desired": {k: v for k, v in r["desired"].items() if k != "waf"}}
                    for r in scene.get("rows") or []]
            if not rows:
                continue
            self.scenes.matrix_patch(scene_id, rows)
            res = self.scenes.apply(scene_id)
            if "operation_id" in res:
                self.wait_op(res)

    def config_restore(self, snap: dict) -> None:
        current = {v["virtual_lamp_id"]: v
                   for v in self.vlamps.list()["virtual_lamps"]}
        for v in snap["vl"]["virtual_lamps"]:
            lid = v["virtual_lamp_id"]
            want = (v.get("binding") or {}).get("physical_short_address")
            have = (current.get(lid, {}).get("binding")
                    or {}).get("physical_short_address")
            if want is None and have is not None:
                self.vlamps.unbind(lid)
            elif want is not None and want != have:
                self.vlamps.bind(lid, want)
        rows = [{"virtual_lamp_id": r["virtual_lamp_id"], "desired": r["desired"]}
                for r in snap["group_matrix"]["rows"]]
        if rows:
            self.groups.matrix_patch(rows)
        res = self.groups.apply()
        if "operation_id" in res:
            self.wait_op(res)
        self._scene_restore(snap.get("scenes") or [])

    def snapshot_states(self):
        return [{"short_address": d["short_address"], "state": d.get("state")}
                for d in self.devices()["physical_devices"]]

    def state_divergences(self, snapshot):
        live = {d["short_address"]: (d.get("state") or {})
                for d in self.devices()["physical_devices"]}
        compared = ("power", "level", "color_mode", "color_temperature_kelvin", "rgb")
        out = []
        for entry in snapshot:
            state = entry.get("state") or {}
            short = entry["short_address"]
            now = live.get(short)
            if now is None or all(now.get(k) == state.get(k) for k in compared):
                continue
            out.append("SA%d %s -> %s" % (
                short,
                {k: state.get(k) for k in compared},
                {k: now.get(k) for k in compared}))
        return out

    def _reread_raw_touched(self, snapshot):
        touched, self.raw_touched = self.raw_touched, set()
        if self.TOUCHED_ALL in touched:
            touched = {entry["short_address"] for entry in snapshot}
        for short in sorted(touched):
            try:
                self.attr_read_checked(short, groups="runtime_status,dt8_color")
            except ApiError:
                pass

    def restore_states(self, snapshot):
        self._reread_raw_touched(snapshot)
        live = {d["short_address"]: (d.get("state") or {})
                for d in self.devices()["physical_devices"]}
        compared = ("power", "level", "color_mode", "color_temperature_kelvin", "rgb")
        for entry in snapshot:
            state = entry.get("state") or {}
            short = entry["short_address"]
            now = live.get(short)
            if now is not None and all(now.get(k) == state.get(k) for k in compared):
                continue
            try:
                if state.get("power") == "on":
                    setpoint = {"power": "on"}
                    if state.get("level") is not None:
                        setpoint["level"] = state["level"]
                    mode = state.get("color_mode")
                    if mode == "rgb" and state.get("rgb"):
                        setpoint.update(color_mode="rgb", rgb=state["rgb"])
                    elif mode == "cct" and state.get("color_temperature_kelvin"):
                        setpoint.update(color_mode="cct",
                                        color_temperature_kelvin=state["color_temperature_kelvin"])
                    self.ts(short, setpoint)
                else:
                    self.off(short)
            except CapabilityUnsupported:
                try:
                    self.off(short)
                except ApiError:
                    pass
            except ApiError:
                continue
            except LampNotAllowed as exc:
                print("restore_states: %s" % exc)



class _Namespace:
    def __init__(self, client: "Client"):
        self._c = client


class _HclSchedules(_Namespace):
    def list(self) -> list:
        return self._c._req("GET", "hcl-schedules")["schedules"]

    def get(self, schedule_id: str) -> dict:
        return self._c._req("GET", "hcl-schedules/%s" % schedule_id)

    def create(self, body: dict) -> dict:
        return self._c._config_write("POST", "hcl-schedules", body)

    def patch(self, schedule_id: str, body: dict) -> dict:
        return self._c._config_write("PATCH", "hcl-schedules/%s" % schedule_id, body)

    def delete(self, schedule_id: str):
        return self._c._req("DELETE", "hcl-schedules/%s" % schedule_id)

    def override(self, schedule_id: str) -> dict:
        return self._c._req("GET", "hcl-schedules/%s/override" % schedule_id)

    def clear_override(self, schedule_id: str):
        return self._c._req("DELETE", "hcl-schedules/%s/override" % schedule_id)


class _Firmware(_Namespace):
    def get(self) -> dict:
        return self._c._req("GET", "firmware")

    def update(self, url: str) -> dict:
        return self._c._req("POST", "firmware/updates", {"url": url})


class _PollerSettings(_Namespace):
    RESTORABLE = ("attribute_groups_default", "enabled", "include_diagnostics",
                  "include_dt8_color", "include_energy", "interval_ms",
                  "skip_unbound_virtual_lamps")

    def get(self) -> dict:
        return self._c._req("GET", "settings/poller")

    def patch(self, body: dict) -> dict:
        return self._c._req("PATCH", "settings/poller", body)


class _DaliSettings(_Namespace):
    def get(self) -> dict:
        return self._c._req("GET", "settings/dali")

    def patch(self, body: dict) -> dict:
        return self._c._req("PATCH", "settings/dali", body)


class _Redundancy(_Namespace):
    def get(self) -> dict:
        return self._c._req("GET", "redundancy")

    def settings(self) -> dict:
        return self._c._req("GET", "settings/redundancy")

    def patch_settings(self, body: dict) -> dict:
        return self._c._req("PATCH", "settings/redundancy", body)

    def switchover(self) -> dict:
        return self._c._req("POST", "redundancy/switchover", {})


class _ConfigSlices(_Namespace):
    def manifest(self) -> list:
        return self._c._req("GET", "config/slices")

    def get(self, name: str) -> bytes:
        from urllib.parse import quote
        return self._c.raw_response("GET", "config/slices/%s" % quote(name, safe="")).content


class _HomeAssistantSettings(_Namespace):
    RESTORABLE = (
        "enabled", "broker_host", "broker_port", "broker_username",
        "discovery_prefix", "state_topic_prefix", "controller_id",
        "publish_qos", "retain_state", "retain_discovery",
    )

    def get(self) -> dict:
        return self._c._req("GET", "settings/home-assistant")

    def patch(self, body: dict) -> dict:
        return self._c._req("PATCH", "settings/home-assistant", body)

    def discovery_publish(self) -> dict:
        res = self._c._req("POST", "settings/home-assistant/discovery-publish", {})
        return self._c.wait_op(res)


class _Groups(_Namespace):
    def list(self) -> dict:
        return self._c._req("GET", "adapters/%d/groups" % self._c.adapter)

    def get(self, group_id: int) -> dict:
        return self._c._req("GET", "adapters/%d/groups/%d"
                            % (self._c.adapter, group_id))

    def patch(self, group_id: int, patch: dict) -> dict:
        return self._c._req("PATCH", "adapters/%d/groups/%d"
                            % (self._c.adapter, group_id), patch)

    def matrix(self) -> dict:
        return self._c._req("GET", "adapters/%d/group-membership-matrix"
                            % self._c.adapter)

    def matrix_patch(self, rows: list) -> dict:
        return self._c._config_write("PATCH", "adapters/%d/group-membership-matrix"
                                     % self._c.adapter, {"rows": rows})

    def matrix_put(self, rows: list) -> dict:
        return self._c._config_write("PUT", "adapters/%d/group-membership-matrix"
                                     % self._c.adapter, {"rows": rows})

    def apply(self) -> dict:
        return self._c._req("POST", "adapters/%d/groups/apply" % self._c.adapter)

    def ts(self, group_id: int, setpoint: dict) -> dict:
        return self._c._req("PUT", "adapters/%d/groups/%d/target-state"
                            % (self._c.adapter, group_id), setpoint)

    def join(self, lamp_ids: list, group_id: int, timeout_s: float = 60.0,
             check=None, apply: bool = True):
        rows = {r["virtual_lamp_id"]: r for r in self.matrix()["rows"]}
        patch = []
        for lamp_id in lamp_ids:
            desired = list(rows[lamp_id]["desired"])
            desired[group_id] = True
            patch.append({"virtual_lamp_id": lamp_id, "desired": desired})
        self.matrix_patch(patch)
        if not apply:
            return patch
        res = self.apply()
        if "operation_id" in res:
            view = self._c.wait_op(res, timeout_s=timeout_s)
            if check is not None:
                check(view)
            elif view.get("status") != "succeeded":
                raise AssertionError("groups.join apply did not succeed: %s"
                                     % view)
            return view
        return res


class _Scenes(_Namespace):
    def list(self) -> dict:
        return self._c._req("GET", "adapters/%d/scenes" % self._c.adapter)

    def get(self, scene_id: int) -> dict:
        return self._c._req("GET", "adapters/%d/scenes/%d"
                            % (self._c.adapter, scene_id))

    def patch(self, scene_id: int, patch: dict) -> dict:
        return self._c._req("PATCH", "adapters/%d/scenes/%d"
                            % (self._c.adapter, scene_id), patch)

    def matrix(self, scene_id: int) -> dict:
        return self._c._req("GET", "adapters/%d/scenes/%d/matrix"
                            % (self._c.adapter, scene_id))

    def matrix_patch(self, scene_id: int, rows: list) -> dict:
        return self._c._config_write("PATCH", "adapters/%d/scenes/%d/matrix"
                                     % (self._c.adapter, scene_id), {"rows": rows})

    def matrix_put(self, scene_id: int, rows: list) -> dict:
        return self._c._config_write("PUT", "adapters/%d/scenes/%d/matrix"
                                     % (self._c.adapter, scene_id), {"rows": rows})

    def apply(self, scene_id: int) -> dict:
        return self._c._req("POST", "adapters/%d/scenes/%d/apply"
                            % (self._c.adapter, scene_id))

    def recall(self, scene_id: int, group_id=None) -> dict:
        body = None
        if group_id is not None:
            body = {"scope": "group", "group_id": group_id}
        return self._c._req("POST", "adapters/%d/scenes/%d/recall"
                            % (self._c.adapter, scene_id), body)


class _VirtualLamps(_Namespace):
    def list(self) -> dict:
        body = self._c._req("GET", "adapters/%d/virtual-lamps" % self._c.adapter)
        wanted = self._c.cfg.gear_short_set()
        if wanted is None or not isinstance(body, dict):
            return body
        def _kept(v):
            bound = (v.get("binding") or {}).get("physical_short_address")
            return bound is None or bound in wanted
        return dict(body, virtual_lamps=[v for v in body.get("virtual_lamps", [])
                                         if _kept(v)])

    def get(self, lamp_id: int) -> dict:
        return self._c._req("GET", "adapters/%d/virtual-lamps/%d"
                            % (self._c.adapter, lamp_id))

    def patch(self, lamp_id: int, patch: dict) -> dict:
        return self._c._req("PATCH", "adapters/%d/virtual-lamps/%d"
                            % (self._c.adapter, lamp_id), patch)

    def bind(self, lamp_id: int, short: int) -> dict:
        return self._c._req("PUT", "adapters/%d/virtual-lamps/%d/binding"
                            % (self._c.adapter, lamp_id),
                            {"physical_short_address": short})

    def unbind(self, lamp_id: int) -> dict:
        return self._c._req("DELETE", "adapters/%d/virtual-lamps/%d/binding"
                            % (self._c.adapter, lamp_id))

    def ts(self, lamp_id: int, setpoint: dict) -> dict:
        return self._c._req("PUT", "adapters/%d/virtual-lamps/%d/target-state"
                            % (self._c.adapter, lamp_id), setpoint)
