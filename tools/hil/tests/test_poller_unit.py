import pytest

import test_poller as poller


def _delta(reads, sweep_reads, sweeps):
    return {"reads_published": reads, "sweep_reads": sweep_reads,
            "sweeps": sweeps}


def test_a_watch_run_that_met_no_yes_is_inconclusive_not_green():
    verdict = poller.sweep_read_verdict(_delta(0, 0, 0))
    assert verdict and verdict.startswith("INCONCLUSIVE"), verdict


@pytest.mark.parametrize("reads, sweep_reads, sweeps", [(1, 0, 0), (3, 2, 1)])
def test_a_read_beside_the_sweep_fails_whether_or_not_a_yes_came(reads, sweep_reads, sweeps):
    verdict = poller.sweep_read_verdict(_delta(reads, sweep_reads, sweeps))
    assert verdict and not verdict.startswith("INCONCLUSIVE"), verdict


def test_a_sweep_whose_reads_are_all_the_reads_passes():
    assert poller.sweep_read_verdict(_delta(2, 2, 1)) is None


def test_a_yes_whose_sweep_read_nothing_is_inconclusive_not_green():
    verdict = poller.sweep_read_verdict(_delta(0, 0, 1))
    assert verdict and verdict.startswith("INCONCLUSIVE"), verdict


def test_sweep_reads_judged_without_a_fresh_sweep_in_the_window_pass():
    assert poller.sweep_read_verdict(_delta(2, 2, 0)) is None
