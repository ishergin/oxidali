#!/usr/bin/env python3

import re
import subprocess
import sys

HEAD = re.compile(r"^([0-9a-f]+) <(.+)>:$")
INSN = re.compile(r"^\s*[0-9a-f]+:\s+(\S+)\s*(.*)$")
IMMEDIATE = r"-?(?:0x[0-9a-f]+|\d+)"
SP_IMMEDIATE = re.compile(rf"^sp,(?:sp,)?({IMMEDIATE})$")
SP_REGISTER = re.compile(r"^sp,sp,(\w+)$")
SP_IMMEDIATE_MNEMONICS = ("addi", "c.addi16sp", "c.addi")
SP_REGISTER_MNEMONICS = ("add", "sub")
LUI_MNEMONICS = ("lui", "c.lui")
LI_MNEMONICS = ("li", "c.li")
MOVE_MNEMONICS = ("mv", "c.mv")
ADDI_MNEMONICS = ("addi", "addiw", "c.addi")
CALL_MNEMONICS = ("jal", "jalr", "c.jal", "c.jalr")
NO_DESTINATION = re.compile(
    r"^(?:s[bhw]|fs[wd]|c\.f?sw(?:sp)?|(?:c\.)?b(?:eq|ne|lt|ge|le|gt)u?z?|(?:c\.)?jr?"
    r"|ret|nop|fence\S*|ecall|ebreak|unimp|mret|wfi)$"
)
LUI_SHIFT = 12
WORD_BITS = 32
WORD_SIGN = 1 << (WORD_BITS - 1)
WORD_MODULUS = 1 << WORD_BITS
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


def wrap_word(value):
    return (value + WORD_SIGN) % WORD_MODULUS - WORD_SIGN


def split_operands(operands):
    return operands.split("#", 1)[0].strip().split(",")


def known(regs, name):
    return 0 if name == "zero" else regs.get(name)


def added_value(regs, mnemonic, parts):
    if mnemonic == "c.addi":
        base, immediate = known(regs, parts[0]), parts[1]
    else:
        base, immediate = known(regs, parts[1]), parts[2]
    return None if base is None else wrap_word(base + int(immediate, 0))


def loaded_value(regs, mnemonic, parts):
    if mnemonic in LUI_MNEMONICS:
        return wrap_word(int(parts[1], 0) << LUI_SHIFT)
    if mnemonic in LI_MNEMONICS:
        return int(parts[1], 0)
    if mnemonic in MOVE_MNEMONICS:
        return known(regs, parts[1])
    if mnemonic in ADDI_MNEMONICS:
        return added_value(regs, mnemonic, parts)
    return None


def track_register(regs, mnemonic, operands):
    parts = split_operands(operands)
    value = loaded_value(regs, mnemonic, parts)
    if value is not None:
        regs[parts[0]] = value
    elif mnemonic in CALL_MNEMONICS:
        regs.clear()
    elif not NO_DESTINATION.match(mnemonic):
        regs.pop(parts[0], None)


def register_growth(mnemonic, value):
    return max(value if mnemonic == "sub" else -value, 0)


def stack_growth(regs, mnemonic, operands):
    operands = operands.split("#", 1)[0].strip()
    immediate = SP_IMMEDIATE.match(operands)
    if immediate and mnemonic in SP_IMMEDIATE_MNEMONICS:
        return max(-int(immediate.group(1), 0), 0)
    register = SP_REGISTER.match(operands)
    if register and mnemonic in SP_REGISTER_MNEMONICS:
        return register_growth(mnemonic, known(regs, register.group(1)) or 0)
    return 0


def note_stack(function, regs, mnemonic, operands):
    function.size += stack_growth(regs, mnemonic, operands)
    track_register(regs, mnemonic, operands)


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
        insn = INSN.match(line)
        if insn:
            note_stack(current, regs, insn.group(1), insn.group(2))
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
