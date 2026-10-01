import time

from hil import api as api_mod
from hil import config as config_mod

ANCHOR_TZ = "UTC0"

UPTIME_SLACK_S = 30.0


def validity_of(config):
    return config._hil_validity


def track(config, obj):
    config._hil_counted.append(obj)
    return obj


def standalone_client(config):
    client = getattr(config, "_hil_standalone_client", None)
    if client is None:
        client = track(config, api_mod.Client(config_mod.load()))
        config._hil_standalone_client = client
    return client


def peer_health(peer_cfg):
    try:
        health = api_mod.Client(peer_cfg).health()
        return health, (time.time(), float(health["uptime_seconds"]))
    except Exception:
        return None, None
