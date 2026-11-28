use dali2rust_contracts::msg::fixed_text_64;
use dali2rust_domain::registry::{
    ClusterInputReadPort, InputDeviceKeys, InputDeviceView, InputInstanceView, INPUT_INSTANCE_GROUPS,
};
use dali2rust_platform::liveness::in_reach;

use super::input_devices::{InputDeviceRecord, InstanceRecord};
use super::store::RegistryStore;

impl RegistryStore {
    pub(crate) fn hold_input_stamps_in_reach(&self, now_ms: u32) {
        let mut g = self.write_inner();
        let instances = g.input_devices.values_mut().flat_map(|record| record.instances.iter_mut());
        for instance in instances {
            instance.last_event_mono_ms = instance.last_event_mono_ms.map(|at_ms| in_reach(now_ms, at_ms));
        }
    }
}

fn device_view(record: &InputDeviceRecord) -> InputDeviceView {
    InputDeviceView {
        adapter_id: record.adapter_id,
        short_address: record.short_address,
        name: fixed_text_64(record.name.as_deref().unwrap_or_default()),
        ha_expose: record.ha_expose,
        present: record.present,
        instance_count: record.instance_count,
    }
}

fn instance_view(instance: &InstanceRecord) -> InputInstanceView {
    InputInstanceView {
        instance_number: instance.instance_number,
        instance_type: instance.instance_type,
        groups: instance.instance_groups.map(|group| group.value.flatten()),
        last_event_info: instance.last_event_info,
        input_value: instance.input_value,
        last_event_mono_ms: instance.last_event_mono_ms,
    }
}

impl ClusterInputReadPort for RegistryStore {
    fn input_device_keys(&self) -> InputDeviceKeys {
        let mut keys = InputDeviceKeys::default();
        for (adapter, short) in self.read_inner().input_devices.keys() {
            keys.insert(*adapter, *short);
        }
        keys
    }

    fn input_device_view(&self, adapter_id: u8, short_address: u8) -> Option<InputDeviceView> {
        self.read_inner().input_devices.get(&(adapter_id, short_address)).map(device_view)
    }

    fn input_instance_view(&self, adapter_id: u8, short_address: u8, index: usize) -> Option<InputInstanceView> {
        let g = self.read_inner();
        g.input_devices.get(&(adapter_id, short_address))?.instances.get(index).map(instance_view)
    }

    fn input_instance_groups(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> [Option<u8>; INPUT_INSTANCE_GROUPS] {
        RegistryStore::input_instance_groups(self, adapter_id, short_address, instance_number)
    }
}

#[cfg(test)]
mod tests {
    use dali2rust_platform::liveness::{elapsed_ms, AGE_CAP_MS};

    use super::super::input_devices::ReadValue;
    use super::*;

    fn store_with_device() -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        let mut record = InputDeviceRecord::empty(0, 5);
        record.name = Some("hall panel".to_string());
        record.ha_expose = false;
        record.present = true;
        record.instance_count = 2;
        for number in [1, 0] {
            let instance = record.instance_mut_or_declare(number).expect("declared");
            instance.instance_type = Some(1);
        }
        let first = record.instance_mut(1).expect("instance 1");
        first.instance_groups[2] = ReadValue::observed(Some(9), 1);
        first.last_event_info = Some(1);
        first.last_event_mono_ms = Some(400);
        store.write_inner().input_devices.insert((0, 5), record);
        store
    }

    #[test]
    fn the_port_lists_the_devices_and_reads_each_instance_in_its_place() {
        let store = store_with_device();
        assert_eq!(store.input_device_keys().iter().collect::<Vec<_>>(), [(0, 5)]);
        let device = store.input_device_view(0, 5).expect("device");
        assert_eq!((device.name.as_str(), device.ha_expose, device.present, device.instance_count), ("hall panel", false, true, 2));
        let first = store.input_instance_view(0, 5, 0).expect("first instance");
        let expected = InputInstanceView {
            instance_number: 1,
            instance_type: Some(1),
            groups: [None, None, Some(9)],
            last_event_info: Some(1),
            input_value: None,
            last_event_mono_ms: Some(400),
        };
        assert_eq!(first, expected);
        assert_eq!(store.input_instance_view(0, 5, 1).map(|view| view.instance_number), Some(0));
        assert_eq!(store.input_instance_view(0, 5, 2), None);
        assert_eq!(store.input_device_view(0, 6), None);
        assert_eq!(ClusterInputReadPort::input_instance_groups(&store, 0, 5, 1), [None, None, Some(9)]);
    }

    #[test]
    fn an_event_stamp_held_in_reach_reads_old_however_long_ago_the_press_was() {
        let store = store_with_device();
        let pressed_at_ms = 400_u32;
        let step_ms = AGE_CAP_MS / 2;
        for steps in 1..=7_u32 {
            let since_ms = steps * step_ms;
            let now_ms = pressed_at_ms.wrapping_add(since_ms);
            store.hold_input_stamps_in_reach(now_ms);
            let at_ms = store.input_instance_view(0, 5, 0).and_then(|view| view.last_event_mono_ms).expect("a stamp");
            assert_eq!(elapsed_ms(now_ms, at_ms), since_ms.min(AGE_CAP_MS), "{steps} half caps after the press");
        }
        assert_eq!(store.input_instance_view(0, 5, 1).and_then(|view| view.last_event_mono_ms), None, "no stamp stays none");
    }
}
