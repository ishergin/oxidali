#!/usr/bin/env python3
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

ISR_SYMBOL_HINTS = ("dali2rust_dali_phy",)

ENTRY_SYMBOLS = ("dali_phy_raw_isr", "dali_phy_alarm_isr")

_FUNC = re.compile(r"^([0-9a-f]+) <(.+)>:")
_TARGET = re.compile(r"#\s*([0-9a-f]{8})\s*<([^>]+)>")
_DIRECT = re.compile(r"\b(?:jal|j)\s+(?:ra,)?([0-9a-f]{8})\s*<([^>]+)>")
_MNEMONIC = re.compile(r"^\s*[0-9a-f]+:\s+(?:[0-9a-f]{2,8}\s+)+(\S+)")

_CALL_OPS = {
    "jal", "jalr", "j", "jr", "call", "tail",
    "c.j", "c.jr", "c.jal", "c.jalr",
}
_MEM_OPS = {
    "lw", "lh", "lhu", "lb", "lbu", "lwu", "ld",
    "sw", "sh", "sb", "sd",
    "flw", "fsw", "fld", "fsd",
    "c.lw", "c.sw", "c.lwsp", "c.swsp",
}

_WHY = {
    "call": "executing a flash instruction with the cache off is the Cache access error",
    "load": "reading flash-mapped data with the cache off faults the same way",
    "address": "forming a flash pointer means an operand of a call that must not exist "
               "(panic Location structs, message strings)",
}


def _tool(name, require):
    hits = sorted(Path.home().glob(
        ".espressif/tools/riscv32-esp-elf/*/riscv32-esp-elf/bin/riscv32-esp-elf-" + name))
    if not hits:
        print("verify_dali_isr_iram: SKIP (riscv32-esp-elf-%s not installed)" % name)
        sys.exit(2 if require else 0)
    return str(hits[-1])


def _sections(readelf, elf):
    out = subprocess.run([readelf, "-S", "-W", elf], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        m = re.match(r"\s*\[\s*\d+\]\s+(\S+)\s+(\S+)\s+([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)",
                     line)
        if not m:
            continue
        name, kind, addr, size = m.group(1), m.group(2), int(m.group(3), 16), int(m.group(4), 16)
        if kind in ("PROGBITS", "NOBITS") and addr:
            rows.append((name, addr, addr + size))
    return rows


def _iram_bounds(sections, elf):
    for name, lo, hi in sections:
        if name == ".iram0.text":
            return lo, hi
    raise SystemExit("verify_dali_isr_iram: no .iram0.text in %s" % elf)


def _flash_ranges(sections, elf):
    ranges = [(name, lo, hi) for name, lo, hi in sections
              if name.startswith((".flash", ".drom", ".irom")) and hi > lo]
    if not ranges:
        raise SystemExit(
            "verify_dali_isr_iram: no flash-mapped sections in %s — the section "
            "names changed and this check would pass vacuously" % elf)
    return ranges


def _flash_hit(addr, flash_ranges):
    for name, lo, hi in flash_ranges:
        if lo <= addr < hi:
            return name
    return None


def _classify(line):
    m = _MNEMONIC.match(line)
    if not m:
        return "address"
    op = m.group(1)
    if op in _CALL_OPS:
        return "call"
    if op in _MEM_OPS:
        return "load"
    return "address"


def main(argv):
    args = [a for a in argv[1:] if a != "--require-tools"]
    require_tools = "--require-tools" in argv[1:]

    elf = args[0] if args else str(ROOT / "target/riscv32imafc-esp-espidf/debug/dali2rust")
    if not Path(elf).exists():
        print("verify_dali_isr_iram: SKIP (%s not built)" % elf)
        return 2 if require_tools else 0

    objdump, readelf = _tool("objdump", require_tools), _tool("readelf", require_tools)
    sections = _sections(readelf, elf)
    lo, hi = _iram_bounds(sections, elf)
    flash = _flash_ranges(sections, elf)
    dis = subprocess.run([objdump, "-d", "--section=.iram0.text", "-C", elf],
                         capture_output=True, text=True).stdout

    current = None
    seen_symbols = set()
    bad = {"call": {}, "load": {}, "address": {}}
    for line in dis.splitlines():
        m = _FUNC.match(line)
        if m:
            current = m.group(2)
            if any(h in current for h in ISR_SYMBOL_HINTS):
                seen_symbols.add(current)
            continue
        if current is None or not any(h in current for h in ISR_SYMBOL_HINTS):
            continue
        for pattern in (_TARGET, _DIRECT):
            m = pattern.search(line)
            if not m:
                continue
            addr, name = int(m.group(1), 16), m.group(2)
            section = _flash_hit(addr, flash)
            if section:
                kind = _classify(line)
                bad[kind].setdefault(current, set()).add(
                    "0x%08x %s  [%s]" % (addr, name, section))

    table = subprocess.run([objdump, "-t", "-C", elf],
                           capture_output=True, text=True).stdout
    linked = [e for e in ENTRY_SYMBOLS if e in table]
    missing = [e for e in linked if not any(e in seen for seen in seen_symbols)]
    if not linked:
        missing = list(ENTRY_SYMBOLS)
    if missing:
        print("verify_dali_isr_iram: FAILED — interrupt entry point(s) not in IRAM: %s"
              % ", ".join(missing))
        print("The hint list is stale (crate renamed? symbols inlined away?).")
        print("This is not a pass: nothing was actually checked.")
        return 1

    if any(bad.values()):
        print("verify_dali_isr_iram: FAILED — the PHY interrupt reaches flash")
        print("IRAM is 0x%08x..0x%08x; see ISSUE-27 and ADR-010." % (lo, hi))
        for kind in ("call", "load", "address"):
            if not bad[kind]:
                continue
            print("\n  %s — %s:" % (kind.upper(), _WHY[kind]))
            for fn in sorted(bad[kind]):
                print("    %s" % fn)
                for t in sorted(bad[kind][fn]):
                    print("        -> %s" % t)
        return 1

    print("verify_dali_isr_iram: OK (%d PHY symbols in IRAM, none reaches flash)"
          % len(seen_symbols))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
