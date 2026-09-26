import importlib.util
import os
import socket

import pytest
import serial

from hil import preflight, remote_serial
from hil.config import HilConfig

SSH = "root@192.168.13.110"
LIVE = 76601
DEAD = 91966
FOREIGN = 4242


def _cfg(tmp_path):
    return HilConfig(state_dir=tmp_path,
                     serial_remote="%s:/dev/serial/by-id/usb-1a86_*-if00" % SSH)


def _tgt():
    return remote_serial.Target("%s:/dev/ttyACM0" % SSH)


@pytest.fixture
def port(monkeypatch):
    state = {"pids": [], "ours": set()}

    monkeypatch.setattr(remote_serial, "_listening_pids", lambda _p: state["pids"])
    monkeypatch.setattr(remote_serial, "_is_our_tunnel",
                        lambda pid, tgt: pid in state["ours"])
    return state


@pytest.fixture
def spawned(monkeypatch):
    calls = []

    class _Proc:
        pid = 55555

    def _popen(argv, **kwargs):
        calls.append(argv)
        return _Proc()

    monkeypatch.setattr(remote_serial.subprocess, "Popen", _popen)
    return calls


def test_a_live_forward_is_reused_and_adopted_over_a_dead_pidfile(tmp_path, port, spawned):
    cfg = _cfg(tmp_path)
    remote_serial.tunnel_pidfile(cfg).write_text(str(DEAD))
    port["pids"], port["ours"] = [LIVE], {LIVE}

    assert remote_serial.start_tunnel(cfg, _tgt()) == "reusing"
    assert spawned == [], "a second ssh cannot bind and must not be started"
    assert remote_serial.tunnel_pidfile(cfg).read_text().strip() == str(LIVE), \
        "the record must be adopted, or stop_tunnel still cannot reach the forward"


def test_the_status_line_names_the_live_forward_a_dead_pidfile_hid(tmp_path, port):
    cfg = _cfg(tmp_path)
    remote_serial.tunnel_pidfile(cfg).write_text(str(DEAD))
    port["pids"], port["ours"] = [LIVE], {LIVE}

    assert remote_serial._tunnel_description(cfg, _tgt()) == "running (pid %d)" % LIVE


def test_a_free_port_starts_one_tunnel_and_records_it(tmp_path, port, spawned):
    cfg = _cfg(tmp_path)
    port["pids"] = []

    assert remote_serial.start_tunnel(cfg, _tgt()) == "started"
    assert len(spawned) == 1
    assert "-L" in spawned[0] and SSH in spawned[0]
    assert remote_serial.tunnel_pidfile(cfg).read_text().strip() == "55555"


def test_stop_kills_the_forward_the_port_names_not_the_one_the_file_does(tmp_path, port, monkeypatch):
    cfg = _cfg(tmp_path)
    remote_serial.tunnel_pidfile(cfg).write_text(str(DEAD))
    port["pids"], port["ours"] = [LIVE], {LIVE}
    killed = []
    monkeypatch.setattr("os.getpgid", lambda pid: pid)
    monkeypatch.setattr("os.killpg", lambda pgid, sig: killed.append(pgid))

    assert remote_serial.stop_tunnel(cfg, _tgt()) is True
    assert killed == [LIVE], "killing the pidfile's pid is what made --restart a no-op"
    assert not remote_serial.tunnel_pidfile(cfg).exists()


def test_a_foreign_holder_is_reported_and_never_signalled(tmp_path, port, spawned, monkeypatch, capsys):
    cfg = _cfg(tmp_path)
    port["pids"], port["ours"] = [FOREIGN], set()
    killed = []
    monkeypatch.setattr("os.killpg", lambda pgid, sig: killed.append(pgid))

    assert remote_serial.stop_tunnel(cfg, _tgt()) is False
    assert killed == []
    assert "not our tunnel" in capsys.readouterr().err

    with pytest.raises(remote_serial.RemoteError, match="is held by pid"):
        remote_serial.start_tunnel(cfg, _tgt())
    assert spawned == [], "an ssh that cannot bind is not a recovery"


