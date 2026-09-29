#!/usr/bin/env python3

import re
import subprocess
import sys

HEAD = re.compile(r"^([0-9a-f]+) <(.+)>:$")
INSN = re.compile(r"^\s*[0-9a-f]+:\s+(\S+)\s*(.*)$")
ADDI_SP = re.compile(r"\baddi\s+sp,sp,(-?\d+)")
CADDI_SP = re.compile(r"\bc\.addi16sp\s+sp,(-?\d+)")
SUB_SP = re.compile(r"\bsub\s+sp,sp,(\w+)")
LOAD_IMM = re.compile(r"\b(?:li|addi)\s+(\w+),(?:zero,)?(-?\d+)")
LUI = re.compile(r"\blui\s+(\w+),(0x[0-9a-f]+|-?\d+)")
OUTLINED_PROLOGUE = re.compile(r"\bjalr\s+t0,.*#\s*([0-9a-f]+)\s+<OUTLINED_FUNCTION_\d+>")
ANNOTATED_TARGET = re.compile(r"#\s*([0-9a-f]+)\s+<.+>")
DIRECT_TARGET = re.compile(r"^(?:(\w+),)?([0-9a-f]+)\s+<.+>")
NEVER_RETURNS = re.compile(
    r"^(?:abort|__assert_func|_esp_error_check_failed|esp_system_abort|panic_abort"
    r"|_panic_handler|panicHandler|esp_panic_handler)$"
    r"|^__rustc::(?:__rust_abort|__rust_start_panic|rust_panic|rust_begin_unwind"
    r"|__rust_alloc_error_handler)"
    r"|^core::panicking::|^std::panicking::(?:begin_panic|rust_panic|panic_with_hook"
    r"|panic_handler|default_hook)|__rust_end_short_backtrace"
    r"|^core::(?:result|option)::(?:unwrap|expect)_failed"
    r"|^alloc::(?:alloc::handle_alloc_error|raw_vec::(?:capacity_overflow|handle_error))"
    r"|^core::(?:slice|str)::\S*(?:_fail|do_panic)|^core::cell::panic_already"
)

USAGE = """usage: python3 scripts/measure_stack_frames.py <objdump> <elf> [substring]
       python3 scripts/measure_stack_frames.py <objdump> <elf> --path <function> [--skip <regex>]

Prints the bytes each function of a linked image takes off sp, counting every
stack adjustment, deepest first. <objdump> is riscv32-esp-elf-objdump from the
ESP toolchain; [substring] filters the demangled names.

With --path, prints the deepest static call path below every function whose
demangled name ends with <function>, frame by frame. Direct calls and tail
calls are followed; indirect calls (trait objects, function pointers, the log
facade) are not, and each frame shows how many it makes. Functions that never
return (panics, aborts, failed asserts) are left out, and so is every function
whose name matches --skip. Error branches the census never runs are counted, so
the path is an estimate that names where the depth is, not a bound."""


class Function:
    def __init__(self, name):
        self.name = name
        self.size = 0
        self.prologues = set()
        self.calls = set()
        self.tails = set()
        self.indirect = 0


def note_edge(function, mnemonic, operands):
    if mnemonic in ("jalr", "c.jalr"):
        target = ANNOTATED_TARGET.search(operands)
        if target is None:
            function.indirect += 1
        elif not operands.startswith("t0,"):
            function.calls.add(int(target.group(1), 16))
    elif mnemonic in ("jal", "c.jal"):
        target = DIRECT_TARGET.match(operands)
        if target and target.group(1) in (None, "ra"):
            function.calls.add(int(target.group(2), 16))
    elif mnemonic in ("jr", "c.jr"):
        target = ANNOTATED_TARGET.search(operands)
        if target:
            function.tails.add(int(target.group(1), 16))
    elif mnemonic in ("j", "c.j"):
        target = DIRECT_TARGET.match(operands)
        if target:
            function.tails.add(int(target.group(2), 16))


def note_stack(function, line, regs):
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
            function.size -= int(match.group(1))
    sub = SUB_SP.search(line)
    if sub:
        function.size += regs.get(sub.group(1), 0)


def parse(disassembly):
    functions = {}
    current = None
    regs = {}
    for line in disassembly.splitlines():
        head = HEAD.match(line)
        if head:
            current = Function(head.group(2))
            functions[int(head.group(1), 16)] = current
            regs = {}
            continue
        if current is None:
            continue
        outlined = OUTLINED_PROLOGUE.search(line)
        if outlined:
            current.prologues.add(int(outlined.group(1), 16))
        note_stack(current, line, regs)
        insn = INSN.match(line)
        if insn:
            note_edge(current, insn.group(1), insn.group(2))
    return functions


def frame(functions, addr):
    function = functions[addr]
    return function.size + sum(
        functions[p].size for p in function.prologues if p in functions
    )


def deepest(functions, addr, walkable, memo, open_calls):
    if addr in memo:
        return memo[addr]
    if addr in open_calls:
        return 0, []
    open_calls.add(addr)
    function = functions[addr]
    callee_depth, callee_path = 0, []
    for callee in function.calls & walkable:
        depth, path = deepest(functions, callee, walkable, memo, open_calls)
        if depth > callee_depth:
            callee_depth, callee_path = depth, path
    best = (frame(functions, addr) + callee_depth, [addr] + callee_path)
    for tail in (function.tails & walkable) - {addr}:
        depth, path = deepest(functions, tail, walkable, memo, open_calls)
        if depth > best[0]:
            best = (depth, [addr] + path)
    open_calls.discard(addr)
    memo[addr] = best
    return best


def print_paths(functions, root, skip):
    walkable = {
        addr
        for addr, function in functions.items()
        if not NEVER_RETURNS.search(function.name) and not (skip and skip.search(function.name))
    }
    memo = {}
    roots = [addr for addr, function in functions.items() if function.name.endswith(root)]
    for addr in roots:
        depth, path = deepest(functions, addr, walkable, memo, set())
        print(f"{depth:7d}  {functions[addr].name}")
        for step in path:
            indirect = functions[step].indirect
            marker = f"  [{indirect} indirect]" if indirect else ""
            print(f"         {frame(functions, step):5d}  {functions[step].name}{marker}")
    return 0 if roots else 1


def main():
    if len(sys.argv) < 3:
        print(USAGE)
        return 2
    objdump, elf = sys.argv[1], sys.argv[2]
    args = sys.argv[3:]
    text = subprocess.run(
        [objdump, "-d", "--demangle", "--no-show-raw-insn", elf],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    functions = parse(text)
    if args and args[0] == "--path":
        if len(args) < 2:
            print(USAGE)
            return 2
        skip = re.compile(args[3]) if len(args) > 3 and args[2] == "--skip" else None
        sys.setrecursionlimit(100_000)
        return print_paths(functions, args[1], skip)
    needle = args[0] if args else None
    rows = [(frame(functions, addr), f.name) for addr, f in functions.items()]
    rows = [row for row in rows if row[0] > 0 and (needle is None or needle in row[1])]
    for size, name in sorted(rows, reverse=True):
        print(f"{size:7d}  {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
