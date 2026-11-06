import pytest

from hil_test_guards import PARK_WHERE, ROSTER_WHERE, drivable_of

PARK = [16, 17, 18]
PARK_VL = {"16": 52, "17": 53, "18": 54}
ROSTER = {2: 5, 1: 4, 3: 9}
WHAT = "a fan-out test"


class _Session:
    def __init__(self, park):
        self.park = park

    def vl(self, short):
        return PARK_VL[str(short)]


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
