import pytest

from hil import identity, prod_state
from hil.identity import refresh_short_addresses, restore_short_addresses

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


YES = 0xFF
OLD, NEW, ELSEWHERE = 5, 9, 7
SET = ("cmd", OLD, identity.SET_SHORT_ADDRESS, identity.SEND_TWICE)


class _Bus:
    def __init__(self, gear, foreign_dtr0=None, spoil=None):
        self.at = dict(gear)
        self.dtr0, self.foreign_dtr0, self.spoil = 0, foreign_dtr0, spoil
        self.sent = []

    def devices(self):
        return {"physical_devices": [
            {"short_address": short, "gtin": gtin, "identification_number": idn}
            for short, (gtin, idn) in sorted(self.at.items())]}

    def raw(self, frame, expects_backward=False):
        addr, data = frame >> 8, frame & 0xFF
        if addr == prod_state.DTR0:
            self.dtr0 = data
            self.sent.append(("DTR0", data))
            return {"success": True, "backward_frame": 0}
        short = addr >> 1
        self.sent.append(("query", short, data))
        if short not in self.at:
            return {"success": False, "backward_frame": 0}
        if data == prod_state.QUERY_CONTENT_DTR0:
            seen = self.dtr0 if self.foreign_dtr0 is None else self.foreign_dtr0
            return {"success": True, "backward_frame": seen}
        return {"success": True, "backward_frame": YES}

    def cmd(self, short, opcode, repeat=1):
        self.sent.append(("cmd", short, opcode, repeat))
        if opcode == identity.SET_SHORT_ADDRESS and short in self.at:
            if self.spoil is not None:
                self.dtr0 = self.spoil
            self.at[self.dtr0 >> 1] = self.at.pop(short)
        return {"success": True}


@pytest.fixture(autouse=True)
def _no_pacing(monkeypatch):
    monkeypatch.setattr(prod_state, "FRAME_PACE_S", 0)


def test_the_address_moves_only_after_dtr0_reads_back_and_the_move_is_checked():
    bus = _Bus({OLD: GOOD})
    assert restore_short_addresses(bus, {GOOD: NEW}) == []
    assert bus.at == {NEW: GOOD}
    proof = ("query", OLD, prod_state.QUERY_CONTENT_DTR0)
    assert bus.sent.index(proof) < bus.sent.index(SET)
    assert bus.sent[bus.sent.index(SET) + 1:] == [
        ("query", NEW, identity.QUERY_CONTROL_GEAR_PRESENT),
        ("query", OLD, identity.QUERY_CONTROL_GEAR_PRESENT)]


def test_a_dtr0_that_never_reads_back_sends_no_set_short_address():
    bus = _Bus({OLD: GOOD}, foreign_dtr0=0x10)
    with pytest.raises(identity.AddressNotMoved, match="never read DTR0 back"):
        restore_short_addresses(bus, {GOOD: NEW})
    assert SET not in bus.sent and bus.at == {OLD: GOOD}


def test_a_dtr0_overwritten_after_its_proof_is_refused_by_the_check():
    bus = _Bus({OLD: GOOD}, spoil=(ELSEWHERE << 1) | 1)
    with pytest.raises(identity.AddressNotMoved, match="went astray"):
        restore_short_addresses(bus, {GOOD: NEW})
    assert bus.at == {ELSEWHERE: GOOD}


def test_a_target_another_gear_holds_is_refused_before_anything_is_sent():
    bus = _Bus({OLD: GOOD, NEW: CORRUPT})
    with pytest.raises(identity.AddressNotMoved, match="already answers"):
        restore_short_addresses(bus, {GOOD: NEW})
    assert [frame for frame in bus.sent if frame[0] != "query"] == []
