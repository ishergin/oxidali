import dataclasses
import json
import types

import hil.api
import hil.config
import test_corpus_unit
from hil import cli, corpus, flash, prod_state, serialmon, virtual_gear, write_log
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
                     "allow_stale_ui": False, "via": flash.VIA_RFC2217}]


def test_the_delivery_flag_reaches_flash(monkeypatch):
    seen = []
    monkeypatch.setattr(flash, "run", lambda cfg, **kwargs: seen.append(kwargs) or 0)
    assert cli.main(["flash", "--via", "wb"]) == 0
    assert seen[0]["via"] == flash.VIA_WB and "image" not in seen[0]


def test_flash_has_no_way_to_name_the_emulator_image(monkeypatch, capsys):
    ran = []
    monkeypatch.setattr(flash, "run", _never("flash", ran))
    assert cli.main(["flash", "--image", "gear-sim"]) == cli.EX_USAGE
    assert ran == []


def test_role_is_refused_without_the_peer_flag(monkeypatch, capsys):
    monkeypatch.delenv("HIL_PEER", raising=False)
    assert cli.main(["role", "status"]) == cli.EX_USAGE
    assert "--peer role" in capsys.readouterr().err


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


def test_secrets_are_kept_only_in_an_out_outside_every_repository(monkeypatch, tmp_path, capsys):
    monkeypatch.setattr(corpus, "Client", test_corpus_unit.SliceController)
    monkeypatch.setenv("GIT_CEILING_DIRECTORIES", str(tmp_path.resolve().parent))
    inside = test_corpus_unit.git_repository(tmp_path / "repo") / "corpus"
    outside = tmp_path / "kept"
    argv = ["corpus", "slices", "--keep-secrets", "--primary-only", "--out"]
    assert cli.main(argv + [str(inside)]) == cli.EX_USAGE
    assert "is inside the git work tree" in capsys.readouterr().err
    assert not list(inside.rglob("*.bin"))
    assert cli.main(argv + [str(outside)]) == 0
    assert [p.read_bytes() for p in outside.rglob("home_assistant_settings.bin")] == [
        test_corpus_unit.SECRET]
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


def _restore_with(monkeypatch, tmp_path, argv):
    real_load = hil.config.load
    monkeypatch.setattr(hil.config, "load",
                        lambda: dataclasses.replace(real_load(), state_dir=tmp_path))
    seen = []
    snapshot = tmp_path / "production_state_last.json"
    snapshot.write_text(json.dumps({"taken_at": "t1"}))
    monkeypatch.setattr(hil.api, "Client", lambda cfg: type("C", (), {"base": "http://dut"})())
    monkeypatch.setattr(virtual_gear, "teardown", lambda cfg, client: [])
    monkeypatch.setattr(prod_state, "restore", lambda client, snap, writes, **kw:
                        seen.append(writes) or prod_state.Restoration([], [], True))
    assert cli.main(["state", "restore", str(snapshot)] + argv) == 0
    return seen[0], snapshot


def test_a_killed_session_is_restored_only_through_its_own_write_log(monkeypatch, tmp_path):
    writes, _ = _restore_with(monkeypatch, tmp_path, [])
    assert writes is None
    writes, _ = _restore_with(monkeypatch, tmp_path, ["--all"])
    assert writes.changed("device/9", "name")
    log = write_log.WriteLog("t1", "http://dut",
                             write_log.writes_path(tmp_path / "production_state_last.json"))
    log.note([("settings/poller", {"enabled"})])
    writes, _ = _restore_with(monkeypatch, tmp_path, [])
    assert writes.changed("settings/poller", "enabled") and not writes.changed("device/9")


