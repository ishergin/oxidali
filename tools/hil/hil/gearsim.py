import re
import time

try:
    import serial
except ImportError:
    serial = None


class GearSimUnavailable(RuntimeError):
    pass


_REPLY_QUIET_S = 0.4
_REPLY_TIMEOUT_S = 5.0

_KV = re.compile(r"(\w+)=(-?\d+)")


class GearSim:
    def __init__(self, port, baud=115200):
        if serial is None:
            raise GearSimUnavailable("pyserial is not installed")
        if not port:
            raise GearSimUnavailable("HIL_GEAR_SIM_PORT is not set")
        try:
            self._port = serial.Serial(port, baud, timeout=0.1)
        except Exception as exc:
            raise GearSimUnavailable("cannot open %s: %s" % (port, exc)) from exc
        time.sleep(1.5)
        self._port.reset_input_buffer()

    def close(self):
        try:
            self._port.close()
        except Exception:
            pass

    def command(self, verb):
        self._port.reset_input_buffer()
        self._port.write((verb + "\n").encode())
        self._port.flush()

        lines = []
        deadline = time.monotonic() + _REPLY_TIMEOUT_S
        last = time.monotonic()
        while time.monotonic() < deadline:
            raw = self._port.readline()
            if not raw:
                if lines and time.monotonic() - last > _REPLY_QUIET_S:
                    break
                continue
            text = raw.decode(errors="replace").strip()
            last = time.monotonic()
            if text.startswith("#"):
                lines.append(text.lstrip("# ").rstrip())
        return lines

    def stats(self):
        values = {}
        for line in self.command("stats"):
            for key, number in _KV.findall(line):
                values[key] = int(number)
        return values

    def settle_bands(self):
        bands = {}
        for line in self.command("stats"):
            match = re.match(r"priority (\d) (\d+)", line)
            if match:
                bands[int(match.group(1))] = int(match.group(2))
        return bands

    def violations(self):
        stats = self.stats()
        return {
            key: stats.get(key, 0)
            for key in ("enable_consumed", "below_p1_floor", "send_twice_interloper")
        }
