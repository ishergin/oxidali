import argparse
import os
import sys

EX_USAGE = 64

API_USAGE = ("usage: hil api health|devices|addrs|state N|ts N '<json>'|off N|"
             "off-all|dapc N L|cmd N OP|attr-read N [groups] [banks]|"
             "discovery MODE|op ID|wait-op ID|snapshot|restore '<json>'")

USAGE = """hil [--peer] <command> [args]: the HIL toolkit's single operator entry point.

`--peer`, before the command, points it at the pair's other controller: its URL,
its serial bridge, and its own state/peer/ and runs/peer/."""

COMMAND_HELP = {
    "remote": "remote start|stop|status|ping|bootloader|run [--restart]: the WB serial "
              "bridge and its ssh tunnel.",
    "api": "manual calls against the controller's API; exit 2 means the firmware "
           "lacks the capability.\n" + API_USAGE,
    "lamps": "one optical readout of every calibrated lamp; switches every lamp off "
             "for its baseline unless --no-baseline.",
    "corpus": "usage: hil corpus [all|slices|rest|gear|wire|bench ...] [--out DIR]\n"
              "                  [--primary-only|--peer-only] [--keep-secrets]\n"
              "                  [--wire-keep N] [--wire-scan-mb N]\n"
              "Captures both boards by default; `hil --peer corpus` is "
              "`hil corpus --peer-only`.",
    "state": "state save|restore|diff [FILE]: the owner's installation as a file. "
             "FILE defaults to state/production_state_last.json, which every guarded "
             "session writes, so a killed session is recovered with `hil state restore`.",
}


class UsageError(Exception):
    pass


class _Parser(argparse.ArgumentParser):
    def __init__(self, *args, **kwargs):
        kwargs.setdefault("allow_abbrev", False)
        kwargs.setdefault("add_help", False)
        super().__init__(*args, **kwargs)

    def error(self, message):
        raise UsageError("%s%s: error: %s" % (self.format_usage(), self.prog, message))


def _flags(prog, rest, *flags):
    ap = _Parser(prog=prog)
    for flag in flags:
        ap.add_argument(flag, action="store_true")
    return ap.parse_args(rest)


def _cmd_decode(rest):
    from hil.sniffer import decode_main
    return decode_main(rest, parser_class=_Parser) or 0


def _cmd_calibrate(rest):
    ap = _Parser(prog="hil calibrate")
    ap.add_argument("--profile", choices=("day", "night"), default=None)
    ap.add_argument("--fingerprints-only", action="store_true")
    ap.add_argument("--skip-fingerprints", action="store_true")
    args = ap.parse_args(rest)
    from hil.api import Client
    from hil.camera.backend import probe_and_select
    from hil.camera.calibrate import Calibrator
    from hil.config import load as load_config
    cfg = load_config()
    backend = probe_and_select(cfg)
    cal = Calibrator(cfg, Client(cfg), backend)
    try:
        if args.fingerprints_only:
            cal.refresh_fingerprints()
        else:
            cal.run(profile=args.profile,
                    skip_fingerprints=args.skip_fingerprints)
        return 0
    finally:
        backend.close()


def _cmd_camera_server(rest):
    ap = _Parser(prog="hil camera-server")
    mode = ap.add_mutually_exclusive_group()
    for flag in ("--restart", "--stop", "--status", "--spawn-terminal"):
        mode.add_argument(flag, action="store_true")
    args = ap.parse_args(rest)
    from hil.camera import server
    from hil.config import load as load_config
    if args.restart:
        return server.restart_in_terminal()
    if args.stop:
        return server.stop(load_config())
    if args.status:
        return server.status(load_config())
    if args.spawn_terminal:
        return server.spawn_in_terminal()
    return server.serve() or 0


def _cmd_camera_bench(rest):
    args = _flags("hil camera-bench", rest, "--spawn-terminal")
    from hil.camera import bench, server
    if args.spawn_terminal:
        import time as _time
        from pathlib import Path as _Path

        from hil.config import load as load_config
        marker = _Path(load_config().state_dir) / "camera_bench.json"
        t0 = _time.time()
        return server.run_in_terminal(
            "camera-bench", lambda: marker.exists()
            and marker.stat().st_mtime > t0,
            timeout_s=180, label="camera bench")
    return bench.run()


def _cmd_preflight(rest):
    _flags("hil preflight", rest)
    from hil.config import load as load_config
    from hil.preflight import run
    return run(load_config())


def _monitor_parser():
    ap = _Parser(prog="hil monitor")
    subs = ap.add_subparsers(dest="sub")
    subs.add_parser("start").add_argument("log", nargs="?")
    subs.add_parser("stop")
    subs.add_parser("status")
    subs.add_parser("tail").add_argument("lines", nargs="?", type=int, default=20)
    run = subs.add_parser("_run")
    run.add_argument("port")
    run.add_argument("baud")
    run.add_argument("discovery", nargs="?", choices=("pinned", "auto"), default="auto")
    return ap


