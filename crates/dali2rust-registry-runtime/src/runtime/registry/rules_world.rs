use dali2rust_contracts::msg::PowerState;
use dali2rust_domain::dali::dev103::{instance_type, OccupancyEvent};

use super::groups::GROUP_COUNT;
use super::input_devices::InstanceRecord;
use super::store::{Inner, RegistryStore};
use super::virtual_lamps::VlRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RulesLampRow {
    pub adapter_id: u8,
    pub id: u16,
    pub is_on: bool,
    pub level: Option<u8>,
    pub cct_kelvin: Option<u16>,
    pub last_level: Option<u8>,
    pub bound: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RulesGroupRow {
    pub adapter_id: u8,
    pub id: u16,
    pub any_on: bool,
    pub member_count: u8,
}

impl RulesGroupRow {
    #[must_use]
    pub const fn all_off(&self) -> bool {
        !self.any_on
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RulesDeviceRow {
    pub adapter_id: u8,
    pub short_address: u8,
    pub online: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RulesInputRow {
    pub adapter_id: u8,
    pub short_address: u8,
    pub instance_number: u8,
    pub instance_type: Option<u8>,
    pub occupied: Option<bool>,
    pub light: Option<u16>,
    pub position: Option<u16>,
    pub last_event_age_ms: Option<u32>,
}

const ONLINE_WINDOW_MS: u64 = 15 * 60 * 1_000;

const STAMP_HALF_RANGE: u32 = u32::MAX / 2;

fn age_since(now_mono_ms: u32, at_mono_ms: u32) -> u32 {
    let age = now_mono_ms.wrapping_sub(at_mono_ms);
    if age > STAMP_HALF_RANGE {
        0
    } else {
        age
    }
}

fn lamp_row(g: &Inner, adapter_id: u8, lamp_id: u8, lamp: &VlRecord) -> RulesLampRow {
    let rt = lamp
        .binding_short
        .and_then(|short| g.physical_devices.get(&(adapter_id, short)));
    let level = rt.and_then(|pd| pd.runtime_level);
    RulesLampRow {
        adapter_id,
        id: u16::from(lamp_id),
        is_on: rt.is_some_and(|pd| pd.runtime.power == PowerState::On),
        level,
        cct_kelvin: rt.and_then(|pd| pd.runtime.kelvin),
        last_level: level.map(|l| l.max(1)),
        bound: lamp.binding_short.is_some(),
    }
}

fn input_row(
    adapter_id: u8,
    short_address: u8,
    inst: &InstanceRecord,
    now_mono_ms: u32,
) -> RulesInputRow {
    let reading_of =
        |kind| (inst.instance_type == Some(kind)).then_some(inst.input_value).flatten();
    RulesInputRow {
        adapter_id,
        short_address,
        instance_number: inst.instance_number,
        instance_type: inst.instance_type,
        occupied: reading_of(instance_type::OCCUPANCY)
            .and_then(OccupancyEvent::from_info)
            .map(|event| event.occupied),
        light: reading_of(instance_type::LIGHT_SENSOR),
        position: reading_of(instance_type::ABSOLUTE_INPUT),
        last_event_age_ms: inst.last_event_mono_ms.map(|at| age_since(now_mono_ms, at)),
    }
}

impl RegistryStore {
    pub fn rules_lamp_rows(&self) -> Vec<RulesLampRow> {
        let g = self.read_inner();
        let mut rows: Vec<RulesLampRow> = g
            .lamps
            .iter()
            .map(|((adapter, lamp_id), lamp)| lamp_row(&g, *adapter, *lamp_id, lamp))
            .collect();
        rows.sort_unstable_by_key(|r| (r.adapter_id, r.id));
        rows
    }

    pub fn rules_group_rows(&self) -> Vec<RulesGroupRow> {
        let g = self.read_inner();
        let mut rows = Vec::new();
        for adapter in 0..u8::try_from(g.adapters.len()).unwrap_or(u8::MAX) {
            for group_id in 0..GROUP_COUNT {
                let (any_on, member_count) =
                    super::ha_publish::group_state_for_rules(&g, adapter, group_id);
                rows.push(RulesGroupRow {
                    adapter_id: adapter,
                    id: u16::from(group_id),
                    any_on,
                    member_count,
                });
            }
        }
        rows
    }

    pub fn rules_device_rows(&self, now_ms: u64) -> Vec<RulesDeviceRow> {
        let g = self.read_inner();
        let mut rows: Vec<RulesDeviceRow> = g
            .physical_devices
            .iter()
            .map(|((adapter, short), pd)| {
                let online = pd
                    .runtime
                    .last_seen_ms
                    .is_some_and(|seen| now_ms.saturating_sub(seen) < ONLINE_WINDOW_MS);
                RulesDeviceRow { adapter_id: *adapter, short_address: *short, online }
            })
            .collect();
        rows.sort_unstable();
        rows
    }

    pub fn rules_input_rows(&self, now_mono_ms: u32) -> Vec<RulesInputRow> {
        let g = self.read_inner();
        let mut rows = Vec::new();
        for ((adapter, short), record) in &g.input_devices {
            rows.extend(
                record.instances.iter().map(|inst| input_row(*adapter, *short, inst, now_mono_ms)),
            );
        }
        rows.sort_unstable();
        rows
    }

    pub fn input_instance_groups(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> [Option<u8>; 3] {
        let g = self.read_inner();
        g.input_devices
            .get(&(adapter_id, short_address))
            .and_then(|record| record.instances.iter().find(|i| i.instance_number == instance_number))
            .map_or([None; 3], |inst| inst.instance_groups.map(|group| group.value.flatten()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::registry::input_devices::InstanceReadback;
    use dali2rust_contracts::msg::{
        Dali103ScanProgressEvent, DaliInputEventObservedEvent, InputEventKind,
    };

    const SHORT: u8 = 3;
    const EVENT_AT_MS: u64 = 1_790_000_000_000;
    const EVENT_AT_MONO_MS: u32 = 4_200;
    const AGE_MS: u32 = 800;
    const NO_GROUP: Option<Option<u8>> = Some(None);

    fn store_with(types: &[u8]) -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        assert!(store.apply_input_scan_progress(
            &Dali103ScanProgressEvent {
                registry_adapter_id: 0,
                short_address: SHORT,
                presence_unproven: false,
                instance_count: u8::try_from(types.len()).unwrap(),
                device_capabilities: None,
                device_status: None,
                version_number: None,
            },
            500,
        ));
        for (number, kind) in (0u8..).zip(types) {
            let readback = InstanceReadback { instance_type: Some(*kind), ..InstanceReadback::default() };
            assert!(store.apply_instance_readback(0, SHORT, number, &readback, 500));
        }
        store
    }

    fn occupied_info() -> u16 {
        OccupancyEvent { movement: true, occupied: true, still: false, movement_based: true }.to_info()
    }

    fn reading(instance: u8, typed: InputEventKind, value: u16) -> DaliInputEventObservedEvent {
        DaliInputEventObservedEvent {
            registry_adapter_id: 0,
            scheme: 2,
            short_address: Some(SHORT),
            device_group: None,
            instance_group: None,
            instance_number: Some(instance),
            instance_type: None,
            event_info: value,
            typed,
            typed_value: value,
            observed_at_ms: EVENT_AT_MS,
            observed_at_mono_ms: EVENT_AT_MONO_MS,
        }
    }

    #[test]
    fn sensor_readings_reach_the_rules_world_by_instance_type() {
        let store = store_with(&[
            instance_type::OCCUPANCY,
            instance_type::LIGHT_SENSOR,
            instance_type::ABSOLUTE_INPUT,
        ]);
        assert!(store.apply_input_event(&reading(0, InputEventKind::Occupancy, occupied_info())));
        assert!(store.apply_input_event(&reading(1, InputEventKind::Illuminance, 300)));
        assert!(store.apply_input_event(&reading(2, InputEventKind::Position, 512)));

        let rows = store.rules_input_rows(EVENT_AT_MONO_MS + AGE_MS);
        assert_eq!(rows[0].occupied, Some(true));
        assert_eq!(rows[1].light, Some(300));
        assert_eq!(rows[2].position, Some(512));
        assert!(
            rows.iter().all(|row| row.last_event_age_ms == Some(AGE_MS)),
            "the event's age is taken on the monotonic clock, which a wall-clock step cannot move"
        );
        assert_eq!((rows[0].light, rows[1].position, rows[2].occupied), (None, None, None));
    }

    #[test]
    fn an_event_stamped_after_the_clock_was_read_is_new_not_49_days_old() {
        assert_eq!(age_since(1_000, 1_500), 0);
        assert_eq!(age_since(1_500, 1_000), 500);
        assert_eq!(age_since(10, u32::MAX - 9), 20, "the stamp wraps with the clock");
    }

    #[test]
    fn an_input_row_ages_by_the_clock_it_is_given_across_the_wrap() {
        let store = store_with(&[instance_type::OCCUPANCY]);
        let mut event = reading(0, InputEventKind::Occupancy, occupied_info());
        event.observed_at_mono_ms = u32::MAX - 9;
        assert!(store.apply_input_event(&event));
        assert_eq!(store.rules_input_rows(10)[0].last_event_age_ms, Some(20));
        assert_eq!(store.rules_input_rows(u32::MAX - 20)[0].last_event_age_ms, Some(0));
    }

    #[test]
    fn an_instance_never_heard_from_has_no_age() {
        let store = store_with(&[instance_type::OCCUPANCY]);
        assert_eq!(store.rules_input_rows(EVENT_AT_MONO_MS)[0].last_event_age_ms, None);
    }

    fn store_with_groups(readback_groups: [Option<Option<u8>>; 3]) -> RegistryStore {
        let store = store_with(&[instance_type::OCCUPANCY, instance_type::LIGHT_SENSOR]);
        let readback = InstanceReadback { instance_groups: readback_groups, ..InstanceReadback::default() };
        assert!(store.apply_instance_readback(0, SHORT, 1, &readback, 600));
        store
    }

    #[test]
    fn an_instance_reads_the_groups_it_was_read_back_into() {
        let store = store_with_groups([Some(Some(5)), NO_GROUP, Some(Some(31))]);
        assert_eq!(store.input_instance_groups(0, SHORT, 1), [Some(5), None, Some(31)]);
    }

    #[test]
    fn an_instance_not_read_back_has_no_groups() {
        let store = store_with_groups([Some(Some(5)), None, None]);
        assert_eq!(store.input_instance_groups(0, SHORT, 0), [None; 3]);
        assert_eq!(store.input_instance_groups(0, SHORT, 1), [Some(5), None, None]);
    }

    #[test]
    fn a_device_or_instance_the_registry_does_not_hold_has_no_groups() {
        let store = store_with_groups([Some(Some(5)); 3]);
        assert_eq!(store.input_instance_groups(0, SHORT + 1, 1), [None; 3]);
        assert_eq!(store.input_instance_groups(0, SHORT, 7), [None; 3]);
        assert_eq!(store.input_instance_groups(1, SHORT, 1), [None; 3]);
    }
}
