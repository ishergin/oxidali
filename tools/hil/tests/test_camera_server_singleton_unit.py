import contextlib
import json
import os
import threading
import time
import types

import numpy as np
import pytest

from hil.camera import server as server_mod
from hil.camera.backend import CameraError
from hil.camera.backends import FrameServerBackend


def _cfg(tmp_path):
    return types.SimpleNamespace(state_dir=str(tmp_path))



def test_second_server_is_refused_and_first_is_named(tmp_path):
    cfg = _cfg(tmp_path)
    assert server_mod.incumbent(cfg) is None, "a fresh state dir owns no server"

    first = server_mod.acquire_singleton(cfg)
    assert first is not None
    try:
        assert server_mod.acquire_singleton(cfg) is None, \
            "the second server must be refused, not allowed to share the mailbox"
        live = server_mod.incumbent(cfg)
        assert live["pid"] == os.getpid()
        assert live["uptime_s"] >= 0
        assert "pid %d" % live["pid"] in server_mod.describe(cfg)
    finally:
        first.close()


def test_lock_is_released_when_the_holder_goes_away(tmp_path):
    cfg = _cfg(tmp_path)
    handle = server_mod.acquire_singleton(cfg)
    assert handle is not None
    handle.close()

    assert server_mod.incumbent(cfg) is None
    second = server_mod.acquire_singleton(cfg)
    assert second is not None, "a released lock must be re-acquirable"
    second.close()


def test_spawn_does_not_launch_a_terminal_while_a_server_is_live(tmp_path,
                                                                monkeypatch,
                                                                capsys):
    cfg = _cfg(tmp_path)
    monkeypatch.setattr(server_mod, "load_config", lambda: cfg)

    def _must_not_run(*args, **kwargs):
        raise AssertionError("spawn opened a Terminal while a server was live")

    monkeypatch.setattr(server_mod, "run_in_terminal", _must_not_run)

    handle = server_mod.acquire_singleton(cfg)
    try:
        assert server_mod.spawn_in_terminal() == 0
        assert "--restart" in capsys.readouterr().out
    finally:
        handle.close()


def test_serve_refuses_before_touching_the_camera(tmp_path, monkeypatch):
    cfg = _cfg(tmp_path)
    monkeypatch.setattr(server_mod, "load_config", lambda: cfg)

    def _must_not_construct(*args, **kwargs):
        raise AssertionError("serve() opened a camera session despite the lock")

    monkeypatch.setattr(server_mod, "DirectBackend", _must_not_construct)

    handle = server_mod.acquire_singleton(cfg)
    try:
        assert server_mod.serve() == 1
        assert not (tmp_path / "frame_server" / "server.alive").exists()
    finally:
        handle.close()



def test_only_a_serving_invocation_counts_as_a_server():
    serving = ("/Applications/Xcode.app/.../Python "
               "/Users/x/work/dali2rust/tools/hil/.venv/bin/hil camera-server")
    assert server_mod.is_serving_argv(serving)
    assert server_mod.is_serving_argv("hil camera-server")

    for verb in ("--status", "--restart", "--stop", "--spawn-terminal"):
        assert not server_mod.is_serving_argv(
            "/py /venv/bin/hil camera-server %s" % verb), verb

    assert not server_mod.is_serving_argv(
        "/bin/zsh -c /venv/bin/hil camera-server --status | grep -v warn")
    assert not server_mod.is_serving_argv("/py /venv/bin/hil camera-bench")
    assert not server_mod.is_serving_argv("")



def _mailbox_backend(tmp_path, monkeypatch):
    mailbox = tmp_path / "frame_server"
    mailbox.mkdir(parents=True)
    (mailbox / "server.alive").write_text(json.dumps(
        {"ts": 0, "healthy": True, "consecutive_failures": 0, "last_error": None}))
    backend = FrameServerBackend(_cfg(tmp_path))
    monkeypatch.setattr(backend, "alive", lambda: True)
    monkeypatch.setattr(backend, "TIMEOUT_S", 5.0)
    return mailbox, backend


@contextlib.contextmanager
def _answering_server(mailbox, write_answer):
    stop = threading.Event()

    def loop():
        while not stop.is_set():
            for req in mailbox.glob("req_*.json"):
                write_answer(req.name[4:-5])
                req.unlink(missing_ok=True)
                return
            time.sleep(0.02)

    thread = threading.Thread(target=loop, daemon=True)
    thread.start()
    try:
        yield
    finally:
        stop.set()
        thread.join(timeout=2.0)


def test_torn_frame_raises_camera_error_not_zlib(tmp_path, monkeypatch):
    mailbox, backend = _mailbox_backend(tmp_path, monkeypatch)

    def torn(rid):
        (mailbox / ("frame_%s.npz" % rid)).write_bytes(b"PK\x03\x04" + b"\x00" * 60)

    with _answering_server(mailbox, torn):
        with pytest.raises(CameraError) as excinfo:
            backend.capture(warmup=1, avg=1)

    message = str(excinfo.value)
    assert "torn frame" in message
    assert "camera-server" in message, "the message must name the likely cause"
    assert not list(mailbox.glob("frame_*.npz")), \
        "a torn frame must be removed, not left to poison the next reader"


def test_intact_frame_still_decodes(tmp_path, monkeypatch):
    mailbox, backend = _mailbox_backend(tmp_path, monkeypatch)
    expected = np.array([[1.0, 2.0], [3.0, 4.0]], dtype=np.float32)

    def good(rid):
        tmp = mailbox / (".frame_%s.tmp.npz" % rid)
        np.savez_compressed(tmp, frame=expected)
        tmp.rename(mailbox / ("frame_%s.npz" % rid))

    with _answering_server(mailbox, good):
        frame = backend.capture(warmup=1, avg=1)

    assert np.array_equal(frame, expected)
    assert not list(mailbox.glob("frame_*.npz")), "a consumed frame is removed"
