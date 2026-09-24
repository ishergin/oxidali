use crate::runtime::registry::store::{evict_stale, registry_unix_ms, Staged};
use crate::runtime::registry::store::Inner;
use crate::runtime::registry::store::RegistryStore;

use dali2rust_domain::dali::banks::{
    BANK_ACTIVE_ENERGY, BANK_APPARENT_ENERGY, BANK_CONTROL_GEAR_DIAGNOSTICS,
    BANK_LIGHT_SOURCE_DIAGNOSTICS, BANK_LOADSIDE_ENERGY, BANK_LUMINAIRE_MAINTENANCE,
};
use dali2rust_domain::registry::{ConditionView, EnergyBankView};

use super::memory_bank_parser::{
    parse_bank0_bus_unit, parse_bank0_identity, parse_bank1_luminaire, parse_bank1_profile,
    parse_energy_bank, parse_gear_diagnostics, parse_luminaire_maintenance,
    parse_source_diagnostics,
};
use super::physical_devices::{
    merge_read_attr_opt, MemoryBankRangeRecord, MemoryBankRecord, PhysicalDeviceRecord,
};

#[derive(Debug)]
pub(crate) struct MemoryBankStaging {
    pub(crate) adapter_id: u8,
    pub(crate) short_address: u8,
    pub(crate) bank: u8,
    pub(crate) start_offset: u16,
    pub(crate) bytes: Vec<u8>,
    pub(crate) staged_at_ms: u64,
    last_chunk_index: Option<u16>,
}

impl MemoryBankStaging {
    fn new(
        adapter_id: u8,
        short_address: u8,
        bank: u8,
        start_offset: u16,
        data: &[u8],
        now: u64,
        chunk_index: u16,
    ) -> Self {
        Self {
            adapter_id,
            short_address,
            bank,
            start_offset,
            bytes: data.to_vec(),
            staged_at_ms: now,
            last_chunk_index: Some(chunk_index),
        }
    }

    #[allow(clippy::too_many_arguments, reason = "one param per identity field of the chunk")]
    fn accept_chunk(
        &mut self,
        adapter_id: u8,
        short_address: u8,
        bank: u8,
        start_offset: u16,
        chunk_index: u16,
        data: &[u8],
        now: u64,
    ) -> bool {
        if self.adapter_id != adapter_id
            || self.short_address != short_address
            || self.bank != bank
            || self.start_offset != start_offset
        {
            return false;
        }
        let ok_order = match self.last_chunk_index {
            None => true,
            Some(last) => last
                .checked_add(1)
                .map(|expected| chunk_index == expected)
                .unwrap_or(false),
        };
        if !ok_order {
            return false;
        }
        self.last_chunk_index = Some(chunk_index);
        self.bytes.extend_from_slice(data);
        self.staged_at_ms = now;
        true
    }
}

fn commit_staged_bank(g: &mut Inner, correlation_id: u64) -> Option<(u8, bool)> {
    let st = g.memory_bank_stage.remove(&correlation_id)?;
    let aid = st.adapter_id;
    let sa = st.short_address;
    let bank = st.bank;
    let now_commit = registry_unix_ms();
    let rec = g.physical_devices.get_mut(&(aid, sa))?;
    let changed = commit_memory_bank(rec, bank, st.start_offset, st.bytes, now_commit);
    g.physical_devices_revision = g.physical_devices_revision.saturating_add(1);
    Some((aid, changed))
}

macro_rules! merge_parsed_fields {
    ($parsed:expr, $group:expr, $now_ms:expr; $($field:ident),+ $(,)?) => {{
        let mut changed = false;
        $(changed |= merge_read_attr_opt(&mut $group.$field, $parsed.$field, $now_ms);)+
        changed
    }};
}

fn apply_bank0_identity(
    rec: &mut PhysicalDeviceRecord,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) -> bool {
    let parsed = parse_bank0_identity(bytes, start_offset);
    merge_parsed_fields!(
        parsed, rec.attributes.memory_identity, now_ms;
        last_memory_bank,
        gtin,
        firmware_version_major,
        firmware_version_minor,
        identification_number,
        hardware_version_major,
        hardware_version_minor,
        dali_101_version,
        dali_102_version,
        dali_103_version,
        logical_control_device_units,
        logical_control_gear_units,
        logical_control_gear_index,
    )
}

fn apply_bank0_bus_unit(
    rec: &mut PhysicalDeviceRecord,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) {
    let parsed = parse_bank0_bus_unit(bytes, start_offset);
    merge_parsed_fields!(
        parsed, rec.bus_unit, now_ms;
        configuration,
        implemented_parts,
    );
}