def test_a_port_that_cannot_be_asked_is_not_a_port_that_is_free(tmp_path, monkeypatch, spawned):
    cfg = _cfg(tmp_path)
    monkeypatch.setattr(remote_serial, "_listening_pids", lambda _p: None)
    monkeypatch.setattr(remote_serial, "pid_alive", lambda _f: True)
    remote_serial.tunnel_pidfile(cfg).write_text(str(LIVE))

    state = remote_serial.tunnel_state(cfg, _tgt())
    assert state.kind == remote_serial.TUNNEL_UNASKED and state.pid == LIVE
    assert remote_serial.tunnel_alive(cfg, _tgt()) is True
    assert remote_serial.start_tunnel(cfg, _tgt()) == "reusing"
    assert spawned == []
    assert "no lsof" in remote_serial._tunnel_description(cfg, _tgt())


def test_lsof_answering_nothing_is_an_answer(monkeypatch):
    class _Out:
        def __init__(self, rc, text):
            self.returncode, self.stdout = rc, text

    seen = {}
    monkeypatch.setattr(remote_serial.subprocess, "run",
                        lambda argv, **kw: seen["out"])

    seen["out"] = _Out(1, "")
    assert remote_serial._listening_pids(4444) == []
    seen["out"] = _Out(0, "76601\n76601\n")
    assert remote_serial._listening_pids(4444) == [LIVE], "one process, two sockets"
    seen["out"] = _Out(2, "lsof: something went wrong")
    assert remote_serial._listening_pids(4444) is None


def test_a_changed_script_never_restarts_a_live_bridge_without_being_asked(monkeypatch, tmp_path, capsys):
    commands = []
    monkeypatch.setattr(remote_serial, "resolve_device", lambda tgt: tgt.device)
    monkeypatch.setattr(remote_serial, "deploy", lambda cfg, tgt: True)
    monkeypatch.setattr(remote_serial, "bridge_listening", lambda tgt, port: True)
    monkeypatch.setattr(remote_serial, "_ssh", lambda tgt, cmd: commands.append(cmd))
    assert remote_serial.start_bridge(_cfg(tmp_path), _tgt()) == "reusing"
    assert commands == []
    assert "--restart" in capsys.readouterr().out


def test_an_explicit_restart_replaces_a_live_bridge(monkeypatch, tmp_path):
    commands = []

    class _Out:
        stdout = ""

    monkeypatch.setattr(remote_serial, "resolve_device", lambda tgt: tgt.device)
    monkeypatch.setattr(remote_serial, "deploy", lambda cfg, tgt: False)
    monkeypatch.setattr(remote_serial, "bridge_listening", lambda tgt, port: True)
    monkeypatch.setattr(remote_serial, "_ssh", lambda tgt, cmd: commands.append(cmd) or _Out())
    assert remote_serial.start_bridge(_cfg(tmp_path), _tgt(), restart=True) == "started"
    assert any("fuser -k" in cmd for cmd in commands)


def _bridge_module():
    spec = importlib.util.spec_from_file_location("serial_bridge", remote_serial.script_path())
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


BRIDGE = _bridge_module()
BAUD = 115200
UNPLUGGED = "device reports readiness to read but returned no data"
GONE_REPLY = "err port-gone: /dev/ttyACM0 is gone (No such file or directory)"


class _HeldPort:
    in_waiting = 1

    def __init__(self, fd, fail=None):
        self.fd, self.fail, self.baudrate = fd, fail, BAUD

    def fileno(self):
        return self.fd

    def read(self, size):
        raise serial.SerialException(self.fail)


def _bridge(node, held_fd, fail=None):
    bridge = BRIDGE.Bridge(str(node), BAUD, "127.0.0.1", 0, 0)
    bridge.serial = _HeldPort(held_fd, fail)
    bridge.log = lambda message: None
    return bridge