def _cmd_monitor(rest):
    args = _monitor_parser().parse_args(rest)
    from hil import serialmon
    from hil.config import load as load_config
    if args.sub == "_run":
        serialmon.reader_loop(args.port, args.baud, pinned=args.discovery == "pinned")
        return 0
    cfg = load_config()
    return {"start": lambda: serialmon.start(cfg, args.log),
            "stop": lambda: serialmon.stop(cfg),
            "tail": lambda: serialmon.tail(cfg, args.lines),
            }.get(args.sub, lambda: serialmon.status(cfg))()


def _cmd_flash(rest):
    args = _flags("hil flash", rest,
                  "--build-only", "--allow-nonbench-build", "--allow-red-isr",
                  "--allow-stale-ui")
    from hil import flash
    from hil.config import load as load_config
    return flash.run(
        load_config(),
        build_only=args.build_only,
        allow_nonbench=args.allow_nonbench_build,
        allow_red_isr=args.allow_red_isr,
        allow_stale_ui=args.allow_stale_ui,
    )


REMOTE_CONTROL_VERBS = ("ping", "bootloader", "run")


def _remote_parser():
    ap = _Parser(prog="hil remote")
    subs = ap.add_subparsers(dest="sub")
    subs.add_parser("start").add_argument("--restart", action="store_true")
    for verb in ("stop", "status") + REMOTE_CONTROL_VERBS:
        subs.add_parser(verb)
    return ap


def _cmd_remote(rest):
    args = _remote_parser().parse_args(rest)
    from hil import remote_serial
    from hil.config import load as load_config
    cfg = load_config()
    sub = args.sub or "status"
    if not remote_serial.enabled(cfg) and sub != "status":
        print("serial is local (HIL_SERIAL_REMOTE is empty)", file=sys.stderr)
        return 1
    try:
        if sub == "status":
            return remote_serial.status(cfg)
        if sub == "start":
            remote_serial.ensure(cfg, restart=args.restart)
            return 0
        if sub == "stop":
            print("tunnel stopped"
                  if remote_serial.stop_tunnel(cfg, remote_serial.target(cfg))
                  else "tunnel not running")
            return 0
        print(remote_serial.control(cfg, sub))
        return 0
    except (OSError, remote_serial.RemoteError) as exc:
        print("remote serial: %s" % exc, file=sys.stderr)
        return 1


API_SUBCOMMANDS = ("health", "devices", "addrs", "state", "ts", "off", "off-all", "dapc",
                   "cmd", "attr-read", "discovery", "op", "wait-op", "snapshot", "restore")


def _cmd_api(rest):
    import json as _json

    if not rest:
        print(API_USAGE)
        return EX_USAGE
    ap = _Parser(prog="hil api")
    ap.add_argument("sub", choices=API_SUBCOMMANDS)
    ap.add_argument("args", nargs="*")
    parsed = ap.parse_args(rest)
    from hil.api import ApiError, Client
    from hil.config import load as load_config
    from hil.lamp_guard import LampNotAllowed
    client = Client(load_config())
    sub, args = parsed.sub, parsed.args
    try:
        table = {
            "health": lambda: client.health(),
            "devices": lambda: client.devices(),
            "addrs": lambda: client.addrs(),
            "state": lambda: client.state(int(args[0])),
            "ts": lambda: client.ts(int(args[0]), _json.loads(args[1])),
            "off": lambda: client.off(int(args[0])),
            "off-all": lambda: client.off_all(),
            "dapc": lambda: client.dapc(int(args[0]), int(args[1])),
            "cmd": lambda: client.cmd(int(args[0]), int(args[1], 0)),
            "attr-read": lambda: client.attr_read(int(args[0]), *args[1:]),
            "discovery": lambda: client.discovery(args[0]),
            "op": lambda: client.op(args[0]),
            "wait-op": lambda: client.wait_op(args[0]),
            "snapshot": lambda: client.snapshot_states(),
            "restore": lambda: client.restore_states(_json.loads(args[0])),
        }
        result = table[sub]()
        if result is not None:
            print(_json.dumps(result, indent=1, sort_keys=True))
        return 0
    except ApiError as exc:
        print(str(exc), file=sys.stderr)
        return 2 if exc.status in (404, 422) else 1
    except LampNotAllowed as exc:
        print(str(exc), file=sys.stderr)
        return 1


def _cmd_lamps(rest):
    args = _flags("hil lamps", rest, "--no-baseline")
    from hil.api import Client
    from hil.camera.backend import probe_and_select
    from hil.camera.calibrate import load as load_cal
    from hil.camera.masks import load_geometry
    from hil.config import load as load_config
    from hil.oracle import CameraOracle
    from hil.results import dumps
    cfg = load_config()
    backend = probe_and_select(cfg)
    cal = load_cal(cfg)
    geometry = load_geometry(cfg)
    oracle = CameraOracle(backend, cal, geometry, cfg)
    api_client = Client(cfg)
    if not args.no_baseline:
        oracle.fresh_baseline(api_client)
    out = {}
    for label in sorted(geometry):
        try:
            out[str(label)] = oracle.measure(label)
        except Exception as exc:
            out[str(label)] = {"error": "%s: %s" % (type(exc).__name__, exc)}
    print(dumps({"camera_mode": cal.get("camera_mode"), "lamps": out},
                indent=1, sort_keys=True))
    backend.close()
    return 0


