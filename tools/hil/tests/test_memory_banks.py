import pytest




MASK = {1: 0xFF, 2: 0xFFFF, 3: 0xFFFFFF, 4: 0xFFFFFFFF, 6: 0xFFFFFFFFFFFF}
TMASK = {width: value - 1 for width, value in MASK.items()}

BANK_LAST_OFFSET = {202: 0x0F, 203: 0x0F, 204: 0x0F, 205: 0x1C, 206: 0x20, 207: 0x07}

ENERGY_BANKS = (202, 203, 204)
DIAGNOSTIC_BANKS = (205, 206)

DEVICE_TYPE_ENERGY = 51
DEVICE_TYPE_DIAGNOSTICS = 52


def _declared_types(api, short):
    return api.state(short).get("supported_device_types")


def _gear_with_type(api, device_type):
    for short in api.addrs():
        declared = _declared_types(api, short)
        if declared and device_type in declared:
            return short
    return None


@pytest.fixture(scope="session")
def energy_gear(api):
    short = _gear_with_type(api, DEVICE_TYPE_ENERGY)
    if short is None:
        pytest.skip(
            "no gear on this segment declares device type 51 (Part 252 energy "
            "reporting); banks 202-204 are legitimately absent"
        )
    return short


@pytest.fixture(scope="session")
def diagnostics_gear(api):
    short = _gear_with_type(api, DEVICE_TYPE_DIAGNOSTICS)
    if short is None:
        pytest.skip(
            "no gear on this segment declares device type 52 (Part 253 "
            "diagnostics); banks 205-207 are legitimately absent"
        )
    return short


def _bank_summary(api, short, bank):
    for entry in api.memory_banks(short).get("memory_banks", []):
        if entry["bank"] == bank:
            return entry
    return None


def _reading(section, key):
    observed = section.get(key)
    return observed["value"] if observed else None


@pytest.mark.slow
def test_energy_banks_read_within_their_declared_length(api, energy_gear, paced, op_check):
    op_check(api.attr_read_checked(energy_gear, groups="runtime_status",
                                   banks="energy"))
    read_any = False
    for bank in ENERGY_BANKS:
        summary = _bank_summary(api, energy_gear, bank)
        if summary is None:
            continue
        read_any = True
        assert summary["total_bytes_read"] > 0
        for rng in summary["ranges"]:
            last = rng["start"] + rng["length"] - 1
            assert last <= BANK_LAST_OFFSET[bank], (
                "bank %d range %d..%d runs past the bank's last addressable "
                "location %#04x" % (bank, rng["start"], last, BANK_LAST_OFFSET[bank])
            )
    assert read_any, (
        "gear %d declares device type 51 and answered no energy bank at all"
        % energy_gear
    )


@pytest.mark.slow
def test_energy_readings_are_values_or_named_sentinels_never_both(
    api, energy_gear, paced, op_check
):
    op_check(api.attr_read_checked(energy_gear, groups="runtime_status",
                                   banks="energy"))
    section = api.attributes(energy_gear, ["memory_energy"]) \
                 .get("attributes", {}).get("memory_energy")
    if not section:
        pytest.skip("gear %d declares type 51 but reported no energy section"
                    % energy_gear)

    checked = 0
    for key, observed in section.items():
        reading = observed.get("value") if isinstance(observed, dict) else None
        if not isinstance(reading, dict) or "value" not in reading:
            continue
        checked += 1
        flags = (reading.get("not_implemented"), reading.get("temporarily_unavailable"))
        assert not all(flags), "%s is both MASK and TMASK" % key
        if any(flags):
            assert reading["value"] is None, (
                "%s carries a sentinel AND a number: %r" % (key, reading)
            )
        value = reading["value"]
        if value is not None:
            assert value not in MASK.values(), (
                "%s reported the MASK sentinel as a value: %r" % (key, value)
            )
            assert value not in TMASK.values(), (
                "%s reported the TMASK sentinel as a value: %r" % (key, value)
            )
    assert checked, "the energy section carried no bank readings at all"


@pytest.mark.slow
def test_energy_scale_factors_are_signed_and_in_range(api, energy_gear, paced, op_check):
    op_check(api.attr_read_checked(energy_gear, groups="runtime_status",
                                   banks="energy"))
    section = api.attributes(energy_gear, ["memory_energy"]) \
                 .get("attributes", {}).get("memory_energy")
    if not section:
        pytest.skip("no energy section")
    scales = {k: v for k, v in section.items() if "scale" in k}
    if not scales:
        pytest.skip("this gear reported no scale factors")
    for key, observed in scales.items():
        value = observed.get("value") if isinstance(observed, dict) else None
        if isinstance(value, dict):
            value = value.get("value")
        if value is None:
            continue
        assert -6 <= value <= 6, (
            "%s = %r is outside the [-6..+6] the bank tables allow; a value of "
            "253 here is 0xFD read unsigned" % (key, value)
        )


@pytest.mark.slow
def test_diagnostic_banks_declare_the_lengths_the_standard_gives_them(
    api, diagnostics_gear, paced, op_check
):
    op_check(api.attr_read_checked(diagnostics_gear, groups="runtime_status",
                                   banks="diagnostics"))
    seen = []
    for bank in DIAGNOSTIC_BANKS:
        summary = _bank_summary(api, diagnostics_gear, bank)
        if summary is None:
            continue
        seen.append(bank)
        for rng in summary["ranges"]:
            last = rng["start"] + rng["length"] - 1
            assert last <= BANK_LAST_OFFSET[bank], (
                "bank %d read to %#04x, past its declared last location %#04x"
                % (bank, last, BANK_LAST_OFFSET[bank])
            )
    assert seen, (
        "gear %d declares device type 52 and answered neither bank 205 nor 206"
        % diagnostics_gear
    )


