import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import measure_stack_frames as frames

BASE = 0x48000000
INSN_BYTES = 4
ANNOTATION = " # 1fa0 <CSR_UINTSTATUS+0x12ef>"
SECOND_BASE = 0x48000100


def listing(address, name, instructions):
    lines = [f"{address:08x} <{name}>:"]
    for index, text in enumerate(instructions):
        lines.append(f"{address + INSN_BYTES * index:08x}:\t{text}")
    return "\n".join(lines) + "\n"


def frame_of(*instructions):
    functions = frames.parse(listing(BASE, "demo", instructions))
    return frames.frame(functions, BASE)


class PlainAdjustments(unittest.TestCase):
    def test_addi_sp_takes_its_immediate(self):
        self.assertEqual(frame_of("addi\tsp,sp,-48"), 48)

    def test_positive_addi_sp_is_an_epilogue(self):
        self.assertEqual(frame_of("addi\tsp,sp,-48", "addi\tsp,sp,48"), 48)

    def test_compressed_addi16sp_takes_its_immediate(self):
        self.assertEqual(frame_of("c.addi16sp\tsp,-64"), 64)

    def test_hex_immediate_on_sp(self):
        self.assertEqual(frame_of("addi\tsp,sp,-0x30"), 48)


class LuiWithAddi(unittest.TestCase):
    def test_negative_low_part_is_subtracted(self):
        self.assertEqual(
            frame_of(
                "lui\tt0,0x2",
                f"addi\tt0,t0,-1664{ANNOTATION}",
                "sub\tsp,sp,t0",
            ),
            6528,
        )

    def test_positive_low_part_is_added(self):
        self.assertEqual(
            frame_of("lui\tt0,0x1", "addi\tt0,t0,1024", "sub\tsp,sp,t0"),
            5120,
        )

    def test_positive_hex_low_part_is_added(self):
        self.assertEqual(
            frame_of("lui\tt0,0x1", "addi\tt0,t0,0x400", "sub\tsp,sp,t0"),
            5120,
        )

    def test_negative_hex_low_part_is_subtracted(self):
        self.assertEqual(
            frame_of("lui\tt0,0x2", "addi\tt0,t0,-0x680", "sub\tsp,sp,t0"),
            6528,
        )

    def test_addiw_combines_like_addi(self):
        self.assertEqual(
            frame_of("lui\tt0,0x2", "addiw\tt0,t0,-1664", "sub\tsp,sp,t0"),
            6528,
        )

    def test_compressed_lui_and_addi_combine(self):
        self.assertEqual(
            frame_of("c.lui\tt0,0x2", "c.addi\tt0,-1664", "sub\tsp,sp,t0"),
            6528,
        )

    def test_frame_after_a_register_save_adds_up(self):
        self.assertEqual(
            frame_of(
                "addi\tsp,sp,-256",
                "sw\ts8,216(sp)",
                "lui\ta6,0x2",
                f"addi\ta6,a6,-96{ANNOTATION}",
                "sub\tsp,sp,a6",
            ),
            256 + 8192 - 96,
        )

    def test_epilogue_register_add_does_not_count(self):
        self.assertEqual(
            frame_of(
                "lui\ta6,0x2",
                "addi\ta6,a6,-96",
                "sub\tsp,sp,a6",
                "lui\ta0,0x2",
                "addi\ta0,a0,-96",
                "add\tsp,sp,a0",
            ),
            8192 - 96,
        )

    def test_negative_register_added_to_sp_grows_the_frame(self):
        self.assertEqual(
            frame_of("lui\tt0,0xffffe", "addi\tt0,t0,-96", "add\tsp,sp,t0"),
            8192 + 96,
        )


class LoadedConstants(unittest.TestCase):
    def test_li_register_is_the_frame(self):
        self.assertEqual(frame_of("li\tt0,4000", "sub\tsp,sp,t0"), 4000)

    def test_addi_from_zero_is_a_load(self):
        self.assertEqual(frame_of("addi\tt0,zero,1500", "sub\tsp,sp,t0"), 1500)

    def test_move_copies_the_constant(self):
        self.assertEqual(
            frame_of("lui\ta5,0x2", "addi\ta5,a5,-96", "mv\ta6,a5", "sub\tsp,sp,a6"),
            8192 - 96,
        )

    def test_store_keeps_the_register(self):
        self.assertEqual(
            frame_of("lui\tt0,0x2", "sw\tt0,0(sp)", "sub\tsp,sp,t0"),
            8192,
        )

    def test_branch_keeps_the_register(self):
        self.assertEqual(
            frame_of(
                "lui\tt0,0x2",
                f"beqz\tt0,{BASE:x} <demo>",
                "sub\tsp,sp,t0",
            ),
            8192,
        )


class UnknownRegisters(unittest.TestCase):
    def test_run_time_size_is_not_a_stale_constant(self):
        self.assertEqual(
            frame_of("li\ta5,20", "andi\ta5,a5,-16", "sub\tsp,sp,a5"),
            0,
        )

    def test_run_time_size_after_addi_is_not_a_constant(self):
        self.assertEqual(
            frame_of(
                "lui\ta5,0x2",
                "mul\ta5,a0,a5",
                "addi\ta5,a5,15",
                "sub\tsp,sp,a5",
            ),
            0,
        )

    def test_call_clobbers_the_register(self):
        self.assertEqual(
            frame_of(
                "lui\ta0,0x2",
                "auipc\tra,0x7",
                "jalr\t-676(ra) # 48034132 <callee>",
                "sub\tsp,sp,a0",
            ),
            0,
        )

    def test_registers_do_not_cross_functions(self):
        text = listing(BASE, "first", ["lui\tt0,0x2"]) + listing(
            SECOND_BASE, "second", ["sub\tsp,sp,t0"]
        )
        functions = frames.parse(text)
        self.assertEqual(frames.frame(functions, SECOND_BASE), 0)


if __name__ == "__main__":
    unittest.main()
