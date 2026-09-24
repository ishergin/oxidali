import glob

SERIAL_VIDS = (
    0x303A,
    0x1A86,
    0x10C4,
    0x0403,
)

FALLBACK_PORT = "/dev/cu.usbmodem*"


def candidates():
    try:
        from serial.tools import list_ports
    except ImportError:
        return sorted(glob.glob("/dev/cu.usbmodem*"))
    found = [p.device for p in list_ports.comports() if p.vid in SERIAL_VIDS]
    return sorted(found)


def resolve(explicit=None):
    if explicit:
        return explicit
    found = candidates()
    return found[0] if found else FALLBACK_PORT


def describe(cfg, recorded=None):
    found = candidates()
    if cfg.serial_port_pinned:
        state = "pinned via HIL_SERIAL_PORT"
        if found and cfg.serial_port not in found:
            state += "; attached board ports: %s" % ", ".join(found)
        return "%s (%s)" % (cfg.serial_port, state)
    if recorded:
        state = "pinned by an earlier HIL_SERIAL_PORT run (state/serial_port.pin)"
        if found and recorded not in found:
            state += "; attached board ports: %s" % ", ".join(found)
        return "%s (%s)" % (recorded, state)
    if not found:
        state = "no board serial port attached"
    elif len(found) == 1:
        state = "auto-discovered"
    else:
        state = "auto-discovered, %d attached (%s) — pin HIL_SERIAL_PORT to choose" % (
            len(found), ", ".join(found))
    return "%s (%s)" % (cfg.serial_port, state)
