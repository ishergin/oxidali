import dataclasses
import os
import time
from dataclasses import dataclass, field
from pathlib import Path

HIL_ROOT = Path(__file__).resolve().parent.parent


def _env(name, default):
    return os.environ.get(name, default)


def _parse_shorts(spec: str, name: str) -> frozenset:
    out = set()
    for part in spec.split(","):
        part = part.strip()
        if not part:
            continue
        if "-" in part:
            lo, hi = part.split("-", 1)
            out.update(range(int(lo), int(hi) + 1))
        else:
            out.add(int(part))
    if not out:
        raise ValueError("%s resolves to no addresses: %r" % (name, spec))
    return frozenset(out)


def _serialport():
    from hil import serialport

    return serialport


class PeerUnconfigured(RuntimeError):
    pass


def _read_pin(path: Path) -> str:
    try:
        return path.read_text().strip()
    except OSError:
        return ""


@dataclass(frozen=True)
class HilConfig:
    base: str = field(default_factory=lambda: _env("HIL_BASE", "http://192.168.13.116"))
    adapter: int = field(default_factory=lambda: int(_env("HIL_ADAPTER", "0")))
    wb_ssh: str = field(default_factory=lambda: _env("HIL_WB_SSH", "root@192.168.13.110"))
    wb_device: str = field(default_factory=lambda: _env("HIL_WB_DEVICE", "wb-dali_19"))
    wb_bus: int = field(default_factory=lambda: int(_env("HIL_WB_BUS", "1")))
    mqtt_broker_host: str = field(
        default_factory=lambda: _env("HIL_MQTT_BROKER_HOST", "192.168.13.110"))
    mqtt_broker_port: int = field(
        default_factory=lambda: int(_env("HIL_MQTT_BROKER_PORT", "1883")))
    optical_shorts: str = field(
        default_factory=lambda: _env("HIL_OPTICAL_SHORTS", ""))

    def optical_short_set(self) -> frozenset:
        lamps = self.lamp_short_set()
        if not self.optical_shorts.strip():
            return lamps
        return _parse_shorts(self.optical_shorts, "HIL_OPTICAL_SHORTS") & lamps
    gear_shorts: str = field(
        default_factory=lambda: _env("HIL_GEAR_SHORTS", ""))

    def gear_short_set(self):
        if not self.gear_shorts.strip():
            return None
        return _parse_shorts(self.gear_shorts, "HIL_GEAR_SHORTS")
    serial_port: str = field(
        default_factory=lambda: _serialport().resolve(os.environ.get("HIL_SERIAL_PORT")))
    serial_port_pinned: bool = field(
        default_factory=lambda: bool(os.environ.get("HIL_SERIAL_PORT")))
    serial_baud: int = field(default_factory=lambda: int(_env("HIL_SERIAL_BAUD", "115200")))
    serial_remote: str = field(
        default_factory=lambda: _env(
            "HIL_SERIAL_REMOTE",
            "root@192.168.13.110:/dev/serial/by-id/"
            "usb-1a86_USB_Single_Serial_5B90157454-if00"))
    serial_bridge_port: int = field(
        default_factory=lambda: int(_env("HIL_SERIAL_BRIDGE_PORT", "4444")))
    flash_baud: int = field(default_factory=lambda: int(_env("HIL_FLASH_BAUD", "1500000")))
    gear_sim_port: str = field(default_factory=lambda: _env("HIL_GEAR_SIM_PORT", ""))
    lamp_shorts: str = field(default_factory=lambda: _env("HIL_LAMP_SHORTS", "0,2,3"))
    lamps_read_only: bool = field(
        default_factory=lambda: _env("HIL_LAMPS_READ_ONLY", "0") not in ("", "0", "false", "no"))

    def lamp_short_set(self) -> frozenset:
        if not self.lamp_shorts.strip():
            return frozenset()
        return _parse_shorts(self.lamp_shorts, "HIL_LAMP_SHORTS")
    board: str = field(default_factory=lambda: _env("HIL_BOARD", "esp32p4"))
    peer_base: str = field(default_factory=lambda: _env("HIL_PEER_BASE", ""))
    peer_serial_port: str = field(
        default_factory=lambda: _env("HIL_PEER_SERIAL_PORT", ""))
    peer_serial_remote: str = field(
        default_factory=lambda: _env(
            "HIL_PEER_SERIAL_REMOTE",
            "root@192.168.13.110:/dev/serial/by-id/"
            "usb-1a86_USB_Single_Serial_5B90038752-if00"))
    peer_serial_bridge_port: int = field(
        default_factory=lambda: int(_env("HIL_PEER_SERIAL_BRIDGE_PORT", "4446")))
    is_peer: bool = False
    camera_name: str = field(default_factory=lambda: _env("HIL_CAMERA_NAME", "USB Camera"))
    camera_id: str = field(default_factory=lambda: _env("HIL_CAMERA_ID", ""))

    root: Path = HIL_ROOT
    state_dir: Path = HIL_ROOT / "state"
    runs_dir: Path = HIL_ROOT / "runs"
    vendor_dir: Path = HIL_ROOT / "vendor"

    def ensure_dirs(self):
        self.state_dir.mkdir(parents=True, exist_ok=True)
        self.runs_dir.mkdir(parents=True, exist_ok=True)

    @property
    def uvc_util(self) -> Path:
        return self.vendor_dir / "uvc-util" / "uvc-util"

    @property
    def persist_serial_log(self) -> Path:
        return self.state_dir / "persist" / "serial.log"

    def new_run_dir(self) -> Path:
        self.ensure_dirs()
        ts = time.strftime("%Y%m%d-%H%M%S", time.gmtime())
        run = self.runs_dir / ts
        run.mkdir(parents=True, exist_ok=True)
        current = self.runs_dir / "current"
        if current.is_symlink() or current.exists():
            current.unlink()
        current.symlink_to(run)
        return run

    def run_dir(self) -> Path:
        explicit = os.environ.get("HIL_RUN_DIR")
        if explicit:
            p = Path(explicit)
            p.mkdir(parents=True, exist_ok=True)
            return p
        current = self.runs_dir / "current"
        if current.is_dir():
            return current.resolve()
        return self.new_run_dir()

    @property
    def base_pin_path(self) -> Path:
        return self.state_dir / "base.pin"

    @property
    def has_peer(self) -> bool:
        return bool(self.peer_base or self.peer_serial_port
                    or _read_pin(self.state_dir / "peer" / "base.pin"))

    def peer(self) -> "HilConfig":
        if self.is_peer:
            peer_state, peer_runs = self.state_dir.parent, self.runs_dir.parent
        else:
            peer_state, peer_runs = self.state_dir / "peer", self.runs_dir / "peer"
        base = self.peer_base or _read_pin(peer_state / "base.pin")
        if not (base or self.peer_serial_port):
            raise PeerUnconfigured(
                "no second controller is named: set HIL_PEER_BASE (its URL) "
                "and/or HIL_PEER_SERIAL_PORT (its local serial device)")
        return dataclasses.replace(
            self,
            base=base,
            serial_port=self.peer_serial_port or self.serial_port,
            serial_port_pinned=bool(self.peer_serial_port),
            serial_remote=self.peer_serial_remote,
            serial_bridge_port=self.peer_serial_bridge_port,
            state_dir=peer_state,
            runs_dir=peer_runs,
            peer_base=self.base,
            peer_serial_port="" if self.serial_remote else self.serial_port,
            peer_serial_remote=self.serial_remote,
            peer_serial_bridge_port=self.serial_bridge_port,
            is_peer=not self.is_peer,
        )


def load() -> HilConfig:
    cfg = HilConfig()
    if os.environ.get("HIL_PEER") == "1":
        cfg = cfg.peer()
    cfg.ensure_dirs()
    return cfg
