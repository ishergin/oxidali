import collections
import json
import subprocess
import time
from pathlib import Path

from hil.lamp_guard import LampGuard, LampNotAllowed
from hil.sniffer import ssh_argv

WS_PORT = 8080
TEST_RULE_PREFIX = "hil-"
COMMAND_BIT = 0x01
POWER_NOTIFICATION = 0xFE
POWER_NOTIFICATION_TOP = 0xE0
ADDRESS_VALID = 0x40
SHORT_MASK = 0x3F
FIELD_MASK = 0x1F
DEVICE_SCHEMES = (0, 1)
GROUP_SCHEME_TOP = 3
INSTANCE_TRIGGERS = frozenset({"input_event", "input_occupancy", "input_light_cross",
                               "input_position_change"})
POWER_CYCLED = "input_device_power_cycled"

_REMOTE_CLIENT = r"""
import asyncio, json, sys
import websockets

async def main():
    frames = json.loads(sys.argv[1])
    async with websockets.connect("ws://127.0.0.1:%d") as ws:
        greet = json.loads(await asyncio.wait_for(ws.recv(), 5))
        if not frames:
            print(json.dumps(greet)); return
        for f in frames:
            msg = {"type": "daliFrame", "data": {
                "numberOfBits": f["bits"],
                "daliData": f["bytes"],
                "line": %d,
                "mode": {"sendTwice": bool(f.get("send_twice")),
                         "priority": 0,
                         "waitForAnswer": bool(f.get("wait_answer"))}}}
            await ws.send(json.dumps(msg))
            print(json.dumps(json.loads(await asyncio.wait_for(ws.recv(), 8))))

asyncio.run(main())
"""


def frame_hex(frame):
    return "".join("%02x" % b for b in frame["bytes"])


def frames_on_wire(lines, frames):
    wanted = collections.Counter(frame_hex(f) for f in frames)
    requested = any(">>" in line and "(from lunatone)" in line for line in lines)
    seen, errors, pending = 0, [], []
    for line in lines:
        body = line.split("bus_monitor", 1)[-1].strip()
        if body.startswith(">>") and "(from lunatone)" in body:
            pending.append(body[2:].split()[0] if body[2:].split() else "")
            continue
        if not body.startswith("<<"):
            continue
        text = body[2:].strip()
        token = text.split()[0] if text.split() else ""
        if "gateway" in text:
            errors.append(text)
            if pending:
                pending.pop(0)
        elif requested and pending and token == pending[0]:
            pending.pop(0)
            if wanted[token] > 0:
                wanted[token] -= 1
                seen += 1
        elif not requested and wanted[token] > 0:
            wanted[token] -= 1
            seen += 1
    return seen, errors


def event_source24(raw):
    b0, b1, b2 = raw
    if b0 & COMMAND_BIT:
        return None
    if b0 == POWER_NOTIFICATION and b1 & POWER_NOTIFICATION_TOP == POWER_NOTIFICATION_TOP:
        return {"power": b2 & SHORT_MASK if b2 & ADDRESS_VALID else None}
    top, numbered = b0 >> 6, b1 >> 7
    if top in DEVICE_SCHEMES:
        return {"short": b0 >> 1 & SHORT_MASK,
                "instance": b1 >> 2 & FIELD_MASK if numbered else None}
    if top == GROUP_SCHEME_TOP and numbered:
        return None
    return {"group": b0 >> 1 & FIELD_MASK if top == GROUP_SCHEME_TOP else None}


def _fires(trigger, source, adapter):
    if trigger.get("adapter_id", adapter) != adapter:
        return False
    kind = trigger.get("kind")
    if "power" in source:
        return kind == POWER_CYCLED and source["power"] is not None and \
            trigger.get("device_short_address") == source["power"]
    if kind not in INSTANCE_TRIGGERS:
        return False
    if "instance_group" in trigger:
        return source.get("instance") is not None or \
            source.get("group") == trigger["instance_group"]
    return source.get("instance") is not None and \
        (trigger.get("device_short_address"), trigger.get("instance_number")) == \
        (source.get("short"), source["instance"])


