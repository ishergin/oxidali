from hil.identity import refresh_short_addresses

GOOD = (677222947651, 2738477721888259483)
CORRUPT = (677029561089, 74022002431466241)

CALIBRATED = [{"label": 1, "short_address": 0,
               "identity": {"gtin": GOOD[0], "identification_number": GOOD[1]}}]


class _FakeApi:
    def __init__(self, identities, heal_to=None):
        self._identities = dict(identities)
        self._heal_to = heal_to
        self.reads = []

    def devices(self):
        return {"physical_devices": [
            {"short_address": short, "gtin": gtin, "identification_number": idn}
            for short, (gtin, idn) in sorted(self._identities.items())]}

    def attr_read(self, short, groups=None, banks=None):
        self.reads.append(short)
        if self._heal_to is not None:
            self._identities[short] = self._heal_to
        return {"operation_id": "op-%d" % short}

    def wait_op(self, op):
        return {"status": "succeeded"}


def test_corrupt_cache_heals_on_reread():
    api = _FakeApi({0: CORRUPT}, heal_to=GOOD)

    mapping, missing = refresh_short_addresses(api, CALIBRATED)

    assert api.reads == [0], "the unmatched lamp must be re-read exactly once"
    assert mapping == {1: 0}
    assert missing == []


def test_healthy_cache_costs_no_reads():
    api = _FakeApi({0: GOOD})

    mapping, missing = refresh_short_addresses(api, CALIBRATED)

    assert api.reads == [], "a clean first match must not touch the bus"
    assert mapping == {1: 0} and missing == []


def test_genuinely_absent_lamp_still_reported_missing():
    other = (111111111111, 222222222222)
    api = _FakeApi({0: other})

    mapping, missing = refresh_short_addresses(api, CALIBRATED)

    assert api.reads == [0], "the suspect address is re-read before giving up"
    assert mapping == {} and missing == [1]


def test_empty_bus_gives_up_without_reads():
    api = _FakeApi({})

    mapping, missing = refresh_short_addresses(api, CALIBRATED)

    assert api.reads == [] and mapping == {} and missing == [1]
