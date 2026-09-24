import pytest

from hil import config as config_mod
from hil import flash, serialmon
from hil.config import HilConfig

PEER_PORT = "/dev/cu.usbmodemPEER1"


def _cfg(tmp_path, **over):
    base = dict(state_dir=tmp_path / "state", runs_dir=tmp_path / "runs",
                serial_remote="root@wb:/dev/ttyACM0", serial_port="",
                serial_port_pinned=False, base="http://primary",
                peer_base="", peer_serial_port="", peer_serial_remote="")
    base.update(over)
    return HilConfig(**base)


def test_an_unnamed_peer_is_refused_rather_than_defaulted():
    with pytest.raises(config_mod.PeerUnconfigured):
        _cfg(__import__("pathlib").Path("/nonexistent")).peer()


def test_the_peer_view_keeps_its_state_out_of_the_primarys(tmp_path):
    cfg = _cfg(tmp_path, peer_base="http://peer", peer_serial_port=PEER_PORT)
    peer = cfg.peer()
    assert peer.base == "http://peer"
    assert peer.serial_port == PEER_PORT and peer.serial_port_pinned
    assert peer.serial_remote == ""
    assert peer.serial_bridge_port == 4446 and cfg.serial_bridge_port == 4444
    assert peer.state_dir == cfg.state_dir / "peer"
    assert peer.runs_dir == cfg.runs_dir / "peer"
    assert serialmon.pin_path(peer) != serialmon.pin_path(cfg)
    assert peer.persist_serial_log != cfg.persist_serial_log


def test_the_peer_view_points_back_at_the_primary(tmp_path):
    cfg = _cfg(tmp_path, peer_base="http://peer", peer_serial_port=PEER_PORT)
    back = cfg.peer().peer()
    assert back.base == cfg.base
    assert back.serial_remote == cfg.serial_remote
    assert back.state_dir == cfg.state_dir and back.runs_dir == cfg.runs_dir


def test_a_recorded_base_outlives_the_env_var(tmp_path):
    cfg = _cfg(tmp_path, peer_serial_port=PEER_PORT)
    peer = cfg.peer()
    assert peer.base == ""
    flash.record_base(peer, "http://192.0.2.9")
    assert cfg.peer().base == "http://192.0.2.9"
    assert cfg.has_peer
    assert _cfg(tmp_path, peer_base="http://typed").peer().base == "http://typed"


def test_peer_manifests_stay_out_of_the_primarys_identity(tmp_path):
    from hil import validity
    cfg = _cfg(tmp_path, peer_base="http://peer")
    run = cfg.peer().new_run_dir()
    (run / "manifest.json").write_text('{"git": {"commit": "peer"}}')
    assert (cfg.runs_dir / "current").exists() is False
    assert validity.run_identity(cfg.runs_dir) == {}


def test_a_lease_is_learned_from_lines_after_the_mark(tmp_path, monkeypatch):
    cfg = _cfg(tmp_path, peer_base="http://peer")
    log = cfg.persist_serial_log
    log.parent.mkdir(parents=True)
    log.write_text("I (1) net: DHCP bound 10.0.0.5\n")
    mark = serialmon.log_size(cfg)
    assert serialmon.announced_address(cfg) == "10.0.0.5"
    assert serialmon.announced_address(cfg, since=mark) is None
    with open(log, "a") as fh:
        fh.write("I (2) net: address now 10.0.0.7\n")
    monkeypatch.setattr(flash.time, "sleep", lambda s: None)
    assert flash.learn_base(cfg, since=mark, timeout_s=2) == "http://10.0.0.7"


def test_a_board_with_no_base_and_no_monitor_is_refused_before_the_build(
        tmp_path, monkeypatch, capsys):
    cfg = _cfg(tmp_path, peer_serial_port=PEER_PORT).peer()
    monkeypatch.setattr(flash, "board_spec", lambda cfg: {})
    monkeypatch.setattr(flash, "serial_port_for_flash", lambda cfg: PEER_PORT)
    built = []
    monkeypatch.setattr(flash, "build", lambda *a, **k: built.append(1) or 0)
    assert flash.run(cfg) == 2
    assert built == []
    assert "monitor" in capsys.readouterr().err


def test_hil_peer_switches_the_loader(monkeypatch, tmp_path):
    from hil import cli
    monkeypatch.setenv("HIL_PEER_BASE", "http://peer")
    monkeypatch.setenv("HIL_SERIAL_REMOTE", "")
    monkeypatch.setenv("HIL_SERIAL_PORT", "/dev/none")
    monkeypatch.delenv("HIL_PEER", raising=False)
    seen = []
    original = cli.COMMANDS["monitor"]
    cli.COMMANDS["monitor"] = lambda rest: seen.append(config_mod.load().base) or 0
    try:
        assert cli.main(["--peer", "monitor", "status"]) == 0
    finally:
        cli.COMMANDS["monitor"] = original
        import os
        os.environ.pop("HIL_PEER", None)
    assert seen == ["http://peer"]
