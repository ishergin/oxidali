use dali2rust_contracts::msg::FixedText64;

use super::limits::{MAX_ADAPTERS, MAX_SHORT_ADDRESSES};

pub const CLUSTER_INPUT_DEVICES: usize = 32;

pub const CLUSTER_INPUT_INSTANCES: usize = 32;

pub const INPUT_INSTANCE_GROUPS: usize = 3;

const ADAPTER_SLOTS: usize = MAX_ADAPTERS as usize;

const SHORT_ADDRESSES: u8 = MAX_SHORT_ADDRESSES as u8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputDeviceKeys {
    pub addressed: [u64; ADAPTER_SLOTS],
}

impl InputDeviceKeys {
    pub fn insert(&mut self, adapter: u8, short_address: u8) {
        if let Some(mask) = self.addressed.get_mut(usize::from(adapter)) {
            *mask |= 1_u64.checked_shl(u32::from(short_address)).unwrap_or(0);
        }
    }

    pub fn iter(self) -> impl Iterator<Item = (u8, u8)> {
        (0..MAX_ADAPTERS).flat_map(move |adapter| {
            let mask = self.addressed.get(usize::from(adapter)).copied().unwrap_or(0);
            (0..SHORT_ADDRESSES).filter(move |short| mask & (1 << short) != 0).map(move |short| (adapter, short))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDeviceView {
    pub adapter_id: u8,
    pub short_address: u8,
    pub name: FixedText64,
    pub ha_expose: bool,
    pub present: bool,
    pub instance_count: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InputInstanceView {
    pub instance_number: u8,
    pub instance_type: Option<u8>,
    pub groups: [Option<u8>; INPUT_INSTANCE_GROUPS],
    pub last_event_info: Option<u16>,
    pub input_value: Option<u16>,
    pub last_event_mono_ms: Option<u32>,
}

impl InputInstanceView {
    #[must_use]
    pub const fn without_value(self) -> Self {
        Self { last_event_info: None, input_value: None, last_event_mono_ms: None, ..self }
    }
}

pub trait ClusterInputReadPort: Send + Sync {
    fn input_device_keys(&self) -> InputDeviceKeys;

    fn input_device_view(&self, adapter_id: u8, short_address: u8) -> Option<InputDeviceView>;

    fn input_instance_view(&self, adapter_id: u8, short_address: u8, index: usize) -> Option<InputInstanceView>;

    fn input_instance_groups(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> [Option<u8>; INPUT_INSTANCE_GROUPS];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_come_out_by_adapter_then_by_short_address() {
        let mut keys = InputDeviceKeys::default();
        for (adapter, short) in [(7, 0), (0, 63), (0, 3), (2, 9)] {
            keys.insert(adapter, short);
        }
        assert_eq!(keys.iter().collect::<Vec<_>>(), [(0, 3), (0, 63), (2, 9), (7, 0)]);
    }

    #[test]
    fn a_key_past_the_last_adapter_or_short_address_is_not_kept() {
        let mut keys = InputDeviceKeys::default();
        keys.insert(MAX_ADAPTERS, 0);
        keys.insert(0, SHORT_ADDRESSES);
        assert_eq!(keys.iter().count(), 0);
    }

    #[test]
    fn a_view_without_its_value_keeps_what_the_device_is() {
        let view = InputInstanceView {
            instance_number: 2,
            instance_type: Some(3),
            groups: [Some(1), None, Some(31)],
            last_event_info: Some(7),
            input_value: Some(2),
            last_event_mono_ms: Some(9),
        };
        let bare = InputInstanceView { last_event_info: None, input_value: None, last_event_mono_ms: None, ..view };
        assert_eq!(view.without_value(), bare);
    }
}
