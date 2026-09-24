#!/usr/bin/env python3

import re
import subprocess
import sys

HEAD = re.compile(r"^([0-9a-f]+) <(.+)>:$")
ADDI_SP = re.compile(r"\baddi\s+sp,sp,(-?\d+)")
CADDI_SP = re.compile(r"\bc\.addi16sp\s+sp,(-?\d+)")
SUB_SP = re.compile(r"\bsub\s+sp,sp,(\w+)")
LOAD_IMM = re.compile(r"\b(?:li|addi)\s+(\w+),(?:zero,)?(-?\d+)")
LUI = re.compile(r"\blui\s+(\w+),(0x[0-9a-f]+|-?\d+)")
OUTLINED_PROLOGUE = re.compile(r"\bjalr\s+t0,.*#\s*([0-9a-f]+)\s+<OUTLINED_FUNCTION_\d+>")

USAGE = """usage: python3 scripts/measure_stack_frames.py <objdump> <elf> [substring]

Prints the bytes each function of a linked image takes off sp, counting every
stack adjustment, deepest first. <objdump> is riscv32-esp-elf-objdump from the
ESP toolchain; [substring] filters the demangled names."""


def frames(disassembly):
    current = None
    names = {}
    sizes = {}
    regs = {}
    prologues = {}
    for line in disassembly.splitlines():
        head = HEAD.match(line)
        if head:
            current, regs = int(head.group(1), 16), {}
            names[current] = head.group(2)
            sizes[current] = 0
            continue
        if current is None:
            continue
        outlined = OUTLINED_PROLOGUE.search(line)
        if outlined:
            prologues.setdefault(current, set()).add(int(outlined.group(1), 16))
        lui = LUI.search(line)
        if lui:
            regs[lui.group(1)] = int(lui.group(2), 0) << 12
        imm = LOAD_IMM.search(line)
        if imm and "sp,sp" not in line:
            base = regs.get(imm.group(1), 0) if "addi" in line else 0
            regs[imm.group(1)] = base + int(imm.group(2))
        for pattern in (ADDI_SP, CADDI_SP):
            match = pattern.search(line)
            if match and int(match.group(1)) < 0:
                sizes[current] -= int(match.group(1))
        sub = SUB_SP.search(line)
        if sub:
            sizes[current] += regs.get(sub.group(1), 0)
    total = {addr: size + sum(sizes.get(f, 0) for f in prologues.get(addr, ())) for addr, size in sizes.items()}
    return [(total[addr], names[addr]) for addr in total]


def main():
    if len(sys.argv) < 3:
        print(USAGE)
        return 2
    objdump, elf = sys.argv[1], sys.argv[2]
    needle = sys.argv[3] if len(sys.argv) > 3 else None
    text = subprocess.run(
        [objdump, "-d", "--demangle", elf], capture_output=True, text=True, check=True
    ).stdout
    rows = [(size, name) for size, name in frames(text) if size > 0]
    if needle:
        rows = [row for row in rows if needle in row[1]]
    for size, name in sorted(rows, reverse=True):
        print(f"{size:7d}  {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
