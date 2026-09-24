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
