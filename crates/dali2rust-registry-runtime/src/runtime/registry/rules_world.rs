use dali2rust_contracts::msg::PowerState;

use super::groups::GROUP_COUNT;
use super::store::RegistryStore;

pub type RulesLampRow = (u8, u16, bool, u8, Option<u16>, u8);
pub type RulesGroupRow = (u8, u16, bool, u8);
pub type RulesDeviceRow = (u8, u8, bool);
pub type RulesInputRow = (u8, u8, u8, Option<bool>, Option<u16>);

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
                let level = rt.and_then(|pd| pd.runtime_level).unwrap_or(0);
                let is_on = rt.is_some_and(|pd| pd.runtime.power == PowerState::On);
                let kelvin = rt.and_then(|pd| pd.runtime.kelvin);
                (*adapter, u16::from(*lamp_id), is_on, level, kelvin, level.max(1))
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
                let occupied = (inst.instance_type == Some(3))
                    .then(|| inst.input_value.map(|v| v & 0x02 != 0))
                    .flatten();
                let light = (inst.instance_type == Some(4))
                    .then_some(inst.input_value)
                    .flatten();
                rows.push((*adapter, *short, inst.instance_number, occupied, light));
            }
        }
        rows.sort_unstable();
        rows
    }
}
