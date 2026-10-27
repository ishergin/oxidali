class FakeClock:
    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


def install(monkeypatch, *modules):
    fake = FakeClock()
    for module in modules:
        monkeypatch.setattr(module, "time", fake)
    return fake
