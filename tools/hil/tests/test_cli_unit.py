from hil import cli, corpus, flash, serialmon
from hil.camera import backend


def test_help_after_a_command_does_not_run_it(capsys):
    ran = []
    original = cli.COMMANDS["flash"]
    cli.COMMANDS["flash"] = lambda rest: ran.append(rest)
    try:
        for flag in ("--help", "-h"):
            assert cli.main(["flash", flag]) == 0
        assert ran == [], "the handler ran despite a help flag"
    finally:
        cli.COMMANDS["flash"] = original
    assert "flash" in capsys.readouterr().out


def test_help_anywhere_in_the_arguments_still_wins(capsys):
    ran = []
    original = cli.COMMANDS["calibrate"]
    cli.COMMANDS["calibrate"] = lambda rest: ran.append(rest)
    try:
        assert cli.main(["calibrate", "--profile", "night", "--help"]) == 0
        assert ran == []
    finally:
        cli.COMMANDS["calibrate"] = original
    capsys.readouterr()


def test_a_real_invocation_still_reaches_the_handler():
    seen = []
    original = cli.COMMANDS["flash"]
    cli.COMMANDS["flash"] = lambda rest: seen.append(rest) or 0
    try:
        assert cli.main(["flash", "--build-only"]) == 0
    finally:
        cli.COMMANDS["flash"] = original
    assert seen == [["--build-only"]]


def test_unknown_command_is_rejected_without_running_anything(capsys):
    assert cli.main(["definitely-not-a-command"]) == 64
    assert "unknown command" in capsys.readouterr().err


def _never(name, ran):
    return lambda *args, **kwargs: ran.append(name)


def test_a_misspelt_flash_flag_is_refused_before_anything_is_flashed(monkeypatch, capsys):
    ran = []
    monkeypatch.setattr(flash, "run", _never("flash", ran))
    assert cli.main(["flash", "--build-onyl"]) == cli.EX_USAGE
    assert ran == []
    assert "--build-onyl" in capsys.readouterr().err


def test_the_real_flash_flags_still_reach_flash(monkeypatch):
    seen = []
    monkeypatch.setattr(flash, "run", lambda cfg, **kwargs: seen.append(kwargs) or 0)
    assert cli.main(["flash", "--build-only", "--allow-red-isr"]) == 0
    assert seen == [{"build_only": True, "allow_nonbench": False, "allow_red_isr": True,
                     "allow_stale_ui": False}]


def test_a_misspelt_lamps_flag_switches_nothing_off(monkeypatch, capsys):
    ran = []
    monkeypatch.setattr(backend, "probe_and_select", _never("camera", ran))
    assert cli.main(["lamps", "--no-basline"]) == cli.EX_USAGE
    assert ran == []
    assert "--no-basline" in capsys.readouterr().err


def test_corpus_does_not_take_peer_for_peer_only(monkeypatch, capsys):
    ran = []
    monkeypatch.setattr(corpus, "run", _never("corpus", ran))
    assert cli.main(["corpus", "--peer"]) == cli.EX_USAGE
    assert ran == []
    capsys.readouterr()


def test_peer_after_the_command_is_refused(monkeypatch, capsys):
    ran = []
    monkeypatch.setattr(flash, "run", _never("flash", ran))
    monkeypatch.setattr(serialmon, "status", _never("monitor", ran))
    assert cli.main(["flash", "--peer"]) == cli.EX_USAGE
    assert cli.main(["monitor", "status", "--peer"]) == cli.EX_USAGE
    assert cli.main(["api", "health", "--peer"]) == cli.EX_USAGE
    assert ran == []
    capsys.readouterr()


def test_an_unknown_subcommand_is_a_usage_error(capsys):
    assert cli.main(["monitor", "restart"]) == cli.EX_USAGE
    assert cli.main(["remote", "reboot"]) == cli.EX_USAGE
    assert cli.main(["state", "wipe"]) == cli.EX_USAGE
    assert cli.main(["preflight", "--all"]) == cli.EX_USAGE
    assert "invalid choice" in capsys.readouterr().err
