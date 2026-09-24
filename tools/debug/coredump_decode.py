#!/usr/bin/env python3

from __future__ import annotations

import argparse
import base64
import re
import struct
import sys
from pathlib import Path

MARKER_START = "CORE DUMP START"
MARKER_END = "CORE DUMP END"
B64_LINE = re.compile(r"^[A-Za-z0-9+/=]+$")
ELF_MAGIC = b"\x7fELF"
PT_LOAD, PT_NOTE = 1, 4
NT_PRSTATUS = 1
PRSTATUS_PID_OFF = 24
PRSTATUS_REG_OFF = 72
EM_NAMES = {94: "xtensa", 243: "riscv (ESP32-P4)"}

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_LOG = REPO_ROOT / "tools/hil/state/persist/serial.log"
FIRMWARE_ELF = REPO_ROOT / "target/riscv32imafc-esp-espidf/debug/dali2rust"
DESCRIPTION = "Decode an ESP-IDF UART core dump out of a serial log and triage it."


def extract_blocks(text: str) -> list[tuple[list[str], bool]]:
    blocks: list[tuple[list[str], bool]] = []
    current: list[str] | None = None
    for raw in text.splitlines():
        line = raw.strip().strip("\x00")
        if MARKER_START in line:
            current = []
        elif MARKER_END in line:
            if current is not None:
                blocks.append((current, True))
            current = None
        elif current is not None:
            current.append(line)
    if current:
        blocks.append((current, False))
    return blocks


def decode_lines(lines: list[str]) -> tuple[bytes, int]:
    chunks: list[bytes] = []
    skipped = 0
    for raw in lines:
        tokens = raw.split()
        line = tokens[-1] if tokens else ""
        if not line or not B64_LINE.match(line):
            skipped += 1
            continue
        try:
            chunks.append(base64.b64decode(line, validate=True))
        except ValueError:
            skipped += 1
    return b"".join(chunks), skipped


def u16(b: bytes, off: int) -> int:
    return struct.unpack_from("<H", b, off)[0]


def u32(b: bytes, off: int) -> int:
    return struct.unpack_from("<I", b, off)[0]


def parse_program_headers(elf: bytes) -> list[tuple[int, int, int, int]]:
    phoff, phentsize, phnum = u32(elf, 28), u16(elf, 42), u16(elf, 44)
    out = []
    for i in range(phnum):
        base = phoff + i * phentsize
        out.append((u32(elf, base), u32(elf, base + 4), u32(elf, base + 8),
                    u32(elf, base + 16)))
    return out


def iter_notes(elf: bytes, seg_off: int, seg_len: int):
    pos, end = seg_off, seg_off + seg_len
    while pos + 12 <= end:
        namesz, descsz, ntype = struct.unpack_from("<III", elf, pos)
        pos += 12 + (namesz + 3 & ~3)
        desc = elf[pos:pos + descsz]
        pos += descsz + 3 & ~3
        yield ntype, desc


def guess_task_name(elf: bytes, loads: list[tuple[int, int, int, int]],
                    tcb: int) -> str:
    for _, p_offset, p_vaddr, p_filesz in loads:
        if not p_vaddr <= tcb < p_vaddr + p_filesz:
            continue
        window = elf[p_offset + (tcb - p_vaddr):
                     p_offset + min(tcb - p_vaddr + 352, p_filesz)]
        for match in re.finditer(rb"[A-Za-z_][A-Za-z0-9_\- ]{1,19}\x00", window):
            return match.group()[:-1].decode()
    return "<name not found>"


def summarize(elf: bytes) -> list[str]:
    machine = u16(elf, 18)
    lines = [f"ELF machine: {EM_NAMES.get(machine, machine)}"]
    headers = parse_program_headers(elf)
    loads = [h for h in headers if h[0] == PT_LOAD]
    tasks = []
    for p_type, p_offset, _, p_filesz in headers:
        if p_type != PT_NOTE:
            continue
        for ntype, desc in iter_notes(elf, p_offset, p_filesz):
            if ntype != NT_PRSTATUS or len(desc) < PRSTATUS_REG_OFF + 8:
                continue
            tcb = u32(desc, PRSTATUS_PID_OFF)
            pc = u32(desc, PRSTATUS_REG_OFF)
            tasks.append((tcb, pc, guess_task_name(elf, loads, tcb)))
    lines.append(f"tasks: {len(tasks)} (first note = the CRASHED task)")
    for idx, (tcb, pc, name) in enumerate(tasks):
        tag = "  << CRASHED" if idx == 0 else ""
        lines.append(f"  [{idx:2}] TCB=0x{tcb:08x} PC=0x{pc:08x} {name}{tag}")
    return lines


def next_steps(elf_path: Path) -> str:
    espcoredump = (REPO_ROOT / ".embuild/espressif/esp-idf/v5.5.3/"
                   "components/espcoredump/espcoredump.py")
    return "\n".join([
        "next steps:",
        f"  python3 {espcoredump} info_corefile -t elf -c {elf_path} \\",
        f"      {FIRMWARE_ELF}",
        "  (if it aborts on an app-SHA mismatch — esp-idf-sys leaves the app",
        "   SHA zeroed — comment out the `self._sha256_validate()` call in the",
        "   installed esp_coredump/corefile/loader.py; see the runbook)",
        "  ~/.espressif/tools/esp-clang/*/esp-clang/bin/llvm-symbolizer \\",
        f"      -e {FIRMWARE_ELF} <PC>",
    ])


def main() -> int:
    ap = argparse.ArgumentParser(description=DESCRIPTION)
    ap.add_argument("log", nargs="?", type=Path, default=DEFAULT_LOG,
                    help=f"serial log to scan (default: {DEFAULT_LOG})")
    ap.add_argument("--index", type=int, default=-1,
                    help="which dump to decode when the log has several "
                         "(0-based; default: -1 = the most recent)")
    ap.add_argument("--out", type=Path, default=Path("coredump.elf"),
                    help="where to write the extracted ELF core")
    ap.add_argument("--list", action="store_true",
                    help="only list the dumps found in the log")
    args = ap.parse_args()

    if not args.log.exists():
        print(f"error: log not found: {args.log}", file=sys.stderr)
        return 2
    blocks = extract_blocks(args.log.read_text(errors="replace"))
    if not blocks:
        print(f"no '{MARKER_START}' block in {args.log}", file=sys.stderr)
        return 2
    if args.list:
        for i, (lines, complete) in enumerate(blocks):
            state = "complete" if complete else "TRUNCATED"
            print(f"[{i}] {len(lines)} lines, {state}")
        return 0

    lines, complete = blocks[args.index]
    blob, skipped = decode_lines(lines)
    if not complete:
        print("warning: dump is TRUNCATED (no END marker) — decoding what "
              "is there", file=sys.stderr)
    if skipped:
        print(f"warning: skipped {skipped} non-base64 line(s)", file=sys.stderr)

    magic_at = blob.find(ELF_MAGIC)
    if magic_at < 0:
        print("error: no ELF magic in the decoded blob", file=sys.stderr)
        return 1
    if magic_at != 24:
        print(f"note: esp_core_dump header is {magic_at} bytes "
              "(24 expected as built)", file=sys.stderr)
    elf = blob[magic_at:]
    args.out.write_bytes(elf)
    print(f"wrote {args.out} ({len(elf)} bytes, header {magic_at} B stripped)")
    for line in summarize(elf):
        print(line)
    print(next_steps(args.out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