def test_a_bare_restore_takes_every_open_session_newest_first_with_its_own_log(
        monkeypatch, tmp_path, capsys):
    real_load = hil.config.load
    monkeypatch.setattr(hil.config, "load",
                        lambda: dataclasses.replace(real_load(), state_dir=tmp_path))
    cfg = types.SimpleNamespace(state_dir=tmp_path)
    for stamp in ("2026-09-29T10:00:00", "2026-09-30T10:00:00"):
        path = prod_state.session_path(cfg, stamp)
        prod_state.save({"taken_at": stamp, "session_open": True}, path)
        write_log.WriteLog(stamp, "http://dut", write_log.writes_path(path)).note(
            [("device/%s" % stamp[8:10], {"name"})])
    seen = []
    monkeypatch.setattr(hil.api, "Client", lambda cfg: type("C", (), {"base": "http://dut"})())
    monkeypatch.setattr(virtual_gear, "teardown", lambda cfg, client: [])
    monkeypatch.setattr(prod_state, "restore", lambda client, snap, writes, **kw:
                        seen.append((snap["taken_at"], writes)) or
                        prod_state.Restoration([], [], True))
    assert cli.main(["state", "restore"]) == 0
    assert [stamp for stamp, _ in seen] == ["2026-09-30T10:00:00", "2026-09-29T10:00:00"]
    assert seen[0][1].changed("device/30") and not seen[0][1].changed("device/29")
    assert prod_state.open_sessions(cfg) == []
    monkeypatch.setenv("HIL_LAMPS_READ_ONLY", "1")
    prod_state.save({"taken_at": "2026-10-01T10:00:00", "session_open": True},
                    prod_state.session_path(cfg, "2026-10-01T10:00:00"))
    seen.clear()
    assert cli.main(["state", "restore", "--all"]) == 0
    assert seen[0][1].refuses("hcl/owner") and seen[0][1].changed("device/9", "name")
    assert "HIL_LAMPS_READ_ONLY=0" in capsys.readouterr().out


def test_an_explicit_file_is_refused_while_a_newer_session_is_open(monkeypatch, tmp_path,
                                                                    capsys):
    real_load = hil.config.load
    monkeypatch.setattr(hil.config, "load",
                        lambda: dataclasses.replace(real_load(), state_dir=tmp_path))
    cfg = types.SimpleNamespace(state_dir=tmp_path)
    older, newer = (prod_state.session_path(cfg, stamp)
                    for stamp in ("2026-09-29T10:00:00", "2026-09-30T10:00:00"))
    for path in (older, newer):
        prod_state.save({"taken_at": path.stem[len(prod_state.SESSION_PREFIX):],
                         "session_open": True}, path)
    seen = []
    monkeypatch.setattr(hil.api, "Client", lambda cfg: type("C", (), {"base": "http://dut"})())
    monkeypatch.setattr(virtual_gear, "teardown", lambda cfg, client: [])
    monkeypatch.setattr(prod_state, "restore", lambda client, snap, writes, **kw:
                        seen.append(snap["taken_at"]) or prod_state.Restoration([], [], True))
    assert cli.main(["state", "restore", str(older)]) == 1
    assert seen == [] and str(newer) in capsys.readouterr().out
    cli.main(["state", "restore", str(newer)])
    assert seen == [newer.stem[len(prod_state.SESSION_PREFIX):]]



def test_restore_exits_non_zero_while_a_session_stays_open_or_cannot_be_read(
        monkeypatch, tmp_path, capsys):
    real_load = hil.config.load
    monkeypatch.setattr(hil.config, "load",
                        lambda: dataclasses.replace(real_load(), state_dir=tmp_path))
    cfg = types.SimpleNamespace(state_dir=tmp_path)
    kept = prod_state.session_path(cfg, "2026-09-29T10:00:00")
    prod_state.save({"taken_at": "2026-09-29T10:00:00", "session_open": True}, kept)
    write_log.writes_path(kept).write_text("{torn")
    monkeypatch.setattr(hil.api, "Client", lambda cfg: type("C", (), {"base": "http://dut"})())
    monkeypatch.setattr(virtual_gear, "teardown", lambda cfg, client: [])
    monkeypatch.setattr(prod_state, "restore", lambda client, snap, writes, **kw:
                        prod_state.Restoration([], [], True))
    assert cli.main(["state", "restore"]) == 1
    assert str(kept) in capsys.readouterr().out
    write_log.WriteLog("2026-09-29T10:00:00", "http://dut", write_log.writes_path(kept)).save()
    prod_state.session_path(cfg, "2026-09-30T10:00:00").write_text("{torn")
    assert cli.main(["state", "restore"]) == 1
    assert "UNREADABLE" in capsys.readouterr().out and prod_state.open_sessions(cfg) == []


def test_retire_takes_one_session_file_and_never_a_full_restore(monkeypatch, tmp_path):
    held = tmp_path / "production_state-20260930T100000.json"
    assert cli.main(["state", "restore", "--retire"]) == cli.EX_USAGE
    assert cli.main(["state", "restore", "--retire", "--all", str(held)]) == cli.EX_USAGE
    assert cli.main(["state", "diff", "--retire", str(held)]) == cli.EX_USAGE
    retired = []
    monkeypatch.setattr(hil.api, "Client", lambda cfg: type("C", (), {"base": "http://dut"})())
    monkeypatch.setattr(prod_state, "retire", lambda cfg, client, path, log=print:
                        retired.append(path) or False)
    assert cli.main(["state", "restore", "--retire", str(held)]) == 1
    assert retired == [held]
