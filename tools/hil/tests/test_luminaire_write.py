import pytest

pytestmark = pytest.mark.virtual_gear

MEMORY_BANK_PROFILE = 1
READ_GROUPS = "runtime_status"
PROFILE_BANKS = "profile"
SECTIONS = ["memory_luminaire", "memory_profile"]
LUMINAIRE_FORMAT = "3"
FLEET_DEFAULT = "off"
BANK_UNLOCKED = 0x55
BANK_LOCKED = 0xAA
LOCK_BYTE_OFFSET = 0x02
YEAR_FIELD, YEAR_OFFSET, YEAR_LAST = "year", 0x13, 99
TEXT_FIELD, TEXT_OFFSET, TEXT_LEN = "luminaire_identification", 0x3C, 60
WRITTEN_TEXT = "HIL BANK 1 WRITE"
NUL = 0x00
WRITE_TYPE = "attribute_write"
OP_TIMEOUT_S = 120.0
CONTENDED_CODES = frozenset({"read_contended", "verify_contended", "preempted"})
CONTENDED_RETRY = "luminaire_write_contended"
PROGRESS = "transactions_started"
WIRE_GATES = ("transaction_budget_exceeded", "transaction_leaks")


@pytest.fixture()
def luminaire_gear(virtual_bench):
    short = virtual_bench.park[0]
    virtual_bench.sim.luminaire(short, LUMINAIRE_FORMAT)
    try:
        yield short, virtual_bench.sim
    finally:
        virtual_bench.sim.luminaire(short, FLEET_DEFAULT)


def _read(api, short, op_check):
    op_check(api.wait_op(api.attr_read(short, groups=READ_GROUPS, banks=PROFILE_BANKS),
                         timeout_s=OP_TIMEOUT_S))
    attributes = api.attributes(short, SECTIONS).get("attributes") or {}
    return ({name: (leaf or {}).get("value") for name, leaf in
             (attributes.get("memory_luminaire") or {}).items()},
            ((attributes.get("memory_profile") or {}).get("bank1_lock_byte") or {}).get("value"))


def _contended(view):
    return view.get("status") == "failed" and \
        (view.get("error") or {}).get("code") in CONTENDED_CODES


def _write(api, short, patch, op_check):
    def once():
        accepted = api.luminaire_write(short, patch)
        assert accepted.get("type") == WRITE_TYPE, accepted
        return api.wait_op(accepted, timeout_s=OP_TIMEOUT_S)
    view = once()
    if _contended(view):
        api.count_retry(CONTENDED_RETRY, "%r ended %r" % (patch, view.get("error")))
        view = once()
    return op_check(view)


def _text_cells(text):
    cells = list(text.encode("ascii"))
    return cells + [NUL] if len(cells) < TEXT_LEN else cells


def _wire(api):
    return api.diagnostics().get("dali_wire") or {}


def _delta(before, after, key):
    return after.get(key, 0) - before.get(key, 0)


def _assert_held(sim, short, year, text, lock, what):
    held = {"year": sim.peek(short, MEMORY_BANK_PROFILE, YEAR_OFFSET, 1),
            "text": sim.peek(short, MEMORY_BANK_PROFILE, TEXT_OFFSET, len(_text_cells(text))),
            "lock": sim.peek(short, MEMORY_BANK_PROFILE, LOCK_BYTE_OFFSET, 1)}
    assert held == {"year": [year], "text": _text_cells(text), "lock": [lock]}, (
        "%s: the emulated gear holds %r, so the read-back sections speak for bytes it does "
        "not keep" % (what, held))


def _assert_shown(found, lock_byte, year, text, lock, what):
    assert (found[YEAR_FIELD] or {}).get("raw") == year, (
        "%s: year reads %r, not %d" % (what, found[YEAR_FIELD], year))
    assert found[TEXT_FIELD] == text, "%s: %s reads %r" % (what, TEXT_FIELD, found[TEXT_FIELD])
    assert lock_byte == lock, (
        "%s: the lock byte reads 0x%02X, not 0x%02X as the write found it — the gear is left "
        "%s" % (what, lock_byte or 0, lock, "open" if lock_byte == BANK_UNLOCKED else "changed"))


def _round(api, short, sim, values, lock, op_check, what):
    year, text = values
    _write(api, short, {YEAR_FIELD: year}, op_check)
    _write(api, short, {TEXT_FIELD: text}, op_check)
    shown, shown_lock = _read(api, short, op_check)
    _assert_shown(shown, shown_lock, year, text, lock, what)
    _assert_held(sim, short, year, text, lock, what)


def _assert_wire_gates(before, after):
    assert _delta(before, after, PROGRESS) > 0, (
        "four writes ran and no transaction was recorded, so the gates below judged nothing")
    for gate in WIRE_GATES:
        assert _delta(before, after, gate) == 0, (
            "%s moved by %d across the writes (since boot: %d)"
            % (gate, _delta(before, after, gate), after.get(gate, 0)))


def _write_and_restore(api, short, sim, lock, op_check):
    sim.bank1_lock(short, lock)
    found, lock_byte = _read(api, short, op_check)
    assert found.get(YEAR_FIELD) and found.get(TEXT_FIELD), (
        "the read shows no Part 251 data on emulated format-%s gear: %r"
        % (LUMINAIRE_FORMAT, found))
    assert lock_byte == lock, "the read did not find the lock the emulator holds: %r" % lock_byte
    year, text = found[YEAR_FIELD]["raw"], found[TEXT_FIELD]
    before = _wire(api)
    written = (year + 1 if year < YEAR_LAST else year - 1, WRITTEN_TEXT)
    _round(api, short, sim, written, lock, op_check, "after the writes")
    _round(api, short, sim, (year, text), lock, op_check, "after the restore")
    _assert_wire_gates(before, _wire(api))


@pytest.mark.hil_id("HIL-LUM-01")
def test_an_open_bank_1_takes_a_number_and_a_text_and_stays_open(
        api, luminaire_gear, op_check):
    short, sim = luminaire_gear
    _write_and_restore(api, short, sim, BANK_UNLOCKED, op_check)


@pytest.mark.hil_id("HIL-LUM-02")
def test_a_locked_bank_1_is_opened_for_the_session_and_locked_again(
        api, luminaire_gear, op_check):
    short, sim = luminaire_gear
    _write_and_restore(api, short, sim, BANK_LOCKED, op_check)