def test_the_bridge_answers_an_error_on_every_command_once_its_port_vanished(tmp_path):
    node = tmp_path / "ttyACM0"
    node.write_text("")
    with open(node) as held:
        bridge = _bridge(node, held.fileno())
        assert bridge._control_reply("ping") == "ok %s %d" % (node, BAUD)
        node.unlink()
        replies = [bridge._control_reply(cmd) for cmd in ("ping", "status", "run", "bootloader")]
    assert all(r.startswith("err %s: " % BRIDGE.PORT_GONE) and "is gone" in r
               for r in replies), replies


def test_a_port_enumerated_again_is_not_the_device_the_bridge_holds(tmp_path, monkeypatch):
    node = tmp_path / "ttyACM0"
    node.write_text("")
    with open(node) as held:
        bridge = _bridge(node, held.fileno())
        monkeypatch.setattr(BRIDGE, "_node_device", lambda path: os.fstat(held.fileno()).st_rdev + 1)
        reply = bridge._control_reply("status")
    assert reply.startswith("err %s: " % BRIDGE.PORT_GONE) and "enumerated again" in reply


def test_a_serial_fault_under_a_client_is_remembered_by_the_control_port(tmp_path):
    node = tmp_path / "ttyACM0"
    node.write_text("")
    readable, writable = os.pipe()
    os.write(writable, b"x")
    ours, theirs = socket.socketpair()
    try:
        bridge = _bridge(node, readable, fail=UNPLUGGED)
        bridge.serve_client(theirs, "127.0.0.1:50000")
        reply = bridge._control_reply("ping")
    finally:
        ours.close()
        os.close(readable)
        os.close(writable)
    assert bridge.client is None
    assert reply.startswith("err %s: " % BRIDGE.PORT_GONE) and UNPLUGGED in reply


def test_the_client_spells_the_bridges_error_the_way_the_bridge_does():
    assert remote_serial.PORT_GONE == BRIDGE.PORT_GONE


class _ControlSocket:
    def __init__(self, reply):
        self.reply = reply.encode()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def sendall(self, data):
        pass

    def recv(self, size):
        return self.reply


def test_a_reused_bridge_without_its_port_fails_at_once_and_names_the_reset(
        tmp_path, monkeypatch):
    monkeypatch.setattr(remote_serial, "start_bridge", lambda cfg, tgt, restart=False: "reusing")
    monkeypatch.setattr(remote_serial, "start_tunnel", lambda cfg, tgt: "reusing")
    monkeypatch.setattr(remote_serial.socket, "create_connection",
                        lambda address, timeout: _ControlSocket(GONE_REPLY + "\n"))
    monkeypatch.setattr(remote_serial.time, "sleep",
                        lambda s: pytest.fail("ensure retried a bridge that answered"))
    with pytest.raises(remote_serial.BridgePortGone) as refused:
        remote_serial.ensure(_cfg(tmp_path), verbose=False)
    assert GONE_REPLY in str(refused.value)
    assert remote_serial.RESTART_RESETS in str(refused.value)


def _port_gone(cfg, command, timeout=None):
    raise remote_serial.BridgePortGone(GONE_REPLY)


def test_status_says_the_bridge_answers_without_its_port(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(remote_serial, "control", _port_gone)
    monkeypatch.setattr(remote_serial, "_tunnel_description", lambda cfg, tgt: "running")
    assert remote_serial.status(_cfg(tmp_path)) == 1
    err = capsys.readouterr().err
    assert GONE_REPLY in err and "unreachable" not in err


def test_preflight_fails_a_bridge_without_its_port_instead_of_calling_it_unreachable(
        tmp_path, monkeypatch):
    monkeypatch.setattr(remote_serial, "control", _port_gone)
    status, name, detail = preflight._check_serial_port(_cfg(tmp_path))
    assert (status, name) == (preflight.FAIL, "serial port")
    assert GONE_REPLY in detail and "unreachable" not in detail
