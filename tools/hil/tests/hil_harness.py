import time

from hil import api as api_mod

ANCHOR_TZ = "UTC0"

UPTIME_SLACK_S = 30.0


def validity_of(config):
    return config._hil_validity


def track(config, obj):
    config._hil_counted.append(obj)
    return obj


def peer_health(peer_cfg):
    try:
        health = api_mod.Client(peer_cfg).health()
        return health, (time.time(), float(health["uptime_seconds"]))
    except Exception:
        return None, None