@pytest.mark.slow
def test_a_condition_counter_never_exceeds_its_saturation_ceiling(
    api, diagnostics_gear, paced, op_check
):
    op_check(api.attr_read_checked(diagnostics_gear, groups="runtime_status",
                                   banks="diagnostics"))
    attrs = api.attributes(diagnostics_gear, ["memory_diagnostics"]) \
               .get("attributes", {}).get("memory_diagnostics")
    if not attrs:
        pytest.skip("no diagnostics section")
    counters = 0
    for key, observed in attrs.items():
        if "counter" not in key:
            continue
        reading = observed.get("value") if isinstance(observed, dict) else None
        if not isinstance(reading, dict):
            continue
        value = reading.get("value")
        if value is None:
            continue
        counters += 1
        assert value <= MASK[1] - 2, (
            "%s = %d is above the §9.2.8 ceiling of MASK-2; a counter cannot "
            "legally be there, so this is a sentinel read as a number"
            % (key, value)
        )
    if not counters:
        pytest.skip("this gear reported no condition counters")


@pytest.mark.smoke
def test_bank_0_extension_reports_a_classified_configuration(api, paced, op_check):
    candidates = api.present_optical_addrs() or api.present_addrs()
    if not candidates:
        pytest.skip("no devices in the registry")

    found = None
    for short in candidates:
        op_check(api.attr_read_checked(short, groups="runtime_status", banks="identity"))
        section = api.attributes(short, ["memory_bus_unit"]) \
                     .get("attributes", {}).get("memory_bus_unit")
        if section:
            found = (short, section)
            break
    if found is None:
        pytest.skip(
            "no gear answered bank 0 above 0x1A; IEC 62386-102:2014 Table 9 "
            "makes [0x1B,0x7F] reserved and answering NO, so this is conforming"
        )

    short, section = found
    config = section.get("configuration")
    if config:
        value = config["value"]
        assert isinstance(value, dict), (
            "configuration must arrive classified ({raw, class}), not as a bare "
            "number — the UI must not have to own 098bp Table 5: %r" % (value,)
        )
        assert 0 <= value["raw"] <= 255
        assert value["class"], "every byte gets a label"
        if 9 <= value["raw"] <= 12:
            assert value.get("emergency_type") in ("A", "B", "C", "D"), (
                "Table 5 rows 9-12 are the Part 202 emergency types"
            )
        else:
            assert "emergency_type" not in value

    parts = section.get("implemented_parts")
    if parts:
        value = parts["value"]
        assert value["bytes"] in (1, 2), (
            "only 0x1C and 0x1D carry this mask: %r" % (value,)
        )
        assert value["raw"] < (1 << (8 * value["bytes"])), (
            "the mask carries bits outside the bytes that answered: %r" % (value,)
        )


@pytest.mark.slow
def test_luminaire_extension_is_gated_by_its_content_format_id(api, gear_sim, paced, op_check):
    PART_251_FIELDS = (
        "year", "week", "nominal_input_power_w", "power_at_minimum_w",
        "nominal_min_ac_voltage_v", "nominal_max_ac_voltage_v",
        "nominal_light_output_lm", "cri", "cct_kelvin",
        "light_distribution_type", "luminaire_colour", "luminaire_identification",
        "light_distribution", "oem_name", "customer_stocking_number",
        "lamp_current_ma", "free_use",
    )

    recognised = 0
    for short in api.addrs():
        section = api.attributes(short, ["memory_luminaire"]) \
                     .get("attributes", {}).get("memory_luminaire")
        if not section:
            continue
        observed = section.get("content_format_id")
        assert observed, (
            "device %d carries a luminaire section with no content format ID; "
            "the ID is the only thing that makes those bytes Part 251 rather "
            "than 102 Table 10's manufacturer-specific space" % short
        )
        fmt = observed["value"]
        if fmt in (3, 4, 5):
            recognised += 1
            continue

        leaked = [f for f in PART_251_FIELDS if section.get(f) is not None]
        assert not leaked, (
            "device %d reports content format %r, which no published layout "
            "claims, yet carries Part 251 fields %r — 102 Table 10 makes those "
            "bytes manufacturer-specific, so parsing them is inventing asset "
            "data out of vendor bytes" % (short, fmt, leaked)
        )

    if not recognised:
        pytest.skip(
            "no gear on the segment declared a Part 251 content format (the "
            "bench luminaires stop bank 1 at 0x1F and answer MASK at 0x11), so "
            "the string regions and formats 4/5 went unexercised; that needs "
            "the C6 fleet configured with format 3/4/5 gear"
        )


@pytest.mark.slow
def test_a_part_251_reading_keeps_its_raw_byte_when_the_value_is_out_of_range(
    api, gear_sim, paced, op_check
):
    seen = 0
    for short in api.addrs():
        section = api.attributes(short, ["memory_luminaire"]) \
                     .get("attributes", {}).get("memory_luminaire")
        if not section:
            continue
        for key, observed in section.items():
            value = observed.get("value") if isinstance(observed, dict) else None
            if not isinstance(value, dict) or "raw" not in value:
                continue
            seen += 1
            assert value["raw"] is not None, "%s lost its raw byte" % key
            if key == "cct_kelvin" and value.get("part209_implemented"):
                assert value["value"] is None, (
                    "a tunable luminaire has no single CCT to report"
                )
                assert value["raw"] == 0xFFFE, (
                    "part209_implemented is exactly MASK-1: %r" % (value,)
                )
    if not seen:
        pytest.skip("no Part 251 numeric fields on the segment")
