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
CONTROL_RECV_BYTES = 64

RESET_HOLD_S = 0.1
RESET_LATCH_S = 0.05


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
    def __init__(self, ser, sock, log):
        self.serial, self.socket, self.log = ser, sock, log
        self._write_lock = threading.Lock()
        self.rfc2217 = serial.rfc2217.PortManager(LinesPinned(self.serial, log), self)

    def write(self, data):
        with self._write_lock:
            self.socket.sendall(data)

    def run(self):
        while True:
            readable, _, _ = select.select(
                [self.socket, self.serial.fileno()], [], [], SELECT_TIMEOUT_S)
            if self.serial.fileno() in readable:
                data = self.serial.read(self.serial.in_waiting or 1)
                if data:
                    self.write(b"".join(self.rfc2217.escape(data)))
            if self.socket in readable:
                data = self.socket.recv(SOCKET_CHUNK_BYTES)
                if not data:
                    return
                self.serial.write(b"".join(self.rfc2217.filter(data)))


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

    def _control_reply(self, cmd):
        if cmd == "ping":
            return "ok %s %d" % (self.port, self.serial.baudrate)
        if cmd == "status":
            return "ok port=%s baud=%d client=%s uptime=%d" % (
                self.port, self.serial.baudrate,
                self.client or "none", time.time() - self.started_at)
        if cmd in ("bootloader", "run"):
            classic_reset(self.serial, cmd == "bootloader")
            self.log("control: reset -> %s" % cmd)
            return "ok %s" % cmd
        if cmd.startswith("baud "):
            self.serial.baudrate = int(cmd.split()[1])
            return "ok baud %d" % self.serial.baudrate
        return "err unknown command %r" % cmd

    def control_loop(self):
        srv = _listener(self.host, self.control_port)
        while True:
            sock, _ = srv.accept()
            try:
                sock.settimeout(5)
                cmd = sock.recv(CONTROL_RECV_BYTES).decode("ascii", "replace").strip()
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
            if self.client is not None:
                self.log("refused %s — %s already connected" % (peer, self.client))
                sock.close()
                continue
            self.client = peer
            self.log("client %s connected" % peer)
            try:
                Session(self.serial, sock, self.log).run()
            except Exception as exc:
                self.log("session %s: %r" % (peer, exc))
            finally:
                sock.close()
                self.client = None
                self.serial.baudrate = self.baud
                self.log("client %s gone" % peer)


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