fn apply_bank1_luminaire(
    rec: &mut PhysicalDeviceRecord,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) {
    let parsed = parse_bank1_luminaire(bytes, start_offset);
    if parsed.content_format_id.is_none() && rec.luminaire_info.is_none() {
        return;
    }
    let view = rec
        .luminaire_info
        .get_or_insert_with(dali2rust_bsp::psram::PsramBox::default);
    merge_parsed_fields!(
        parsed, view, now_ms;
        content_format_id,
        year,
        week,
        nominal_input_power_w,
        power_at_minimum_w,
        nominal_min_ac_voltage_v,
        nominal_max_ac_voltage_v,
        nominal_light_output_lm,
        cri,
        cct_kelvin,
        light_distribution_type,
        luminaire_colour,
        luminaire_identification,
        light_distribution,
        oem_name,
        customer_stocking_number,
        lamp_current_ma,
        free_use,
    );
}

fn apply_bank1_profile(
    rec: &mut PhysicalDeviceRecord,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) -> bool {
    let parsed = parse_bank1_profile(bytes, start_offset);
    merge_parsed_fields!(
        parsed, rec.attributes.memory_profile, now_ms;
        bank1_lock_byte,
        oem_gtin,
        oem_identification_number,
    )
}

fn update_existing_bank_entry(
    entry: &mut MemoryBankRecord,
    total: u16,
    ranges: Vec<MemoryBankRangeRecord>,
    bytes: Vec<u8>,
    now_ms: u64,
) -> bool {
    let changed = entry.total_bytes_read != total || entry.ranges != ranges;
    entry.total_bytes_read = total;
    entry.last_read_ms = now_ms;
    entry.ranges = ranges;
    entry.bytes = bytes;
    changed
}

fn commit_memory_bank(
    rec: &mut PhysicalDeviceRecord,
    bank: u8,
    start_offset: u16,
    bytes: Vec<u8>,
    now_ms: u64,
) -> bool {
    let total = bytes.len().min(usize::from(u16::MAX)) as u16;
    let ranges = vec![MemoryBankRangeRecord {
        start: start_offset,
        length: total,
    }];
    let mut changed = match rec.memory_banks.iter_mut().find(|entry| entry.bank == bank) {
        Some(existing) => update_existing_bank_entry(existing, total, ranges, bytes.clone(), now_ms),
        None => {
            rec.memory_banks.push(MemoryBankRecord {
                bank,
                total_bytes_read: total,
                last_read_ms: now_ms,
                ranges,
                bytes: bytes.clone(),
            });
            true
        }
    };
    changed |= apply_parsed_bank(rec, bank, &bytes, start_offset, now_ms);
    changed
}

fn apply_parsed_bank(
    rec: &mut PhysicalDeviceRecord,
    bank: u8,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) -> bool {
    match bank {
        0 => apply_bank0_identity(rec, bytes, start_offset, now_ms) | {
            apply_bank0_bus_unit(rec, bytes, start_offset, now_ms);
            false
        },
        1 => apply_bank1_profile(rec, bytes, start_offset, now_ms) | {
            apply_bank1_luminaire(rec, bytes, start_offset, now_ms);
            false
        },
        BANK_ACTIVE_ENERGY => apply_energy_bank(&mut rec.energy.active, bytes, start_offset, now_ms),
        BANK_APPARENT_ENERGY => {
            apply_energy_bank(&mut rec.energy.apparent, bytes, start_offset, now_ms)
        }
        BANK_LOADSIDE_ENERGY => {
            apply_energy_bank(&mut rec.energy.loadside, bytes, start_offset, now_ms)
        }
        BANK_CONTROL_GEAR_DIAGNOSTICS => apply_gear_diagnostics(rec, bytes, start_offset, now_ms),
        BANK_LIGHT_SOURCE_DIAGNOSTICS => apply_source_diagnostics(rec, bytes, start_offset, now_ms),
        BANK_LUMINAIRE_MAINTENANCE => apply_luminaire(rec, bytes, start_offset, now_ms),
        _ => false,
    }
}

fn apply_energy_bank(
    view: &mut EnergyBankView,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) -> bool {
    let parsed = parse_energy_bank(bytes, start_offset);
    merge_parsed_fields!(
        parsed, view, now_ms;
        bank_version,
        energy_scale,
        energy,
        power_scale,
        power,
    );
    false
}

fn apply_condition(
    view: &mut ConditionView,
    parsed: super::memory_bank_parser::ParsedCondition,
    now_ms: u64,
) {
    merge_read_attr_opt(&mut view.flag, parsed.flag, now_ms);
    merge_read_attr_opt(&mut view.counter, parsed.counter, now_ms);
}

fn apply_gear_diagnostics(
    rec: &mut PhysicalDeviceRecord,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) -> bool {
    let parsed = parse_gear_diagnostics(bytes, start_offset);
    let view = &mut rec.diagnostics.control_gear;
    merge_parsed_fields!(
        parsed, view, now_ms;
        bank_version,
        operating_time_s,
        start_counter,
        supply_voltage_decivolt,
        supply_frequency_hz,
        power_factor_centi,
        temperature_offset60,
        output_current_percent,
    );
    apply_condition(&mut view.overall_failure, parsed.overall_failure, now_ms);
    apply_condition(&mut view.undervoltage, parsed.undervoltage, now_ms);
    apply_condition(&mut view.overvoltage, parsed.overvoltage, now_ms);
    apply_condition(
        &mut view.output_power_limitation,
        parsed.output_power_limitation,
        now_ms,
    );
    apply_condition(&mut view.thermal_derating, parsed.thermal_derating, now_ms);
    apply_condition(&mut view.thermal_shutdown, parsed.thermal_shutdown, now_ms);
    false
}

