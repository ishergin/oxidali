use dali2rust_contracts::msg::PowerState;
use dali2rust_domain::dali::dev103::{instance_type, OccupancyEvent};

use super::groups::GROUP_COUNT;
use super::store::RegistryStore;

pub type RulesLampRow = (u8, u16, bool, Option<u8>, Option<u16>, Option<u8>, bool);
pub type RulesGroupRow = (u8, u16, bool, u8);
pub type RulesDeviceRow = (u8, u8, bool);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RulesInputRow {
    pub adapter_id: u8,
    pub short_address: u8,
    pub instance_number: u8,
    pub instance_type: Option<u8>,
    pub occupied: Option<bool>,
    pub light: Option<u16>,
    pub position: Option<u16>,
    pub last_event_at_ms: Option<u64>,
}

const ONLINE_WINDOW_MS: u64 = 15 * 60 * 1_000;

impl RegistryStore {
    pub fn rules_lamp_rows(&self) -> Vec<RulesLampRow> {
        let g = self.read_inner();
        let mut rows: Vec<RulesLampRow> = g
            .lamps
            .iter()
            .map(|((adapter, lamp_id), lamp)| {
                let rt = lamp
                    .binding_short
                    .and_then(|short| g.physical_devices.get(&(*adapter, short)));
                let level = rt.and_then(|pd| pd.runtime_level);
                let is_on = rt.is_some_and(|pd| pd.runtime.power == PowerState::On);
                let kelvin = rt.and_then(|pd| pd.runtime.kelvin);
                let bound = lamp.binding_short.is_some();
                (*adapter, u16::from(*lamp_id), is_on, level, kelvin, level.map(|l| l.max(1)), bound)
            })
            .collect();
        rows.sort_unstable_by_key(|r| (r.0, r.1));
        rows
    }

    pub fn rules_group_rows(&self) -> Vec<RulesGroupRow> {
        let g = self.read_inner();
        let mut rows = Vec::new();
        for adapter in 0..u8::try_from(g.adapters.len()).unwrap_or(u8::MAX) {
            for group_id in 0..GROUP_COUNT {
                let state = super::ha_publish::group_state_for_rules(&g, adapter, group_id);
                rows.push((adapter, u16::from(group_id), state.0, state.1));
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
                (*adapter, *short, online)
            })
            .collect();
        rows.sort_unstable();
        rows
    }

    pub fn rules_input_rows(&self) -> Vec<RulesInputRow> {
        let g = self.read_inner();
        let mut rows = Vec::new();
        for ((adapter, short), record) in &g.input_devices {
            for inst in &record.instances {
                let reading_of = |kind| {
                    (inst.instance_type == Some(kind)).then_some(inst.input_value).flatten()
                };
                let occupied = reading_of(instance_type::OCCUPANCY)
                    .and_then(OccupancyEvent::from_info)
                    .map(|event| event.occupied);
                rows.push(RulesInputRow {
                    adapter_id: *adapter,
                    short_address: *short,
                    instance_number: inst.instance_number,
                    instance_type: inst.instance_type,
                    occupied,
                    light: reading_of(instance_type::LIGHT_SENSOR),
                    position: reading_of(instance_type::ABSOLUTE_INPUT),
                    last_event_at_ms: inst.last_event_at_ms,
                });
            }
        }
        rows.sort_unstable();
        rows
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
            observed_at_mono_ms: 10,
        }
    }

    #[test]
    fn sensor_readings_reach_the_rules_world_by_instance_type() {
        let store = store_with(&[
            instance_type::OCCUPANCY,
            instance_type::LIGHT_SENSOR,
            instance_type::ABSOLUTE_INPUT,
        ]);
        let occupied = OccupancyEvent { movement: true, occupied: true, still: false, movement_based: true };
        assert!(store.apply_input_event(&reading(0, InputEventKind::Occupancy, occupied.to_info())));
        assert!(store.apply_input_event(&reading(1, InputEventKind::Illuminance, 300)));
        assert!(store.apply_input_event(&reading(2, InputEventKind::Position, 512)));

        let rows = store.rules_input_rows();
        assert_eq!(rows[0].occupied, Some(true));
        assert_eq!(rows[1].light, Some(300));
        assert_eq!(rows[2].position, Some(512));
        assert!(rows.iter().all(|row| row.last_event_at_ms == Some(EVENT_AT_MS)));
        assert_eq!((rows[0].light, rows[1].position, rows[2].occupied), (None, None, None));
    }
}
