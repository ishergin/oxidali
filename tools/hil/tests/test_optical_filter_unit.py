import pytest

from hil.config import _parse_shorts


def test_shorts_spec_accepts_ranges_lists_and_mixes():
    assert _parse_shorts("0-3") == frozenset({0, 1, 2, 3})
    assert _parse_shorts("0,1,2,5") == frozenset({0, 1, 2, 5})
    assert _parse_shorts("0-1, 5, 8-9") == frozenset({0, 1, 5, 8, 9})


def test_shorts_spec_refuses_emptiness_and_garbage():
    with pytest.raises(ValueError):
        _parse_shorts("")
    with pytest.raises(ValueError):
        _parse_shorts("abc")


def test_optical_shorts_default_is_the_bench_reserved_set(monkeypatch):
    monkeypatch.delenv("HIL_OPTICAL_SHORTS", raising=False)
    from hil.config import HilConfig
    assert HilConfig().optical_short_set() == frozenset({0, 1, 2, 3})


def test_optical_shorts_env_overrides(monkeypatch):
    monkeypatch.setenv("HIL_OPTICAL_SHORTS", "0,2")
    from hil.config import HilConfig
    assert HilConfig().optical_short_set() == frozenset({0, 2})