def injection_refusal(raw, compiled, adapter):
    source = event_source24(raw)
    if source is None:
        return "it is a command to input devices or no event at all, which the guard cannot judge"
    fired = sorted({rule.get("name") for rule in (compiled or {}).get("rules") or []
                    if rule.get("enabled", True)
                    and not str(rule.get("name")).startswith(TEST_RULE_PREFIX)
                    and any(_fires(t, source, adapter) for t in rule.get("triggers") or [])})
    if fired:
        return "the owner's rule(s) %s could fire on the event it carries" % fired
    return None


class ForeignMasterError(RuntimeError):
    pass


class ForeignMasterSilent(ForeignMasterError):
    pass


class ForeignMaster:
    MAX_SENDS = 3
    WITNESS_TIMEOUT_S = 3.0
    QUIET_S = 0.7
    MONITOR_ARM_S = 1.5
    CLIENT_TIMEOUT_S = 30
    QUIESCE_TIMEOUT_S = 4.0

    def __init__(self, cfg, api=None):
        self.cfg = cfg
        self.line = cfg.wb_bus - 1
        self.api = api
        self.retries = collections.Counter()
        self.guard = LampGuard.for_config(
            cfg, segment=api.segment_shorts if api is not None else None)

    def _run_client(self, frames):
        script = _REMOTE_CLIENT % (WS_PORT, self.line)
        remote = "python3 -c %s %s" % (_shq(script), _shq(json.dumps(frames)))
        proc = subprocess.run(ssh_argv(self.cfg, remote),
                              capture_output=True, text=True, timeout=30)
        if proc.returncode != 0:
            raise ForeignMasterError(
                "lunatone ws client failed: %s" % proc.stderr.strip()[:300])
        return [json.loads(line) for line in proc.stdout.splitlines() if line]

    def probe(self):
        greet = self._run_client([])[0]
        doc = {"transport": "lunatone-ws", "host": self.cfg.wb_ssh,
               "port": WS_PORT, "greet": greet}
        (Path(self.cfg.state_dir) / "wb_master_controls.json").write_text(
            json.dumps(doc, indent=1, sort_keys=True))
        return greet

    def _forward_counts(self):
        try:
            phy = self.api.diagnostics()["phy_sniffer"]
            return {16: int(phy["forward16"]), 24: int(phy["forward24"])}
        except Exception:
            return None

    def _wait_quiet(self, timeout_s):
        deadline = time.monotonic() + timeout_s
        last, since = self._forward_counts(), time.monotonic()
        while time.monotonic() < deadline:
            time.sleep(0.15)
            now = self._forward_counts()
            if now != last:
                last, since = now, time.monotonic()
            elif time.monotonic() - since >= self.QUIET_S:
                break
        return last

    def _witness(self, before, width):
        deadline = time.monotonic() + self.WITNESS_TIMEOUT_S
        seen = 0
        while time.monotonic() < deadline:
            now = self._forward_counts()
            if now is not None and before is not None:
                seen = now[width] - before[width]
                if seen >= 1:
                    return seen
            time.sleep(0.15)
        return seen

    def _arm_monitor(self):
        topic = "/wb-dali/%s_bus_%d/bus_monitor" % (self.cfg.wb_device, self.cfg.wb_bus)
        window = int(self.MONITOR_ARM_S + self.CLIENT_TIMEOUT_S + self.WITNESS_TIMEOUT_S + 1)
        remote = "timeout %d mosquitto_sub -v -t %s" % (window, _shq(topic))
        try:
            proc = subprocess.Popen(ssh_argv(self.cfg, remote), stdout=subprocess.PIPE,
                                    stderr=subprocess.DEVNULL, text=True)
        except OSError:
            return None
        time.sleep(self.MONITOR_ARM_S)
        return proc

    def _monitor_verdict(self, monitor, frames):
        if monitor is None:
            return None
        time.sleep(self.WITNESS_TIMEOUT_S)
        out = _stop_monitor(monitor)
        if out is None:
            return None
        lines = out.splitlines()
        return frames_on_wire(lines, frames) if lines else None

    def _send_once(self, frames):
        replies = self._run_client(frames)
        for reply in replies:
            data = reply.get("data") or {}
            if reply.get("type") == "daliMonitor" and data.get("framingError"):
                raise ForeignMasterError("framing error on %s" % data)
        return replies

    @staticmethod
    def _normalize(frames):
        out = []
        for f in frames:
            wide = "bytes" in f
            out.append({
                "bits": f.get("bits", 24 if wide else 16),
                "bytes": list(f["bytes"]) if wide else [f["addr"], f["data"]],
                "send_twice": bool(f.get("send_twice")),
                "wait_answer": bool(f.get("wait_answer")),
            })
        return out

    def _check(self, frames):
        for frame in frames:
            if frame["bits"] == 16:
                self.guard.check_frame(*frame["bytes"])
            else:
                self._check_injection(frame["bytes"])

    def _check_injection(self, raw):
        what = "24-bit frame %s" % "".join("%02X" % b for b in raw)
        if self.api is None:
            raise LampNotAllowed("%s refused: no controller lists the rules it could fire" % what)
        compiled = self.api._req("GET", "rules?format=json").get("rules")
        refusal = injection_refusal(raw, compiled, self.api.adapter)
        if refusal:
            raise LampNotAllowed("%s refused: %s" % (what, refusal))

    def send_frames(self, frames):
        frames = self._normalize(frames)
        self._check(frames)
        if self.api is None or not frames:
            return self._send_once(frames)

        widths = {f["bits"] for f in frames}
        if len(widths) != 1:
            raise ForeignMasterError(
                "a batch mixes frame widths %r; send them separately so each "
                "is witnessed by its own counter" % sorted(widths))
        width = widths.pop()

        expected = len(frames)
        gateway_errors = []
        for attempt in range(self.MAX_SENDS):
            before = self._wait_quiet(self.QUIESCE_TIMEOUT_S)
            monitor = self._arm_monitor()
            try:
                replies = self._send_once(frames)
            except BaseException:
                if monitor is not None:
                    _stop_monitor(monitor)
                raise
            verdict = self._monitor_verdict(monitor, frames)
            if verdict is not None:
                seen, gateway_errors = verdict
                if seen >= expected:
                    return replies
            elif before is None:
                return replies
            elif self._witness(before, width) >= 1:
                return replies
            if attempt + 1 >= self.MAX_SENDS:
                raise ForeignMasterSilent(
                    "wb-mqtt-dali accepted %d frame(s) over WS and they did not "
                    "all reach the wire within %.1fs, over %d attempts%s. "
                    "The bench foreign master is jammed, not the firmware — "
                    "`systemctl restart wb-mqtt-dali` on %s (STRATEGY.md §2)."
                    % (expected, self.WITNESS_TIMEOUT_S, self.MAX_SENDS,
                       "; the WB's own monitor said: %s" % "; ".join(gateway_errors[:3])
                       if gateway_errors else "",
                       self.cfg.wb_ssh))
            self.retries["foreign_master_silent"] += 1
            print("foreign master silent: not all of a %d-frame batch reached the wire, "
                  "re-sending (attempt %d/%d)"
                  % (expected, attempt + 2, self.MAX_SENDS))

    def dapc(self, short, level):
        return self.send_frames([{"addr": short << 1, "data": level}])

    def group_dapc(self, group_id, level):
        return self.send_frames([{"addr": 0x80 | (group_id << 1),
                                  "data": level}])

    def broadcast_dapc(self, level):
        return self.send_frames([{"addr": 0xFE, "data": level}])

    def cmd(self, short, opcode, send_twice=False, wait_answer=False):
        return self.send_frames([{"addr": (short << 1) | 1, "data": opcode,
                                  "send_twice": send_twice,
                                  "wait_answer": wait_answer}])

    def raw16(self, addr_byte, data_byte, send_twice=False):
        return self.send_frames([{"addr": addr_byte, "data": data_byte,
                                  "send_twice": send_twice}])

    def raw24(self, byte0, byte1, byte2, send_twice=False):
        return self.send_frames([{"bits": 24,
                                  "bytes": [byte0, byte1, byte2],
                                  "send_twice": send_twice}])

    def event24(self, short, instance, info):
        if not 0 <= short <= 63:
            raise ValueError("short address out of the 103 space: %r" % short)
        if not 0 <= instance <= 31:
            raise ValueError("instance number out of range: %r" % instance)
        if not 0 <= info <= 0x3FF:
            raise ValueError("event info is 10 bits: %r" % info)
        return self.raw24(short << 1,
                          0x80 | (instance << 2) | ((info >> 8) & 0x03),
                          info & 0xFF)

    BUTTON_EVENTS = {
        "release": 0x000,
        "press": 0x001,
        "short_press": 0x002,
        "double_press": 0x005,
        "long_press_start": 0x009,
        "long_press_repeat": 0x00B,
        "long_press_stop": 0x00C,
        "button_free": 0x00E,
        "button_stuck": 0x00F,
    }

    def button24(self, short, instance, event):
        try:
            info = self.BUTTON_EVENTS[event]
        except KeyError:
            raise ValueError(
                "unknown button event %r; Table 2 defines %s"
                % (event, ", ".join(sorted(self.BUTTON_EVENTS)))) from None
        return self.event24(short, instance, info)

    def goto_scene(self, scene_id, addr_byte=0xFF):
        return self.send_frames([{"addr": addr_byte, "data": 0x10 + scene_id}])

    def set_cct(self, short, kelvin):
        mirek = (1_000_000 + kelvin // 2) // kelvin
        return self.send_frames([
            {"addr": 0xA3, "data": mirek & 0xFF},
            {"addr": 0xC3, "data": (mirek >> 8) & 0xFF},
            {"addr": 0xC1, "data": 8},
            {"addr": (short << 1) | 1, "data": 0xE7},
            {"addr": 0xC1, "data": 8},
            {"addr": (short << 1) | 1, "data": 0xE2},
        ])

    def send_frames_one_at_a_time(self, frames):
        self._check(self._normalize(frames))
        out = []
        for frame in frames:
            echo = self.send_frames([frame])
            if isinstance(echo, list):
                out.extend(echo)
        return out

    def set_rgb(self, short, r, g, b):
        ind = (short << 1) | 1
        return self.send_frames_one_at_a_time([
            {"addr": 0xA3, "data": 0x80},
            {"addr": 0xC1, "data": 8},
            {"addr": ind, "data": 0xED},
            {"addr": 0xA3, "data": r & 0xFF},
            {"addr": 0xC3, "data": g & 0xFF},
            {"addr": 0xC5, "data": b & 0xFF},
            {"addr": 0xC1, "data": 8},
            {"addr": ind, "data": 0xEB},
            {"addr": 0xA3, "data": 0},
            {"addr": 0xC3, "data": 0},
            {"addr": 0xC5, "data": 0},
            {"addr": 0xC1, "data": 8},
            {"addr": ind, "data": 0xEC},
            {"addr": 0xC1, "data": 8},
            {"addr": ind, "data": 0xE2},
        ])


def _stop_monitor(monitor):
    monitor.terminate()
    try:
        out, _ = monitor.communicate(timeout=10)
    except subprocess.TimeoutExpired:
        monitor.kill()
        out, _ = monitor.communicate()
    return out


def _shq(s):
    return "'" + s.replace("'", "'\\''") + "'"
