from hil.api import ApiError, Client
from hil.results import LampIdentity


def _identity_values(device):
    return device.get("gtin"), device.get("identification_number")


def collect(api: Client, ensure_read=True, only=None):
    out = []
    devices = api.devices()["physical_devices"]
    if only is not None:
        devices = [d for d in devices if d["short_address"] in only]
    for d in devices:
        gtin, idn = _identity_values(d)
        if (gtin is None or idn is None) and ensure_read:
            try:
                api.wait_op(api.attr_read(d["short_address"],
                                          groups="common_102", banks="identity"))
            except ApiError:
                pass
            refreshed = next((x for x in api.devices()["physical_devices"]
                              if x["short_address"] == d["short_address"]), d)
            gtin, idn = _identity_values(refreshed)
        out.append(LampIdentity(gtin=gtin, identification_number=idn,
                                short_address=d["short_address"],
                                label=d["short_address"]))
    return out


def restore_short_addresses(api: Client, desired):
    unmatched = dict(desired)
    for lamp in collect(api):
        want = unmatched.pop(lamp.key, None)
        if want is None or want == lamp.short_address:
            continue
        api.raw((0xA3 << 8) | ((want << 1) | 1))
        api.cmd(lamp.short_address, 0x80, repeat=2)
    return list(unmatched)


def _identity_index(api: Client):
    devices = api.devices()["physical_devices"]
    index = {}
    for d in devices:
        key = _identity_values(d)
        if key != (None, None):
            index[key] = d["short_address"]
    return index, {d["short_address"] for d in devices}


def _match(calibration_lamps, index, on_bus):
    mapping, missing = {}, []
    for lamp in calibration_lamps:
        ident = lamp.get("identity") or {}
        key = (ident.get("gtin"), ident.get("identification_number"))
        if key != (None, None) and key in index:
            mapping[lamp["label"]] = index[key]
        elif key == (None, None) and lamp["short_address"] in on_bus:
            mapping[lamp["label"]] = lamp["short_address"]
        else:
            missing.append(lamp["label"])
    return mapping, missing


def refresh_short_addresses(api: Client, calibration_lamps):
    index, on_bus = _identity_index(api)
    mapping, missing = _match(calibration_lamps, index, on_bus)
    if not missing:
        return mapping, missing

    suspects = sorted(on_bus - set(mapping.values()))
    if not suspects:
        return mapping, missing

    for short in suspects:
        try:
            api.wait_op(api.attr_read(short, groups="common_102",
                                      banks="identity"))
        except ApiError:
            continue
    index, on_bus = _identity_index(api)
    return _match(calibration_lamps, index, on_bus)
