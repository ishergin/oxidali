from hil import cli


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
