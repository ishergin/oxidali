import pytest

from hil.config import _parse_shorts


def test_shorts_spec_accepts_ranges_lists_and_mixes():
    assert _parse_shorts("0-3", "HIL_LAMP_SHORTS") == frozenset({0, 1, 2, 3})
    assert _parse_shorts("0,1,2,5", "HIL_LAMP_SHORTS") == frozenset({0, 1, 2, 5})
    assert _parse_shorts("0-1, 5, 8-9", "HIL_LAMP_SHORTS") == frozenset({0, 1, 5, 8, 9})


def test_shorts_spec_refuses_emptiness_and_garbage_naming_the_variable():
    with pytest.raises(ValueError, match="HIL_GEAR_SHORTS"):
        _parse_shorts("", "HIL_GEAR_SHORTS")
    with pytest.raises(ValueError):
        _parse_shorts("abc", "HIL_GEAR_SHORTS")


def test_optical_shorts_default_is_the_lamp_allowlist(monkeypatch):
    monkeypatch.delenv("HIL_OPTICAL_SHORTS", raising=False)
    monkeypatch.setenv("HIL_LAMP_SHORTS", "0,2,3")
    from hil.config import HilConfig
    assert HilConfig().optical_short_set() == frozenset({0, 2, 3})


def test_optical_shorts_env_narrows_within_the_allowlist(monkeypatch):
    monkeypatch.setenv("HIL_LAMP_SHORTS", "0,2,3")
    from hil.config import HilConfig
    monkeypatch.setenv("HIL_OPTICAL_SHORTS", "0,2")
    assert HilConfig().optical_short_set() == frozenset({0, 2})
    monkeypatch.setenv("HIL_OPTICAL_SHORTS", "0-3")
    assert HilConfig().optical_short_set() == frozenset({0, 2, 3})
