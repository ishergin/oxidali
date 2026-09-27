#!/usr/bin/env python3
import os
import select
import socket
import sys
import threading
import time

import serial
import serial.rfc2217

SELECT_TIMEOUT_S = 0.2
SERIAL_READ_TIMEOUT_S = 0
SOCKET_CHUNK_BYTES = 65536
CONTROL_RECV_BYTES = 256
CONTROL_MAX_BYTES = 4096
CONTROL_TAIL_TIMEOUT_S = 0.2

RESET_HOLD_S = 0.1
RESET_LATCH_S = 0.05

PORT_GONE = "port-gone"
PORT_VERBS = ("release", "reacquire", "write")


class SerialFault(RuntimeError):
    pass


def _serial_io(operation):
    try:
        return operation()
    except (serial.SerialException, OSError) as exc:
        raise SerialFault(str(exc)) from exc


def _node_device(path):
    return os.stat(path).st_rdev


def _held_device(ser):
    return os.fstat(ser.fileno()).st_rdev


class LinesPinned:
    def __init__(self, ser, log):
        object.__setattr__(self, "_ser", ser)
        object.__setattr__(self, "_log", log)
        object.__setattr__(self, "_warned", set())

    def __getattr__(self, name):
        return getattr(self._ser, name)

    def __setattr__(self, name, value):
        if name in ("dtr", "rts"):
            if name not in self._warned:
                self._warned.add(name)
                self._log("ignored client %s=%r: EN/IO0 move only via the control port"
                          % (name, value))
            return
        setattr(self._ser, name, value)


class Session:
    def __init__(self, ser, sock, log, port_lock):
        self.serial, self.socket, self.log = ser, sock, log
        self._write_lock = threading.Lock()
        self._port_lock = port_lock
        self.rfc2217 = serial.rfc2217.PortManager(LinesPinned(self.serial, log), self)

    def write(self, data):
        with self._write_lock:
            self.socket.sendall(data)

    def run(self):
        while True:
            readable, _, _ = select.select(
                [self.socket, self.serial.fileno()], [], [], SELECT_TIMEOUT_S)
            if self.serial.fileno() in readable:
                data = _serial_io(lambda: self.serial.read(self.serial.in_waiting or 1))
                if data:
                    self.write(b"".join(self.rfc2217.escape(data)))
            if self.socket in readable:
                data = self.socket.recv(SOCKET_CHUNK_BYTES)
                if not data:
                    return
                payload = b"".join(self.rfc2217.filter(data))
                with self._port_lock:
                    _serial_io(lambda: self.serial.write(payload))


def classic_reset(ser, into_bootloader):
    ser.dtr = False
    ser.rts = True
    time.sleep(RESET_HOLD_S)
    if into_bootloader:
        ser.dtr = True
    ser.rts = False
    time.sleep(RESET_LATCH_S)
    ser.dtr = False


