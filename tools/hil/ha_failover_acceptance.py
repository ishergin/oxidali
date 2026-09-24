from __future__ import annotations

import argparse
import json
import subprocess
import sys
import threading
import time
from pathlib import Path

DESCRIPTION = """Home Assistant across a failover: does the standby re-announce, and does a
Home Assistant command reach the wire while it holds the bus?

Holds the primary in its ROM loader through the bridge's control port for --hold
seconds (always released), then checks in order: the standby claims the bus, its
MQTT bridge connects, it re-announces discovery, availability returns online, a
command published the way Home Assistant does drives the first exposed bench
fixture (restored afterwards), and the primary takes the bus back and re-announces."""

sys.path.insert(0, str(Path(__file__).resolve().parent))

from hil import config as config_mod
from hil import remote_serial
from hil.api import Client
from hil.sniffer import ssh_argv
from hil.wait import wait_until

ALLOWED_SHORTS = (2, 3, 0)
TAKEOVER_MARGIN_S = 3.0
BRIDGE_CONNECT_S = 30.0
REANNOUNCE_S = 45.0
COMMAND_EFFECT_S = 12.0


class MqttStream(threading.Thread):
    def __init__(self, cfg, controller_id: str, state_prefix: str, out: Path):
        super().__init__(daemon=True)
        self.cfg = cfg
        self.out = out
        self.lines: list[tuple[float, str]] = []
        self.proc = None
        self.filters = [
            "homeassistant/+/%s/+/config" % controller_id,
            "%s/%s/#" % (state_prefix, controller_id),
        ]

    def run(self) -> None:
        topics = " ".join("-t '%s'" % f for f in self.filters)
        self.proc = subprocess.Popen(
            ssh_argv(self.cfg, "mosquitto_sub -v %s" % topics),
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
        with self.out.open("w", encoding="utf-8") as fh:
            for line in self.proc.stdout:
                stamp = time.monotonic()
                self.lines.append((stamp, line.rstrip()))
                fh.write("%.3f %s" % (stamp, line))
                fh.flush()

    def stop(self) -> None:
        if self.proc is not None:
            self.proc.terminate()

    def since(self, t0: float, needle: str) -> list[str]:
        return [line for stamp, line in self.lines if stamp >= t0 and needle in line]

    def configs_since(self, t0: float) -> set:
        topics = set()
        for stamp, line in self.lines:
            if stamp < t0:
                continue
            topic = line.split(" ", 1)[0]
            if topic.startswith("homeassistant/") and topic.endswith("/config"):
                topics.add(topic)
        return topics


def _mqtt(api: Client) -> dict:
    return api.diagnostics().get("mqtt", {})


def _role(api: Client) -> str:
    try:
        return api.health().get("role", "?")
    except Exception:
        return "unreachable"


def main() -> int:
    parser = argparse.ArgumentParser(description=DESCRIPTION)
    parser.add_argument("--hold", type=float, default=45.0,
                        help="seconds the primary stays halted")
    parser.add_argument("--dry-run", action="store_true",
                        help="check preconditions and the MQTT stream, halt nothing")
    args = parser.parse_args()

    cfg = config_mod.load()
    if not cfg.has_peer:
        raise SystemExit("no peer configured — this acceptance needs the pair")
    if not remote_serial.enabled(cfg):
        raise SystemExit("halting the primary needs the WB bridge's control port")

    api = Client(cfg)
    peer = Client(cfg.peer())
    if _role(api) != "active" or _role(peer) != "standby":
        raise SystemExit("expected primary active and peer standby, got %s / %s"
                         % (_role(api), _role(peer)))

    ha = api.ha_settings.get() if hasattr(api, "ha_settings") else api._req(
        "GET", "settings/home-assistant")
    peer_ha = peer._req("GET", "settings/home-assistant")
    if not ha.get("enabled") or not peer_ha.get("enabled"):
        raise SystemExit("both units need the HA bridge enabled (%s / %s)"
                         % (ha.get("enabled"), peer_ha.get("enabled")))
    if ha.get("controller_id") != peer_ha.get("controller_id"):
        raise SystemExit("the pair disagrees about controller_id (%s vs %s) — "
                         "replication (A10) is the precondition here"
                         % (ha.get("controller_id"), peer_ha.get("controller_id")))
    controller_id = ha["controller_id"]
    state_prefix = ha["state_topic_prefix"]

    before_primary = _mqtt(api)
    if not before_primary.get("connected"):
        raise SystemExit("the active unit's bridge is not connected — nothing to fail over")

    out_dir = Path(cfg.state_dir) / "ha_failover"
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    stream = MqttStream(cfg, controller_id, state_prefix, out_dir / ("mqtt-%s.log" % stamp))
    stream.start()
    time.sleep(4)

    timeline = {
        "controller_id": controller_id,
        "primary_discovery_before": before_primary.get("discovery_published_total"),
        "retained_configs_before": sorted(stream.configs_since(0)),
    }
    print("controller_id %s, %d config topics retained before"
          % (controller_id, len(timeline["retained_configs_before"])))

    if args.dry_run:
        stream.stop()
        print(json.dumps(timeline, indent=1, ensure_ascii=False))
        return 0

    settings = peer.redundancy.settings()
    bound_s = (settings["takeover_after_missed"] + 1) * settings["probe_interval_ms"] / 1000.0
    level_before = None
    try:
        level_before = api.state(ALLOWED_SHORTS[0]).get("state", {}).get("level")
    except Exception:
        pass

    verdict = {}
    try:
        with api.expect_reboot():
            remote_serial.control(cfg, "bootloader")
            t_halt = time.monotonic()
            claimed = wait_until(lambda: peer.redundancy.get()["active"],
                                 bound_s + TAKEOVER_MARGIN_S, 0.1)
            verdict["standby_claimed_s"] = round(time.monotonic() - t_halt, 2)
            verdict["standby_claimed"] = bool(claimed)
            if not claimed:
                raise SystemExit("the standby never took the bus — see HIL-RED-05")

            connected = wait_until(lambda: _mqtt(peer).get("connected"), BRIDGE_CONNECT_S, 0.5)
            verdict["bridge_connected"] = bool(connected)
            verdict["bridge_connected_s"] = round(time.monotonic() - t_halt, 2)

            expected = timeline["primary_discovery_before"] or 1
            announced = wait_until(
                lambda: (_mqtt(peer).get("discovery_published_total") or 0) >= expected,
                REANNOUNCE_S, 1.0)
            peer_mqtt = _mqtt(peer)
            verdict["discovery_published"] = peer_mqtt.get("discovery_published_total")
            verdict["discovery_expected_at_least"] = expected
            verdict["reannounced"] = bool(announced)
            verdict["configs_republished"] = len(stream.configs_since(t_halt))
            verdict["availability_online"] = bool(
                stream.since(t_halt, "%s/%s/availability online" % (state_prefix, controller_id)))

            vl, fixture_short = None, None
            for short in ALLOWED_SHORTS:
                for lamp in peer._req("GET", "adapters/0/virtual-lamps")["virtual_lamps"]:
                    binding = lamp.get("binding") or {}
                    if (binding.get("physical_short_address") == short
                            and lamp.get("ha_entity_enabled")):
                        vl, fixture_short = lamp["virtual_lamp_id"], short
                        break
                if vl is not None:
                    break
            if vl is None:
                verdict["command_reached_gear"] = ("no EXPOSED virtual lamp bound to any of %s"
                                                   % (ALLOWED_SHORTS,))
            else:
                topic = "%s/%s/a0/vl/%d/set" % (state_prefix, controller_id, vl)
                target_level = 120 if (level_before or 0) != 120 else 90
                verdict["fixture_short"] = fixture_short
                ha_brightness = max(1, round(target_level * 255 / 254))
                subprocess.run(
                    ssh_argv(cfg, "mosquitto_pub -t '%s' -m '%s'"
                             % (topic, json.dumps({"state": "ON",
                                                   "brightness": ha_brightness}))),
                    check=True, capture_output=True, timeout=15)
                t_cmd = time.monotonic()
                def _lit() -> bool:
                    st = peer.state(fixture_short).get("state", {})
                    return st.get("power") == "on" and (st.get("level") or 0) > 0

                moved = wait_until(_lit, COMMAND_EFFECT_S, 0.5)
                verdict["command_reached_gear"] = bool(moved)
                verdict["command_effect_s"] = round(time.monotonic() - t_cmd, 2)
                verdict["state_topic_seen"] = bool(
                    stream.since(t_cmd, "%s/%s/a0/vl/%d/state" % (state_prefix,
                                                                  controller_id, vl)))
                try:
                    peer.ts(fixture_short, {"power": "off"})
                except Exception as exc:
                    print("WARNING: SA%02d left on: %r" % (fixture_short, exc))

            remaining = args.hold - (time.monotonic() - t_halt)
            if remaining > 0:
                time.sleep(remaining)
    finally:
        try:
            remote_serial.control(cfg, "run")
        except Exception as exc:
            print("WARNING: the primary may still be halted: %r" % exc)
        t_run = time.monotonic()

    back = wait_until(lambda: _role(api) == "active", 90.0, 1.0)
    verdict["primary_back_active"] = bool(back)
    verdict["primary_back_s"] = round(time.monotonic() - t_run, 2)
    stood_down = wait_until(lambda: not peer.redundancy.get()["active"], 30.0, 0.5)
    verdict["standby_stood_down"] = bool(stood_down)
    reconnected = wait_until(lambda: _mqtt(api).get("connected"), BRIDGE_CONNECT_S, 1.0)
    verdict["primary_bridge_reconnected"] = bool(reconnected)
    verdict["primary_reannounced"] = wait_until(
        lambda: (_mqtt(api).get("discovery_published_total") or 0) > 0, REANNOUNCE_S, 1.0)
    verdict["primary_discovery_after"] = _mqtt(api).get("discovery_published_total")
    verdict["peer_bridge_after"] = _mqtt(peer).get("connected")

    time.sleep(3)
    stream.stop()
    timeline["verdict"] = verdict
    (out_dir / ("timeline-%s.json" % stamp)).write_text(
        json.dumps(timeline, indent=1, ensure_ascii=False), encoding="utf-8")

    print("\n--- Home Assistant across a failover ---")
    for key, value in verdict.items():
        print("  %-28s %s" % (key, value))
    print("mqtt stream: %s" % stream.out)

    passed = (verdict.get("standby_claimed") and verdict.get("bridge_connected")
              and verdict.get("reannounced") and verdict.get("configs_republished", 0) > 0
              and verdict.get("command_reached_gear") is True
              and verdict.get("primary_back_active") and verdict.get("standby_stood_down")
              and verdict.get("primary_bridge_reconnected"))
    print("\nVERDICT: %s" % ("PASS — Home Assistant follows the bus"
                             if passed else "FAIL / INCOMPLETE (see above)"))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
