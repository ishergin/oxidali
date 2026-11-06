import json
import subprocess

from hil.sniffer import SSH_OPTS

HAMMER = r"""
import json, sys, threading, time, urllib.request

base, seconds, threads = sys.argv[1], float(sys.argv[2]), int(sys.argv[3])
PATHS = ("physical-devices", "diagnostics", "stats")
REQUEST_TIMEOUT_S = 5
served = [0] * threads
failed = [0] * threads


def hammer(index, deadline):
    while time.time() < deadline:
        for path in PATHS:
            if time.time() >= deadline:
                return
            try:
                urllib.request.urlopen(base + "/api/v1/" + path,
                                       timeout=REQUEST_TIMEOUT_S).read()
                served[index] += 1
            except Exception:
                failed[index] += 1


print(json.dumps({"op": "started", "threads": threads}), flush=True)
deadline = time.time() + seconds
workers = [threading.Thread(target=hammer, args=(index, deadline))
           for index in range(threads)]
for worker in workers:
    worker.start()
for worker in workers:
    worker.join(seconds + 10)
print(json.dumps({"op": "report", "served": sum(served), "failed": sum(failed)}))
"""

START_TIMEOUT_S = 20.0

REPORT_SLACK_S = 30.0

STARTED = "started"


class RestLoadUnavailable(RuntimeError):
    pass


class BenchLanLoad:
    def __init__(self, cfg, seconds, threads):
        self.seconds = seconds
        self.threads = threads
        argv = ["ssh", *SSH_OPTS, cfg.wb_ssh, "python3", "-",
                cfg.base.rstrip("/"), str(seconds), str(threads)]
        try:
            self.proc = subprocess.Popen(
                argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True)
        except OSError as exc:
            raise RestLoadUnavailable("ssh did not start: %s" % exc)
        self.proc.stdin.write(HAMMER)
        self.proc.stdin.close()
        self.proc.stdin = None
        self._await_start()

    def _await_start(self):
        line = self.proc.stdout.readline()
        if not line or json.loads(line).get("op") != STARTED:
            self.stop()
            raise RestLoadUnavailable(
                "the generator never reported it started: %r, stderr %r"
                % (line, self.proc.stderr.read()[-200:]))

    def report(self):
        try:
            out, err = self.proc.communicate(timeout=self.seconds + REPORT_SLACK_S)
        except subprocess.TimeoutExpired:
            self.stop()
            raise RestLoadUnavailable("the generator never reported its count")
        for line in reversed(out.strip().splitlines()):
            report = json.loads(line)
            if report.get("op") != STARTED:
                return report
        raise RestLoadUnavailable("no count in %r, stderr %r" % (out[-200:], err[-200:]))

    def stop(self):
        if self.proc.poll() is None:
            self.proc.kill()
        self.proc.wait()