class Bridge:
    def __init__(self, port, baud, host, data_port, control_port):
        self.port, self.baud = port, baud
        self.host, self.data_port, self.control_port = host, data_port, control_port
        self.serial = None
        self.client = None
        self.serial_fault = None
        self.released = False
        self.port_lock = threading.Lock()
        self.started_at = time.time()

    def log(self, msg):
        sys.stderr.write("%s %s\n" % (time.strftime("%Y-%m-%dT%H:%M:%S"), msg))
        sys.stderr.flush()

    def open_serial(self):
        ser = serial.serial_for_url(self.port, do_not_open=True)
        ser.baudrate, ser.timeout = self.baud, SERIAL_READ_TIMEOUT_S
        ser.dtr = False
        ser.rts = False
        ser.open()
        self.serial = ser

    def port_problem(self):
        try:
            node = _node_device(self.port)
        except OSError as exc:
            return "%s is gone (%s)" % (self.port, exc.strerror or exc)
        try:
            held = _held_device(self.serial)
        except (OSError, serial.SerialException) as exc:
            return "the bridge's handle on %s is closed (%s)" % (self.port, exc)
        if node != held:
            return ("%s was enumerated again and the bridge still holds the device "
                    "that vanished" % self.port)
        if self.serial_fault:
            return "%s failed under a client (%s)" % (self.port, self.serial_fault)
        return None

    def _control_reply(self, cmd):
        if self.released:
            return self._released_reply(cmd)
        if cmd == "release":
            return self._release()
        problem = self.port_problem()
        if problem:
            return "err %s: %s" % (PORT_GONE, problem)
        if cmd.startswith("write "):
            return self._write_line(cmd[len("write "):])
        if cmd == "ping":
            return "ok %s %d" % (self.port, self.serial.baudrate)
        if cmd == "status":
            return "ok port=%s baud=%d client=%s uptime=%d verbs=%s" % (
                self.port, self.serial.baudrate,
                self.client or "none", time.time() - self.started_at, ",".join(PORT_VERBS))
        if cmd in ("bootloader", "run"):
            classic_reset(self.serial, cmd == "bootloader")
            self.log("control: reset -> %s" % cmd)
            return "ok %s" % cmd
        if cmd.startswith("baud "):
            self.serial.baudrate = int(cmd.split()[1])
            return "ok baud %d" % self.serial.baudrate
        return "err unknown command %r" % cmd

    def _released_reply(self, cmd):
        if cmd == "reacquire":
            self.open_serial()
            self.released = False
            self.log("control: port reacquired (opening it resets the board)")
            return "ok reacquired"
        if cmd in ("ping", "status"):
            return "ok port=%s released" % self.port
        return "err released: the port is closed for a write on this host; reacquire first"

    def _release(self):
        if self.client is not None:
            return "err busy: %s holds the data port; stop it first" % self.client
        with self.port_lock:
            self.serial.close()
            self.released = True
        self.log("control: port released for a write on this host")
        return "ok released"

    def _write_line(self, text):
        data = (text + "\n").encode("ascii", "replace")
        with self.port_lock:
            _serial_io(lambda: self.serial.write(data))
        return "ok wrote %d" % len(data)

    def control_loop(self):
        srv = _listener(self.host, self.control_port)
        while True:
            sock, _ = srv.accept()
            try:
                sock.settimeout(5)
                cmd = _read_command(sock)
                sock.sendall((self._control_reply(cmd) + "\n").encode())
            except Exception as exc:
                self.log("control error: %r" % (exc,))
            finally:
                sock.close()

    def serve(self):
        self.open_serial()
        self.log("bridge up: %s @%d data=%d control=%d"
                 % (self.port, self.baud, self.data_port, self.control_port))
        threading.Thread(target=self.control_loop, daemon=True).start()
        srv = _listener(self.host, self.data_port)
        while True:
            sock, addr = srv.accept()
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            peer = "%s:%d" % addr
            if self.client is not None or self.released:
                self.log("refused %s — %s" % (
                    peer, "the port is released" if self.released
                    else "%s already connected" % self.client))
                sock.close()
                continue
            self.serve_client(sock, peer)

    def serve_client(self, sock, peer):
        self.client = peer
        self.log("client %s connected" % peer)
        try:
            Session(self.serial, sock, self.log, self.port_lock).run()
        except SerialFault as exc:
            self.serial_fault = str(exc)
            self.log("session %s: serial port failed: %s" % (peer, exc))
        except Exception as exc:
            self.log("session %s: %r" % (peer, exc))
        finally:
            sock.close()
            self.client = None
            self._restore_baud()
            self.log("client %s gone" % peer)

    def _restore_baud(self):
        try:
            _serial_io(lambda: setattr(self.serial, "baudrate", self.baud))
        except SerialFault as exc:
            self.serial_fault = self.serial_fault or str(exc)


def _read_command(sock):
    data = sock.recv(CONTROL_RECV_BYTES)
    sock.settimeout(CONTROL_TAIL_TIMEOUT_S)
    while data and b"\n" not in data and len(data) < CONTROL_MAX_BYTES:
        try:
            chunk = sock.recv(CONTROL_RECV_BYTES)
        except socket.timeout:
            break
        if not chunk:
            break
        data += chunk
    return data.split(b"\n", 1)[0].decode("ascii", "replace").strip()


def _listener(host, port):
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, port))
    srv.listen(1)
    return srv


def main(argv):
    if len(argv) != 5:
        sys.stderr.write(
            "usage: serial_bridge.py <port> <baud> <listen-host> <data-port> "
            "<control-port>\n")
        return 64
    port, baud, host, data_port, control_port = argv
    Bridge(port, int(baud), host, int(data_port), int(control_port)).serve()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