def _cmd_corpus(rest):
    from hil import corpus

    ap = _Parser(prog="hil corpus")
    ap.add_argument("parts", nargs="*", default=["all"],
                    choices=("all", "panel") + corpus.PARTS)
    ap.add_argument("--out", default=None,
                    help="corpus root (default: tools/hil/corpus)")
    ap.add_argument("--primary-only", action="store_true")
    ap.add_argument("--peer-only", action="store_true")
    ap.add_argument("--keep-secrets", action="store_true",
                    help="include the HA settings slice (broker password) — "
                         "only into an untracked --out")
    ap.add_argument("--wire-keep", type=int, default=corpus.WIRE_KEEP_DEFAULT)
    ap.add_argument("--panel-seconds", type=int, default=300,
                    help="with `panel`: how long to record button presses")
    ap.add_argument("--wire-scan-mb", type=int,
                    default=corpus.WIRE_SCAN_BYTES_DEFAULT // (1024 * 1024))
    args = ap.parse_args(rest)
    if args.parts == ["panel"]:
        from hil.config import load as load_cfg
        corpus.capture_panel(load_cfg(), seconds=args.panel_seconds,
                             out_root=args.out)
        return 0
    boards = "both"
    if args.primary_only:
        boards = "primary"
    if args.peer_only or os.environ.get("HIL_PEER") == "1":
        boards = "peer"
    index = corpus.run(parts=args.parts, out_root=args.out, boards=boards,
                       keep_secrets=args.keep_secrets,
                       wire_keep=args.wire_keep,
                       wire_scan=args.wire_scan_mb * 1024 * 1024)
    return 1 if any(b["rebooted_during_capture"]
                    for b in index["boards"].values()) else 0


def _cmd_state(rest):
    from pathlib import Path

    ap = _Parser(prog="hil state")
    ap.add_argument("action", choices=("save", "restore", "diff"))
    ap.add_argument("file", nargs="?")
    args = ap.parse_args(rest)
    from hil import prod_state
    from hil.api import Client
    from hil.config import load as load_config
    cfg = load_config()
    client = Client(cfg)
    path = Path(args.file) if args.file else prod_state.last_path(cfg)
    if args.action == "save":
        prod_state.save(prod_state.capture(client), path)
        print("saved %s" % path)
        return 0
    snap = prod_state.load(path)
    if args.action == "restore":
        residual = prod_state.restore(client, snap, drive_lamps=not cfg.lamps_read_only,
                                      lamp_shorts=cfg.lamp_short_set())
        if not residual:
            prod_state.mark_restored(path, snap)
    else:
        residual = prod_state.diff(snap, prod_state.capture(client))
        for line in residual:
            print(line)
    return 1 if residual else 0


COMMANDS = {
    "preflight": _cmd_preflight,
    "decode": _cmd_decode,
    "calibrate": _cmd_calibrate,
    "camera-server": _cmd_camera_server,
    "camera-bench": _cmd_camera_bench,
    "monitor": _cmd_monitor,
    "remote": _cmd_remote,
    "flash": _cmd_flash,
    "api": _cmd_api,
    "lamps": _cmd_lamps,
    "corpus": _cmd_corpus,
    "state": _cmd_state,
}


HELP_FLAGS = ("-h", "--help")
PEER_FLAG = "--peer"


def _print_help(cmd=None):
    print(USAGE)
    print("\ncommands: %s" % " | ".join(COMMANDS))
    print("`hil --peer <command>` runs it against the pair's other controller "
          "(HIL_PEER_BASE / HIL_PEER_SERIAL_PORT)")
    if cmd is not None:
        doc = COMMAND_HELP.get(cmd, "")
        print("\n%s: %s" % (cmd, doc or "no per-command help — see README.md"))
    return 0


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] == PEER_FLAG:
        os.environ["HIL_PEER"] = "1"
        argv = argv[1:]
    if not argv or argv[0] in HELP_FLAGS:
        return _print_help()
    cmd, rest = argv[0], argv[1:]
    handler = COMMANDS.get(cmd)
    if handler is None:
        print("unknown command: %s%s" % (cmd, " (`--peer` goes first)"
                                         if cmd == PEER_FLAG else ""), file=sys.stderr)
        return EX_USAGE
    if any(flag in rest for flag in HELP_FLAGS):
        return _print_help(cmd)
    try:
        return handler(rest)
    except UsageError as exc:
        print(str(exc), file=sys.stderr)
        return EX_USAGE


if __name__ == "__main__":
    sys.exit(main())
