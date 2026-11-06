import pytest

from hil.virtual_gear import PARK_KINDS
from hil_test_guards import PARK_WHERE, ROSTER_WHERE, drivable_of

PARK = [16, 17, 18]
PARK_VL = {"16": 52, "17": 53, "18": 54}
ROSTER = {2: 5, 1: 4, 3: 9}
WHAT = "a fan-out test"


class _Session:
    def __init__(self, park):
        self.park = park
        self.vl_of_short = {str(short): PARK_VL[str(short)] for short in park}

    def vl(self, short):
        return PARK_VL[str(short)]

    def of_kind(self, kind):
        return {"dt6": [16], "cct": [17], "rgb": []}[kind]


class _Roster:
    def __init__(self, by_label):
        self.by_label = by_label

    def labels(self):
        return sorted(self.by_label)


def test_a_virtual_session_drives_its_park_and_its_own_virtual_lamps():
    subject = drivable_of(_Session(PARK), None)
    assert subject.shorts == PARK
    assert subject.first() == PARK[0]
    assert [subject.vl(short) for short in PARK] == [52, 53, 54]
    assert subject.where == PARK_WHERE


def test_without_a_session_the_calibrated_roster_drives_in_label_order():
    subject = drivable_of(None, _Roster(ROSTER))
    assert subject.shorts == [4, 5, 9]
    assert subject.vl(4) == 0 and subject.vl(5) == 1 and subject.vl(9) == 2
    assert subject.where == ROSTER_WHERE


def test_two_gear_are_two_distinct_subjects():
    assert drivable_of(_Session(PARK), None).two(WHAT) == (16, 17)
    assert drivable_of(None, _Roster(ROSTER)).two(WHAT) == (4, 5)


def test_one_gear_skips_a_test_that_needs_two_and_says_where_it_looked():
    for subject, where in ((drivable_of(_Session(PARK[:1]), None), PARK_WHERE),
                           (drivable_of(None, _Roster({1: 4})), ROSTER_WHERE)):
        with pytest.raises(pytest.skip.Exception) as refused:
            subject.two(WHAT)
        assert WHAT in str(refused.value) and where in str(refused.value)


def test_a_kind_comes_from_the_park_and_only_the_parks_own_kinds_exist():
    subject = drivable_of(_Session(PARK), None)
    assert subject.kind_first("dt6") == 16 and subject.kind_first("cct") == 17
    assert subject.kind_first("rgb") is None
    with pytest.raises(KeyError):
        subject.kind_first("dt8")
    assert PARK_KINDS == ("dt6", "cct", "rgb")
    assert drivable_of(None, _Roster(ROSTER)).kind_first("cct") is None


def test_a_matrix_guard_writes_back_only_the_sessions_rows_on_the_tier():
    from hil_test_guards import ours_to_restore
    rows = [{"virtual_lamp_id": 0}, {"virtual_lamp_id": 52}, {"virtual_lamp_id": 53}]
    assert ours_to_restore(rows, None) == rows
    assert ours_to_restore(rows, _Session(PARK)) == [{"virtual_lamp_id": 52},
                                                     {"virtual_lamp_id": 53}]
