import json
import subprocess

from hil.sniffer import ssh_argv


def _sh_quote(s: str) -> str:
    return "'" + s.replace("'", "'\\''") + "'"


def retained(cfg, topic, timeout_s: float = 3.0):
    cmd = "mosquitto_sub -C 1 -W %d -t %s" % (max(1, int(timeout_s)), _sh_quote(topic))
    proc = subprocess.run(ssh_argv(cfg, cmd), capture_output=True, timeout=timeout_s + 10)
    out = proc.stdout.decode("utf-8", "replace").strip()
    return out if out else None


def retained_json(cfg, topic, timeout_s: float = 3.0):
    raw = retained(cfg, topic, timeout_s)
    if raw is None:
        return None
    return json.loads(raw)


def publish(cfg, topic, payload, retain: bool = False):
    flag = " -r" if retain else ""
    cmd = "mosquitto_pub%s -t %s -m %s" % (flag, _sh_quote(topic), _sh_quote(payload))
    subprocess.run(ssh_argv(cfg, cmd), check=True, capture_output=True, timeout=15)


def clear_retained(cfg, topic):
    cmd = "mosquitto_pub -r -n -t %s" % _sh_quote(topic)
    subprocess.run(ssh_argv(cfg, cmd), check=True, capture_output=True, timeout=15)


def collect_retained(cfg, topic_filter, window_s: float = 2.0):
    cmd = "mosquitto_sub -v -W %d -t %s" % (max(1, int(window_s)), _sh_quote(topic_filter))
    proc = subprocess.run(ssh_argv(cfg, cmd), capture_output=True, timeout=window_s + 10)
    topics = []
    for line in proc.stdout.decode("utf-8", "replace").splitlines():
        line = line.strip()
        if line:
            topics.append(line.split(" ", 1)[0])
    return sorted(set(topics))


def collect_live(cfg, topic_filter, during, window_s: float = 6.0,
                 attach_s: float = 1.5, stimulus_cap_s: float = 60.0):
    import threading
    import time

    cap = max(1, int(attach_s + stimulus_cap_s + window_s))
    cmd = "mosquitto_sub -v -W %d -t %s" % (cap, _sh_quote(topic_filter))
    lines = []

    proc = subprocess.Popen(ssh_argv(cfg, cmd), stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL)

    def _read():
        for raw in proc.stdout:
            lines.append(raw.decode("utf-8", "replace").rstrip("\n"))

    reader = threading.Thread(target=_read, daemon=True)
    reader.start()
    time.sleep(attach_s)
    try:
        during()
        time.sleep(window_s)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        reader.join(timeout=10)

    seen = []
    for line in lines:
        line = line.strip()
        if not line:
            continue
        topic, _, payload = line.partition(" ")
        seen.append((topic, payload))
    return seen
