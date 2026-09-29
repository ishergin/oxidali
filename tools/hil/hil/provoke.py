NAME = "name"


def first_registered(api, allowed):
    shorts = sorted(d["short_address"] for d in api.devices_unfiltered()["physical_devices"]
                    if d["short_address"] in allowed)
    return shorts[0] if shorts else None


class NameRewrite:
    def __init__(self, api, short):
        self.api, self.short = api, short
        self.original = api.state(short).get(NAME)
        self.writes = 0

    def __call__(self):
        self.api.device_patch(self.short, {NAME: self.original})
        self.writes += 1

    def restore(self):
        if self.api.state(self.short).get(NAME) != self.original:
            self.api.device_patch(self.short, {NAME: self.original})
        return self.api.state(self.short).get(NAME) == self.original
