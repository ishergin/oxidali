import time


def wait_until(predicate, timeout_s: float, interval_s: float = 0.5,
               desc: str = None):
    deadline = time.monotonic() + timeout_s
    value = None
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(interval_s)
    if desc:
        print("hil: gave up waiting for %s after %.1fs" % (desc, timeout_s))
    return value


def settled(read, quiet_s: float, max_s: float, poll_s: float, key=len):
    deadline = time.monotonic() + max_s
    count, still_since = object(), time.monotonic()
    while time.monotonic() < deadline:
        now = key(read())
        if now != count:
            count, still_since = now, time.monotonic()
        elif time.monotonic() - still_since >= quiet_s:
            break
        time.sleep(poll_s)
    return read()