fn apply_source_diagnostics(
    rec: &mut PhysicalDeviceRecord,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) -> bool {
    let parsed = parse_source_diagnostics(bytes, start_offset);
    let view = &mut rec.diagnostics.light_source;
    merge_parsed_fields!(
        parsed, view, now_ms;
        bank_version,
        start_counter_resettable,
        start_counter,
        on_time_resettable_s,
        on_time_s,
        voltage_decivolt,
        current_milliamp,
        temperature_offset60,
    );
    apply_condition(&mut view.overall_failure, parsed.overall_failure, now_ms);
    apply_condition(&mut view.short_circuit, parsed.short_circuit, now_ms);
    apply_condition(&mut view.open_circuit, parsed.open_circuit, now_ms);
    apply_condition(&mut view.thermal_derating, parsed.thermal_derating, now_ms);
    apply_condition(&mut view.thermal_shutdown, parsed.thermal_shutdown, now_ms);
    false
}

fn apply_luminaire(
    rec: &mut PhysicalDeviceRecord,
    bytes: &[u8],
    start_offset: u16,
    now_ms: u64,
) -> bool {
    let parsed = parse_luminaire_maintenance(bytes, start_offset);
    let view = &mut rec.diagnostics.luminaire;
    merge_parsed_fields!(
        parsed, view, now_ms;
        bank_version,
        rated_life_kilohours,
        reference_temperature_offset60,
        rated_starts_hundreds,
    );
    false
}

impl RegistryStore {
    #[allow(clippy::too_many_arguments, reason = "one param per field of the chunk event")]
    pub(crate) fn stage_memory_bank_chunk(
        &self,
        correlation_id: u64,
        adapter_id: u8,
        short_address: u8,
        bank: u8,
        start_offset: u16,
        chunk_index: u16,
        last_chunk: bool,
        data: &[u8],
    ) -> bool {
        let now = registry_unix_ms();
        let mut g = self.write_inner();

        if let Some(st) = g.memory_bank_stage.get_mut(&correlation_id) {
            if !st.accept_chunk(
                adapter_id,
                short_address,
                bank,
                start_offset,
                chunk_index,
                data,
                now,
            ) {
                g.memory_bank_stage.remove(&correlation_id);
                return false;
            }
        } else {
            g.memory_bank_stage.insert(
                correlation_id,
                MemoryBankStaging::new(
                    adapter_id,
                    short_address,
                    bank,
                    start_offset,
                    data,
                    now,
                    chunk_index,
                ),
            );
        }

        if !last_chunk {
            return false;
        }

        let committed = commit_staged_bank(&mut g, correlation_id);
        drop(g);
        if let Some((aid, true)) = committed {
            self.dirty.mark_physical_devices_dirty(aid);
        }
        committed.is_some()
    }

    pub(crate) fn abort_memory_bank_stage(&self, correlation_id: u64) {
        let mut g = self.write_inner();
        g.memory_bank_stage.remove(&correlation_id);
    }

    pub(crate) fn evict_stale_memory_bank_staging(&self, max_age_ms: u64) {
        let now = registry_unix_ms();
        evict_stale(&mut self.write_inner().memory_bank_stage, now, max_age_ms);
    }
}

impl Staged for MemoryBankStaging {
    fn staged_at_ms(&self) -> u64 {
        self.staged_at_ms
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::registry::store::RegistryStore;

    const SHORT: u8 = 5;

    fn seeded_store() -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        assert!(store.seed_discovered_dt8(SHORT, Some(0x2222)));
        let _ = store.dirty.take_physical_devices_dirty_for(0);
        store
    }

    #[test]
    fn repeat_memory_bank_read_does_not_redirty() {
        let store = seeded_store();
        let bytes: Vec<u8> = (0u8..27).collect();

        assert!(store.stage_memory_bank_chunk(11, 0, SHORT, 0, 0, 0, true, &bytes));
        assert!(store.dirty.take_physical_devices_dirty_for(0));

        assert!(store.stage_memory_bank_chunk(12, 0, SHORT, 0, 0, 0, true, &bytes));
        assert!(
            !store.dirty.take_physical_devices_dirty_for(0),
            "an identical repeat bank read must not rewrite flash"
        );

        let longer: Vec<u8> = (0u8..30).collect();
        assert!(store.stage_memory_bank_chunk(13, 0, SHORT, 0, 0, 0, true, &longer));
        assert!(store.dirty.take_physical_devices_dirty_for(0));
    }
}
