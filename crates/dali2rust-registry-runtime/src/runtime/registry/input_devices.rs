use std::collections::{HashMap, HashSet};

use dali2rust_contracts::msg::{
    Dali103ScanProgressEvent, DaliInputEventObservedEvent, InputEventKind,
};
use dali2rust_domain::dali::dev103::{EventScheme, FeedbackOpcodeMap};
use dali2rust_domain::registry::HaInputInstanceView;
use dali2rust_platform::slice_store::SliceKey;
use dali2rust_platform::small_sort::insertion_sort_by;

use super::store::Inner;

pub(crate) const MAX_INPUT_DEVICES: usize = 32;

pub(crate) const MAX_INSTANCES_PER_DEVICE: usize = 32;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadValue<T> {
    pub value: Option<T>,
    pub read_at_ms: Option<u64>,
}

impl<T> ReadValue<T> {
    pub const fn unread() -> Self {
        Self {
            value: None,
            read_at_ms: None,
        }
    }

    pub fn observed(value: T, at_ms: u64) -> Self {
        Self {
            value: Some(value),
            read_at_ms: Some(at_ms),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstanceRecord {
    pub(crate) instance_number: u8,
    pub(crate) instance_type: Option<u8>,
    pub(crate) event_filter: ReadValue<[u8; 3]>,
    pub(crate) event_scheme: ReadValue<u8>,
    pub(crate) event_priority: ReadValue<u8>,
    pub(crate) instance_groups: [ReadValue<Option<u8>>; 3],
    pub(crate) timers: [ReadValue<u8>; 4],
    pub(crate) manual_config_active: Option<bool>,
    pub(crate) instance_status: Option<u8>,
    pub(crate) resolution: Option<u8>,
    pub(crate) last_event_info: Option<u16>,
    pub(crate) last_event_at_ms: Option<u64>,
    pub(crate) last_event_mono_ms: Option<u32>,
    pub(crate) event_count: u32,
    pub(crate) input_value: Option<u16>,
    pub(crate) feedback_probe: Option<u8>,
    pub(crate) feedback_capability: ReadValue<u8>,
    pub(crate) feedback_colour_capability: ReadValue<u8>,
    pub(crate) feedback_timing: ReadValue<u8>,
    pub(crate) feedback_active_brightness: ReadValue<u8>,
    pub(crate) feedback_active_colour: ReadValue<u8>,
    pub(crate) feedback_inactive_brightness: ReadValue<u8>,
    pub(crate) feedback_inactive_colour: ReadValue<u8>,
}

impl InstanceRecord {
    pub(crate) const fn empty(instance_number: u8) -> Self {
        Self {
            instance_number,
            instance_type: None,
            instance_status: None,
            resolution: None,
            event_filter: ReadValue::unread(),
            event_scheme: ReadValue::unread(),
            event_priority: ReadValue::unread(),
            instance_groups: [ReadValue::unread(); 3],
            timers: [ReadValue::unread(); 4],
            manual_config_active: None,
            last_event_info: None,
            last_event_at_ms: None,
            last_event_mono_ms: None,
            event_count: 0,
            input_value: None,
            feedback_probe: None,
            feedback_capability: ReadValue::unread(),
            feedback_colour_capability: ReadValue::unread(),
            feedback_timing: ReadValue::unread(),
            feedback_active_brightness: ReadValue::unread(),
            feedback_active_colour: ReadValue::unread(),
            feedback_inactive_brightness: ReadValue::unread(),
            feedback_inactive_colour: ReadValue::unread(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InputDeviceRecord {
    pub(crate) adapter_id: u8,
    pub(crate) short_address: u8,
    pub(crate) name: Option<String>,
    pub(crate) notes: Option<String>,
    pub(crate) ha_expose: bool,
    pub(crate) instance_count: u8,
    pub(crate) device_capabilities: Option<u8>,
    pub(crate) device_status: Option<u8>,
    pub(crate) version_number: Option<u8>,
    pub(crate) instances: Vec<InstanceRecord>,
    pub(crate) nvm_settling_until_ms: Option<u64>,
    pub(crate) manual_config_changed_at_ms: Option<u64>,
    pub(crate) present: bool,
    pub(crate) last_seen_ms: Option<u64>,
    pub(crate) bank: u8,
}

impl InputDeviceRecord {
    pub(crate) fn empty(adapter_id: u8, short_address: u8) -> Self {
        Self {
            adapter_id,
            short_address,
            name: None,
            notes: None,
            ha_expose: true,
            instance_count: 0,
            device_capabilities: None,
            device_status: None,
            version_number: None,
            instances: Vec::new(),
            nvm_settling_until_ms: None,
            manual_config_changed_at_ms: None,
            present: false,
            last_seen_ms: None,
            bank: 0,
        }
    }

    pub(crate) fn instance_mut_or_declare(
        &mut self,
        instance_number: u8,
    ) -> Option<&mut InstanceRecord> {
        if self.instance_mut(instance_number).is_none() {
            let declared = instance_number < self.instance_count;
            let capped = self.instances.len() >= MAX_INSTANCES_PER_DEVICE;
            if !declared || capped {
                return None;
            }
            self.instances.push(InstanceRecord::empty(instance_number));
        }
        self.instance_mut(instance_number)
    }

    pub(crate) fn instance_mut(&mut self, instance_number: u8) -> Option<&mut InstanceRecord> {
        self.instances
            .iter_mut()
            .find(|i| i.instance_number == instance_number)
    }

    pub(crate) fn instance_views(&self) -> Vec<HaInputInstanceView> {
        self.instances
            .iter()
            .map(|i| HaInputInstanceView {
                instance_number: i.instance_number,
                instance_type: i.instance_type,
            })
            .collect()
    }

    pub(crate) fn apply_scan_progress(
        &mut self,
        progress: &Dali103ScanProgressEvent,
        now_ms: u64,
    ) -> bool {
        let mut changed = !self.present;
        self.present = true;
        self.last_seen_ms = Some(now_ms);
        if self.instance_count != progress.instance_count {
            self.instance_count = progress.instance_count;
            changed = true;
        }
        for (field, seen) in [
            (&mut self.device_capabilities, progress.device_capabilities),
            (&mut self.device_status, progress.device_status),
            (&mut self.version_number, progress.version_number),
        ] {
            changed |= merge_if_answered(field, seen);
        }
        changed
    }

    pub(crate) fn apply_input_event(
        &mut self,
        event: &DaliInputEventObservedEvent,
    ) -> bool {
        let Some(number) = event.instance_number else {
            return false;
        };
        let Some(instance) = self.instance_mut_or_declare(number) else {
            return false;
        };
        instance.last_event_info = Some(event.event_info);
        instance.last_event_at_ms = Some(event.observed_at_ms);
        instance.last_event_mono_ms = Some(event.observed_at_mono_ms);
        instance.event_count = instance.event_count.saturating_add(1);
        if matches!(
            event.typed,
            InputEventKind::Occupancy | InputEventKind::Position | InputEventKind::Illuminance
        ) {
            instance.input_value = Some(event.typed_value);
        }
        self.present = true;
        self.last_seen_ms = Some(event.observed_at_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::InputEventKind;
    use dali2rust_domain::registry::HaPublishReadPort;

    fn progress(instance_count: u8) -> Dali103ScanProgressEvent {
        Dali103ScanProgressEvent {
            registry_adapter_id: 0,
            short_address: 3,
            presence_unproven: false,
            instance_count,
            device_capabilities: None,
            device_status: None,
            version_number: None,
        }
    }

    fn event(instance_number: Option<u8>, info: u16) -> DaliInputEventObservedEvent {
        DaliInputEventObservedEvent {
            registry_adapter_id: 0,
            scheme: 2,
            short_address: Some(3),
            device_group: None,
            instance_group: None,
            instance_number,
            instance_type: None,
            event_info: info,
            typed: InputEventKind::Generic,
            typed_value: 0,
            observed_at_ms: 1_000,
            observed_at_mono_ms: 10,
        }
    }

    #[test]
    fn a_scan_reports_presence_and_the_declared_count_and_the_change() {
        let mut record = InputDeviceRecord::empty(0, 3);
        assert!(record.apply_scan_progress(&progress(2), 500));
        assert!(record.present);
        assert_eq!(record.instance_count, 2);
        assert!(record.instances.is_empty(), "an instance is known from its own answer");
        assert!(!record.apply_scan_progress(&progress(2), 600));
    }

    fn typed_readback(instance_type: u8) -> InstanceReadback {
        InstanceReadback {
            instance_type: Some(instance_type),
            ..InstanceReadback::default()
        }
    }

    #[test]
    fn an_instance_type_lands_on_the_number_that_answered() {
        let store = RegistryStore::with_adapter_count(1);
        assert!(store.apply_input_scan_progress(&progress(12), 500));
        assert!(store.apply_instance_readback(0, 3, 9, &typed_readback(4), 500));
        assert!(store.apply_instance_readback(0, 3, 2, &typed_readback(1), 500));
        assert_eq!(store.input_instance_type(0, 3, 9), Some(4));
        assert_eq!(store.input_instance_type(0, 3, 2), Some(1));
        assert_eq!(store.input_instance_type(0, 3, 1), None, "no answer, no type");
        assert!(
            !store.apply_instance_readback(0, 3, 12, &typed_readback(1), 500),
            "an instance past the declared count is not invented"
        );
    }

    #[test]
    fn a_forgotten_device_leaves_its_instances_to_retract_until_it_is_back() {
        let store = RegistryStore::with_adapter_count(1);
        assert!(store.apply_input_scan_progress(&progress(2), 500));
        assert!(store.apply_instance_readback(0, 3, 0, &typed_readback(1), 500));
        assert!(store.apply_instance_readback(0, 3, 1, &typed_readback(3), 500));
        assert!(store.ha_retracted_input_views(0).is_empty(), "a live device retracts nothing");

        assert!(store.forget_input_device(0, 3));
        let retracted = store.ha_retracted_input_views(0);
        assert_eq!(retracted.len(), 1);
        assert_eq!(retracted[0].short_address, 3);
        assert!(!retracted[0].ha_expose, "a forgotten device is never announced");
        let instances: Vec<_> =
            retracted[0].instances.iter().map(|i| (i.instance_number, i.instance_type)).collect();
        assert_eq!(instances, [(0, Some(1)), (1, Some(3))]);
        assert!(store.ha_retracted_input_views(1).is_empty(), "the other adapter forgot nothing");

        assert!(store.apply_input_scan_progress(&progress(2), 600));
        assert!(
            store.ha_retracted_input_views(0).is_empty(),
            "a device back at its address is announced, not retracted"
        );
    }

    #[test]
    fn an_event_is_attributed_only_when_the_scheme_named_an_instance() {
        let mut record = InputDeviceRecord::empty(0, 3);
        record.apply_scan_progress(&progress(1), 500);

        assert!(record.apply_input_event(&event(Some(0), 0x002)));
        assert_eq!(record.instances[0].event_count, 1);
        assert_eq!(record.instances[0].last_event_info, Some(0x002));

        assert!(!record.apply_input_event(&event(None, 0x002)));
        assert_eq!(record.instances[0].event_count, 1);
        assert_eq!(record.instances[0].input_value, None, "a generic event carries no value");
    }

    #[test]
    fn a_sensor_event_records_the_value_it_carries() {
        let mut record = InputDeviceRecord::empty(0, 3);
        record.apply_scan_progress(&progress(1), 500);
        let mut lux = event(Some(0), 0x155);
        lux.typed = InputEventKind::Illuminance;
        lux.typed_value = 0x155;
        assert!(record.apply_input_event(&lux));
        assert_eq!(record.instances[0].input_value, Some(0x155));
        let mut press = event(Some(0), 0x001);
        press.typed = InputEventKind::Button;
        press.typed_value = 0x001;
        assert!(record.apply_input_event(&press));
        assert_eq!(record.instances[0].input_value, Some(0x155), "a button press is not a reading");
    }

    #[test]
    fn timers_start_unread_and_that_is_not_zero() {
        let instance = InstanceRecord::empty(0);
        for timer in instance.timers {
            assert_eq!(timer.value, None);
            assert_eq!(timer.read_at_ms, None);
        }
        assert_eq!(instance.event_scheme.value, None);
    }

    fn view_after(readback: &InstanceReadback) -> InstanceView {
        let mut record = InputDeviceRecord::empty(0, 3);
        record.apply_scan_progress(&progress(1), 500);
        let instance = record.instance_mut_or_declare(0).expect("an instance within the cap");
        apply_config_readback(instance, readback, 500);
        apply_feedback_readback(instance, readback, 500);
        InstanceView::of(instance)
    }

    #[test]
    fn only_the_scheme_the_product_configures_reads_as_confirmed() {
        assert!(!view_after(&InstanceReadback::default()).event_scheme_confirmed(), "unread is not confirmed");
        for code in 0..=4 {
            let readback = InstanceReadback { event_scheme: Some(code), ..InstanceReadback::default() };
            assert_eq!(
                view_after(&readback).event_scheme_confirmed(),
                EventScheme::from_code(code) == Some(EventScheme::DeviceInstance),
                "scheme {code}"
            );
        }
        let readback = InstanceReadback { event_scheme: Some(2), ..InstanceReadback::default() };
        assert!(view_after(&readback).event_scheme_confirmed());
    }

    #[test]
    fn a_feedback_probe_answer_names_the_opcode_map_it_found() {
        let probed = |map| view_after(&InstanceReadback { feedback_opcode_map: Some(map), ..InstanceReadback::default() });
        assert_eq!(view_after(&InstanceReadback::default()).feedback_map(), None, "not probed");
        assert_eq!(probed(0).feedback_map(), None, "probed and absent");
        assert_eq!(probed(1).feedback_map(), Some(FeedbackOpcodeMap::DiiaCorrected));
        assert_eq!(probed(2).feedback_map(), Some(FeedbackOpcodeMap::Ed1));
        assert_eq!(probed(3).feedback_map(), None, "a code no map has");
    }

    #[test]
    fn the_instance_ceiling_is_the_standards_own() {
        assert_eq!(MAX_INSTANCES_PER_DEVICE, 32);
    }
}

use crate::runtime::registry::RegistryStore;
use dali2rust_domain::dali::dev103::{feedback_capability, feedback_colour_capability};

fn admitting_bank(devices: &HashMap<(u8, u8), InputDeviceRecord>) -> Option<u8> {
    if devices.len() >= MAX_INPUT_DEVICES {
        return None;
    }
    (0..SliceKey::INPUT_DEVICE_BANKS).find(|bank| {
        devices.values().filter(|device| device.bank == *bank).count() < INPUT_DEVICES_PER_BANK
    })
}

fn admit_input_device(
    devices: &mut HashMap<(u8, u8), InputDeviceRecord>,
    key: (u8, u8),
    presence_unproven: bool,
) -> Option<&mut InputDeviceRecord> {
    if !devices.contains_key(&key) {
        let mut record = InputDeviceRecord::empty(key.0, key.1);
        record.bank = admitting_bank(devices)?;
        record.ha_expose = !presence_unproven;
        devices.insert(key, record);
    }
    devices.get_mut(&key)
}

impl RegistryStore {
    pub(crate) fn apply_input_scan_progress(
        &self,
        progress: &Dali103ScanProgressEvent,
        now_ms: u64,
    ) -> bool {
        let key = (progress.registry_adapter_id, progress.short_address);
        let mut g = self.write_inner();
        let Some(record) = admit_input_device(&mut g.input_devices, key, progress.presence_unproven)
        else {
            return false;
        };
        if !record.apply_scan_progress(progress, now_ms) {
            return false;
        }
        g.input_devices_revision = g.input_devices_revision.saturating_add(1);
        drop(g);
        self.dirty.mark_input_devices_dirty();
        true
    }

    pub(crate) fn apply_input_event(&self, event: &DaliInputEventObservedEvent) -> bool {
        let Some(short_address) = event.short_address else {
            return false;
        };
        let mut g = self.write_inner();
        let Some(record) = g
            .input_devices
            .get_mut(&(event.registry_adapter_id, short_address))
        else {
            return false;
        };
        record.apply_input_event(event)
    }

    pub(crate) fn note_input_device_power_cycle(
        &self,
        adapter_id: u8,
        short_address: u8,
        at_ms: u64,
    ) -> bool {
        let mut g = self.write_inner();
        let Some(record) = g.input_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        record.present = true;
        record.last_seen_ms = Some(at_ms);
        record.nvm_settling_until_ms = None;
        true
    }

    pub(crate) fn note_input_device_manual_config(
        &self,
        adapter_id: u8,
        short_address: u8,
        at_ms: u64,
        holds_the_address: bool,
    ) -> bool {
        let mut g = self.write_inner();
        let Some(record) = g.input_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        record.manual_config_changed_at_ms = Some(at_ms);
        if holds_the_address {
            record.present = true;
            record.last_seen_ms = Some(at_ms);
        }
        true
    }

    pub fn input_device_summaries(&self, adapter_id: u8) -> Vec<InputDeviceSummary> {
        let g = self.read_inner();
        let mut out: Vec<InputDeviceSummary> = g
            .input_devices
            .values()
            .filter(|record| record.adapter_id == adapter_id)
            .map(InputDeviceSummary::from_record)
            .collect();
        insertion_sort_by(&mut out, |a, b| a.short_address > b.short_address);
        out
    }

    pub fn input_devices_revision(&self) -> u32 {
        self.read_inner().input_devices_revision
    }

    pub(crate) fn forget_input_device(&self, adapter_id: u8, short_address: u8) -> bool {
        let key = (adapter_id, short_address);
        let mut g = self.write_inner();
        let Some(record) = g.input_devices.remove(&key) else {
            return false;
        };
        g.retracted_inputs.insert(key, record.instance_views());
        g.input_devices_revision = g.input_devices_revision.saturating_add(1);
        drop(g);
        self.dirty.mark_input_devices_dirty();
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceSummary {
    pub adapter_id: u8,
    pub device_capabilities: Option<u8>,
    pub device_status: Option<u8>,
    pub version_number: Option<u8>,
    pub short_address: u8,
    pub name: Option<String>,
    pub present: bool,
    pub instance_count: u8,
    pub first_instance_type: Option<u8>,
    pub ha_expose: bool,
    pub last_seen_ms: Option<u64>,
    pub last_event_at_ms: Option<u64>,
}

impl InputDeviceSummary {
    fn from_record(record: &InputDeviceRecord) -> Self {
        Self {
            adapter_id: record.adapter_id,
            short_address: record.short_address,
            name: record.name.clone(),
            present: record.present,
            instance_count: record.instance_count,
            device_capabilities: record.device_capabilities,
            device_status: record.device_status,
            version_number: record.version_number,
            first_instance_type: record.instances.first().and_then(|i| i.instance_type),
            ha_expose: record.ha_expose,
            last_seen_ms: record.last_seen_ms,
            last_event_at_ms: record
                .instances
                .iter()
                .filter_map(|i| i.last_event_at_ms)
                .max(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstanceReadback {
    pub instance_type: Option<u8>,
    pub instance_status: Option<u8>,
    pub resolution: Option<u8>,
    pub instance_status_written: bool,
    pub event_scheme: Option<u8>,
    pub event_filter: Option<[u8; 3]>,
    pub event_priority: Option<u8>,
    pub instance_groups: [Option<Option<u8>>; 3],
    pub timers: [Option<u8>; 4],
    pub manual_config_active: Option<bool>,
    pub feedback_opcode_map: Option<u8>,
    pub feedback_capability: Option<u8>,
    pub feedback_colour_capability: Option<u8>,
    pub feedback_timing: Option<u8>,
    pub feedback_active_brightness: Option<u8>,
    pub feedback_active_colour: Option<u8>,
    pub feedback_inactive_brightness: Option<u8>,
    pub feedback_inactive_colour: Option<u8>,
}

impl InstanceReadback {
    fn carries_feedback_config(&self) -> bool {
        self.feedback_timing.is_some()
            || self.feedback_active_brightness.is_some()
            || self.feedback_active_colour.is_some()
            || self.feedback_inactive_brightness.is_some()
            || self.feedback_inactive_colour.is_some()
    }

    fn carries_written_variable(&self) -> bool {
        self.event_scheme.is_some()
            || self.event_filter.is_some()
            || self.event_priority.is_some()
            || self.instance_groups.iter().any(Option::is_some)
            || self.timers.iter().any(Option::is_some)
            || self.carries_feedback_config()
            || self.instance_status_written
    }
}

fn apply_config_readback(instance: &mut InstanceRecord, readback: &InstanceReadback, now_ms: u64) {
    if let Some(scheme) = readback.event_scheme {
        instance.event_scheme = ReadValue::observed(scheme, now_ms);
    }
    if let Some(filter) = readback.event_filter {
        instance.event_filter = ReadValue::observed(filter, now_ms);
    }
    if let Some(priority) = readback.event_priority {
        instance.event_priority = ReadValue::observed(priority, now_ms);
    }
    for (slot, group) in readback.instance_groups.iter().enumerate() {
        if let Some(group) = group {
            instance.instance_groups[slot] = ReadValue::observed(*group, now_ms);
        }
    }
    for (slot, timer) in readback.timers.iter().enumerate() {
        if let Some(timer) = timer {
            instance.timers[slot] = ReadValue::observed(*timer, now_ms);
        }
    }
}

fn merge_if_answered<T: Copy + PartialEq>(slot: &mut Option<T>, answered: Option<T>) -> bool {
    if answered.is_none() || *slot == answered {
        return false;
    }
    *slot = answered;
    true
}

fn apply_feedback_readback(instance: &mut InstanceRecord, readback: &InstanceReadback, now_ms: u64) {
    merge_if_answered(&mut instance.instance_type, readback.instance_type);
    merge_if_answered(&mut instance.instance_status, readback.instance_status);
    merge_if_answered(&mut instance.resolution, readback.resolution);
    merge_if_answered(&mut instance.manual_config_active, readback.manual_config_active);
    if let Some(map) = readback.feedback_opcode_map {
        instance.feedback_probe = Some(map);
    }
    for (slot, value) in [
        (&mut instance.feedback_capability, readback.feedback_capability),
        (
            &mut instance.feedback_colour_capability,
            readback.feedback_colour_capability,
        ),
        (&mut instance.feedback_timing, readback.feedback_timing),
        (
            &mut instance.feedback_active_brightness,
            readback.feedback_active_brightness,
        ),
        (
            &mut instance.feedback_active_colour,
            readback.feedback_active_colour,
        ),
        (
            &mut instance.feedback_inactive_brightness,
            readback.feedback_inactive_brightness,
        ),
        (
            &mut instance.feedback_inactive_colour,
            readback.feedback_inactive_colour,
        ),
    ] {
        if let Some(value) = value {
            *slot = ReadValue::observed(value, now_ms);
        }
    }
}

fn invalidate_common_feedback(record: &mut InputDeviceRecord, written_instance: u8) {
    let common_brightness = record.instances.iter().any(|i| {
        i.feedback_capability
            .value
            .is_some_and(|cap| cap & feedback_capability::COMMON_BRIGHTNESS != 0)
    });
    let common_colour = record.instances.iter().any(|i| {
        i.feedback_colour_capability
            .value
            .is_some_and(|cap| cap & feedback_colour_capability::COMMON_COLOUR != 0)
    });
    if !(common_brightness || common_colour) {
        return;
    }
    for sibling in record
        .instances
        .iter_mut()
        .filter(|i| i.instance_number != written_instance)
    {
        if common_brightness {
            sibling.feedback_active_brightness = ReadValue::unread();
            sibling.feedback_inactive_brightness = ReadValue::unread();
        }
        if common_colour {
            sibling.feedback_active_colour = ReadValue::unread();
            sibling.feedback_inactive_colour = ReadValue::unread();
        }
    }
}

impl RegistryStore {
    pub(crate) fn clear_input_presence(&self, adapter_id: u8) -> u32 {
        let mut g = self.write_inner();
        let mut cleared = 0;
        for record in g
            .input_devices
            .values_mut()
            .filter(|r| r.adapter_id == adapter_id)
        {
            if record.present || record.last_seen_ms.is_some() {
                record.present = false;
                record.last_seen_ms = None;
                cleared += 1;
            }
        }
        if cleared > 0 {
            g.input_devices_revision = g.input_devices_revision.saturating_add(1);
        }
        drop(g);
        cleared
    }

    pub(crate) fn apply_instance_readback(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
        readback: &InstanceReadback,
        now_ms: u64,
    ) -> bool {
        let mut g = self.write_inner();
        let Some(record) = g.input_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        let Some(instance) = record.instance_mut_or_declare(instance_number) else {
            return false;
        };
        apply_config_readback(instance, readback, now_ms);
        apply_feedback_readback(instance, readback, now_ms);
        if readback.carries_feedback_config() {
            invalidate_common_feedback(record, instance_number);
        }
        if readback.carries_written_variable() {
            record.nvm_settling_until_ms = Some(now_ms.saturating_add(NVM_SETTLING_MS));
        }
        g.input_devices_revision = g.input_devices_revision.saturating_add(1);
        drop(g);
        self.dirty.mark_input_devices_dirty();
        true
    }

    pub(crate) fn patch_input_device_metadata(
        &self,
        adapter_id: u8,
        short_address: u8,
        name: Option<Option<String>>,
        notes: Option<Option<String>>,
        ha_expose: Option<bool>,
    ) -> bool {
        let mut g = self.write_inner();
        let Some(record) = g.input_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        if let Some(name) = name {
            record.name = name;
        }
        if let Some(notes) = notes {
            record.notes = notes;
        }
        if let Some(expose) = ha_expose {
            record.ha_expose = expose;
        }
        g.input_devices_revision = g.input_devices_revision.saturating_add(1);
        drop(g);
        self.dirty.mark_input_devices_dirty();
        true
    }

    pub fn input_device_detail(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<InputDeviceDetail> {
        let g = self.read_inner();
        let record = g.input_devices.get(&(adapter_id, short_address))?;
        Some(InputDeviceDetail {
            summary: InputDeviceSummary::from_record(record),
            notes: record.notes.clone(),
            nvm_settling_until_ms: record.nvm_settling_until_ms,
            manual_config_changed_at_ms: record.manual_config_changed_at_ms,
            instances: record.instances.iter().map(InstanceView::of).collect(),
        })
    }

    pub fn input_instance_type(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> Option<u8> {
        let g = self.read_inner();
        g.input_devices
            .get(&(adapter_id, short_address))?
            .instances
            .iter()
            .find(|i| i.instance_number == instance_number)?
            .instance_type
    }
}

impl dali2rust_domain::registry::InputInstanceTypeReadPort for RegistryStore {
    fn input_instance_type(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> Option<u8> {
        RegistryStore::input_instance_type(self, adapter_id, short_address, instance_number)
    }
}

pub const NVM_SETTLING_MS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceDetail {
    pub summary: InputDeviceSummary,
    pub notes: Option<String>,
    pub nvm_settling_until_ms: Option<u64>,
    pub manual_config_changed_at_ms: Option<u64>,
    pub instances: Vec<InstanceView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceView {
    pub instance_number: u8,
    pub instance_type: Option<u8>,
    pub instance_status: Option<u8>,
    pub resolution: Option<u8>,
    pub event_scheme: ReadValue<u8>,
    pub event_filter: ReadValue<[u8; 3]>,
    pub event_priority: ReadValue<u8>,
    pub instance_groups: [ReadValue<Option<u8>>; 3],
    pub timers: [ReadValue<u8>; 4],
    pub manual_config_active: Option<bool>,
    pub last_event_info: Option<u16>,
    pub last_event_at_ms: Option<u64>,
    pub event_count: u32,
    pub input_value: Option<u16>,
    pub feedback_probe: Option<u8>,
    pub feedback_capability: ReadValue<u8>,
    pub feedback_colour_capability: ReadValue<u8>,
    pub feedback_timing: ReadValue<u8>,
    pub feedback_active_brightness: ReadValue<u8>,
    pub feedback_active_colour: ReadValue<u8>,
    pub feedback_inactive_brightness: ReadValue<u8>,
    pub feedback_inactive_colour: ReadValue<u8>,
}

impl InstanceView {
    #[must_use]
    pub fn event_scheme_confirmed(&self) -> bool {
        self.event_scheme.value == Some(EventScheme::DeviceInstance.code())
    }

    #[must_use]
    pub fn feedback_map(&self) -> Option<FeedbackOpcodeMap> {
        self.feedback_probe.and_then(FeedbackOpcodeMap::from_code)
    }

    fn of(record: &InstanceRecord) -> Self {
        Self {
            instance_number: record.instance_number,
            instance_type: record.instance_type,
            instance_status: record.instance_status,
            resolution: record.resolution,
            event_scheme: record.event_scheme,
            event_filter: record.event_filter,
            event_priority: record.event_priority,
            instance_groups: record.instance_groups,
            timers: record.timers,
            manual_config_active: record.manual_config_active,
            last_event_info: record.last_event_info,
            last_event_at_ms: record.last_event_at_ms,
            event_count: record.event_count,
            input_value: record.input_value,
            feedback_probe: record.feedback_probe,
            feedback_capability: record.feedback_capability,
            feedback_colour_capability: record.feedback_colour_capability,
            feedback_timing: record.feedback_timing,
            feedback_active_brightness: record.feedback_active_brightness,
            feedback_active_colour: record.feedback_active_colour,
            feedback_inactive_brightness: record.feedback_inactive_brightness,
            feedback_inactive_colour: record.feedback_inactive_colour,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedInputInstance(pub [u8; 11]);

const PACKED_UNREAD: u8 = 0xFF;
const PACKED_NO_MEMBERSHIP: u8 = 0xFE;
const PACKED_FILTER_VALID: u8 = 1 << 0;

impl PersistedInputInstance {
    fn pack(instance: &InstanceRecord) -> Self {
        let mut bytes = [PACKED_UNREAD; 11];
        bytes[0] = instance.instance_number;
        bytes[1] = instance.instance_type.unwrap_or(PACKED_UNREAD);
        bytes[2] = instance.event_scheme.value.unwrap_or(PACKED_UNREAD);
        bytes[3] = instance.event_priority.value.unwrap_or(PACKED_UNREAD);
        match instance.event_filter.value {
            Some(filter) => {
                bytes[4] = PACKED_FILTER_VALID;
                bytes[5..8].copy_from_slice(&filter);
            }
            None => {
                bytes[4] = 0;
                bytes[5..8].fill(0);
            }
        }
        for slot in 0..3 {
            bytes[8 + slot] = match instance.instance_groups[slot].value {
                None => PACKED_UNREAD,
                Some(None) => PACKED_NO_MEMBERSHIP,
                Some(Some(group)) => group,
            };
        }
        Self(bytes)
    }

    fn instance_number(&self) -> u8 {
        self.0[0]
    }

    fn unpack_into(&self, instance: &mut InstanceRecord) {
        let unread = |byte: u8| (byte != PACKED_UNREAD).then_some(byte);
        instance.instance_type = unread(self.0[1]);
        instance.event_scheme = ReadValue { value: unread(self.0[2]), read_at_ms: None };
        instance.event_priority = ReadValue { value: unread(self.0[3]), read_at_ms: None };
        let filter = (self.0[4] & PACKED_FILTER_VALID != 0)
            .then(|| [self.0[5], self.0[6], self.0[7]]);
        instance.event_filter = ReadValue { value: filter, read_at_ms: None };
        for slot in 0..3 {
            let value = match self.0[8 + slot] {
                PACKED_UNREAD => None,
                PACKED_NO_MEMBERSHIP => Some(None),
                group => Some(Some(group)),
            };
            instance.instance_groups[slot] = ReadValue { value, read_at_ms: None };
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedInputDevice {
    pub adapter_id: u8,
    pub short_address: u8,
    pub name: Option<String>,
    pub notes: Option<String>,
    pub ha_expose: bool,
    pub instance_count: u8,
    pub instances: Vec<PersistedInputInstance>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistableInputDevicesSlice {
    pub devices: Vec<PersistedInputDevice>,
}

pub(crate) const INPUT_DEVICES_PER_BANK: usize = 8;

const _: () = assert!(
    MAX_INPUT_DEVICES <= INPUT_DEVICES_PER_BANK * SliceKey::INPUT_DEVICE_BANKS as usize,
    "every input device the registry admits has a place in a persisted bank"
);

pub(crate) fn persistable_snapshot_bank(inner: &Inner, bank: u8) -> PersistableInputDevicesSlice {
    let mut members: Vec<&InputDeviceRecord> =
        inner.input_devices.values().filter(|record| record.bank == bank).collect();
    insertion_sort_by(&mut members, |a, b| {
        (a.adapter_id, a.short_address) > (b.adapter_id, b.short_address)
    });
    PersistableInputDevicesSlice { devices: members.into_iter().map(persist_device).collect() }
}

fn persist_device(record: &InputDeviceRecord) -> PersistedInputDevice {
    PersistedInputDevice {
        adapter_id: record.adapter_id,
        short_address: record.short_address,
        name: record.name.clone(),
        notes: record.notes.clone(),
        ha_expose: record.ha_expose,
        instance_count: record.instance_count,
        instances: record.instances.iter().map(PersistedInputInstance::pack).collect(),
    }
}

pub(crate) fn hydrate_input_devices_bank_inner(
    inner: &mut Inner,
    bank: u8,
    slice: &PersistableInputDevicesSlice,
    stored: &mut HashSet<(u8, u8)>,
) -> bool {
    let mut held_lower = false;
    for device in &slice.devices {
        let key = (device.adapter_id, device.short_address);
        if stored.insert(key) {
            hydrate_device(inner, bank, key, device);
        } else {
            held_lower = true;
        }
    }
    inner.input_devices_revision = inner.input_devices_revision.saturating_add(1);
    held_lower
}

fn hydrate_device(inner: &mut Inner, bank: u8, key: (u8, u8), device: &PersistedInputDevice) {
    let record =
        inner.input_devices.entry(key).or_insert_with(|| InputDeviceRecord::empty(key.0, key.1));
    record.bank = bank;
    record.name = device.name.clone();
    record.notes = device.notes.clone();
    record.ha_expose = device.ha_expose;
    record.instance_count = device.instance_count;
    for persisted in &device.instances {
        hydrate_instance(record, persisted);
    }
}

pub(crate) fn rebank_input_devices(inner: &mut Inner, stored: &HashSet<(u8, u8)>) -> bool {
    let excess: Vec<InputDeviceRecord> = excess_input_devices(&inner.input_devices, stored)
        .iter()
        .filter_map(|key| inner.input_devices.remove(key))
        .collect();
    let rebanked = !excess.is_empty();
    for record in excess {
        readmit_input_device(&mut inner.input_devices, record);
    }
    if rebanked {
        inner.input_devices_revision = inner.input_devices_revision.saturating_add(1);
    }
    rebanked
}

fn excess_input_devices(
    devices: &HashMap<(u8, u8), InputDeviceRecord>,
    stored: &HashSet<(u8, u8)>,
) -> Vec<(u8, u8)> {
    let mut seats: Vec<(u8, bool, (u8, u8))> =
        devices.iter().map(|(key, record)| (record.bank, !stored.contains(key), *key)).collect();
    insertion_sort_by(&mut seats, |a, b| a > b);
    let mut taken = [0usize; SliceKey::INPUT_DEVICE_BANKS as usize];
    let mut kept = 0usize;
    let mut excess = Vec::new();
    for (bank, _, key) in seats {
        match taken.get_mut(usize::from(bank)) {
            Some(seat) if *seat < INPUT_DEVICES_PER_BANK && kept < MAX_INPUT_DEVICES => {
                *seat += 1;
                kept += 1;
            }
            _ => excess.push(key),
        }
    }
    excess
}

fn readmit_input_device(
    devices: &mut HashMap<(u8, u8), InputDeviceRecord>,
    mut record: InputDeviceRecord,
) {
    let key = (record.adapter_id, record.short_address);
    let Some(bank) = admitting_bank(devices) else {
        log::warn!(
            "persistence: input device a{}/s{} refused on load: \
             the {MAX_INPUT_DEVICES}-device cap is full",
            key.0,
            key.1
        );
        return;
    };
    log::info!(
        "persistence: input device a{}/s{} moves from bank {} to bank {bank}",
        key.0,
        key.1,
        record.bank
    );
    record.bank = bank;
    devices.insert(key, record);
}

fn hydrate_instance(record: &mut InputDeviceRecord, persisted: &PersistedInputInstance) {
    let number = persisted.instance_number();
    if record.instance_mut(number).is_none() {
        record.instances.push(InstanceRecord::empty(number));
    }
    let Some(instance) = record.instance_mut(number) else {
        return;
    };
    persisted.unpack_into(instance);
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use dali2rust_platform::slice_store::SliceStore;
    use crate::test_support::{scan_input_device, ProbeStore};
    use crate::runtime::registry::persistence_slices::{
        decode_versioned_slice, encode_persistence_blob, PersistenceEnvelope,
        INPUT_DEVICES_SLICE_VERSION,
    };

    fn worst_case_inner() -> Inner {
        let mut inner = Inner::default();
        for short in 0u8..32 {
            let mut record = InputDeviceRecord::empty(0, short);
            record.bank = u8::try_from(usize::from(short) / INPUT_DEVICES_PER_BANK).expect("a bank");
            record.instance_count = 32;
            record.name = Some("и".repeat(32));
            record.notes = Some("ж".repeat(24));
            record.ha_expose = true;
            for n in 0u8..32 {
                let instance = record
                    .instance_mut_or_declare(n)
                    .expect("declared instance materialises");
                instance.instance_type = Some(31);
                instance.event_scheme = ReadValue::observed(4, u64::MAX);
                instance.event_priority = ReadValue::observed(5, u64::MAX);
                instance.event_filter = ReadValue::observed([0xFF; 3], u64::MAX);
                for slot in 0..3 {
                    instance.instance_groups[slot] = ReadValue::observed(Some(31), u64::MAX);
                }
            }
            inner.input_devices.insert((0, short), record);
        }
        inner
    }

    fn bank_sizes(store: &RegistryStore) -> Vec<usize> {
        let inner = store.read_inner();
        (0..SliceKey::INPUT_DEVICE_BANKS)
            .map(|bank| persistable_snapshot_bank(&inner, bank).devices.len())
            .collect()
    }

    #[test]
    fn admission_fills_a_bank_before_the_next_and_refills_a_freed_place() {
        let store = RegistryStore::with_adapter_count(1);
        let cap = u8::try_from(MAX_INPUT_DEVICES).expect("a short address");
        for short in 0..cap {
            assert!(scan_input_device(&store, short), "short {short} admitted");
        }
        assert!(!scan_input_device(&store, cap), "the cap holds");
        let full = vec![INPUT_DEVICES_PER_BANK; usize::from(SliceKey::INPUT_DEVICE_BANKS)];
        assert_eq!(bank_sizes(&store), full);

        let freed = 3;
        assert!(store.forget_input_device(0, freed));
        assert!(scan_input_device(&store, cap), "a freed place admits a new device");
        assert_eq!(bank_sizes(&store), full);
        assert_eq!(store.read_inner().input_devices[&(0, cap)].bank, 0, "the freed place was in bank 0");
    }

    #[test]
    fn the_worst_case_bank_fits_its_slot() {
        let inner = worst_case_inner();
        for bank in 0..SliceKey::INPUT_DEVICE_BANKS {
            let snapshot = persistable_snapshot_bank(&inner, bank);
            assert_eq!(snapshot.devices.len(), INPUT_DEVICES_PER_BANK);
            let envelope = PersistenceEnvelope::new(INPUT_DEVICES_SLICE_VERSION, &snapshot);
            let bytes = encode_persistence_blob(&envelope).expect("encode");
            assert!(
                bytes.len() < dali2rust_bsp::slice_layout::SMALL_SLOT_PAYLOAD_BYTES as usize,
                "bank {bank}: {} B exceeds the 4080 B slot",
                bytes.len()
            );
        }
    }

    #[test]
    fn a_flushed_bank_hydrates_back_to_the_same_summary() {
        let inner = worst_case_inner();
        let mut hydrated = Inner::default();
        let mut stored = HashSet::new();
        for bank in 0..SliceKey::INPUT_DEVICE_BANKS {
            let snapshot = persistable_snapshot_bank(&inner, bank);
            let envelope = PersistenceEnvelope::new(INPUT_DEVICES_SLICE_VERSION, &snapshot);
            let bytes = encode_persistence_blob(&envelope).expect("encode");
            let decoded: PersistableInputDevicesSlice =
                decode_versioned_slice(&bytes, INPUT_DEVICES_SLICE_VERSION).expect("decode");
            assert_eq!(decoded, snapshot, "postcard round-trip must be identity");
            hydrate_input_devices_bank_inner(&mut hydrated, bank, &decoded, &mut stored);
        }
        assert_eq!(hydrated.input_devices.len(), inner.input_devices.len());
        let original = inner.input_devices.get(&(0, 7)).expect("device 7");
        let back = hydrated.input_devices.get(&(0, 7)).expect("hydrated 7");
        assert_eq!(back.name, original.name);
        assert_eq!(back.bank, original.bank, "a device hydrates into the bank it was saved in");
        assert_eq!(back.instance_count, original.instance_count);
        assert_eq!(back.instances.len(), original.instances.len());
        let instance = back.instances.first().expect("instance 0");
        assert_eq!(instance.event_scheme.value, Some(4));
        assert_eq!(instance.event_scheme.read_at_ms, None);
        assert!(!back.present);
    }

    fn stored_device(short_address: u8, name: &str) -> PersistedInputDevice {
        PersistedInputDevice {
            adapter_id: 0,
            short_address,
            name: Some(name.to_string()),
            notes: None,
            ha_expose: true,
            instance_count: 1,
            instances: vec![PersistedInputInstance::pack(&InstanceRecord::empty(0))],
        }
    }

    fn store_bank(slices: &ProbeStore, bank: u8, devices: Vec<PersistedInputDevice>) {
        let slice = PersistableInputDevicesSlice { devices };
        let envelope = PersistenceEnvelope::new(INPUT_DEVICES_SLICE_VERSION, &slice);
        let bytes = encode_persistence_blob(&envelope).expect("encode");
        let mut session = slices.begin_write(SliceKey::InputDevices { bank }).expect("open bank");
        session.append(&bytes).expect("append bank");
        session.commit().expect("commit bank");
    }

    fn stored_bank(slices: &ProbeStore, bank: u8) -> PersistableInputDevicesSlice {
        let bytes = slices.load(SliceKey::InputDevices { bank }).expect("bank stored");
        decode_versioned_slice(&bytes, INPUT_DEVICES_SLICE_VERSION).expect("decode")
    }

    fn store_named_bank(slices: &ProbeStore, bank: u8, shorts: std::ops::Range<u8>) {
        store_bank(slices, bank, shorts.map(|short| stored_device(short, "stored")).collect());
    }

    fn per_bank() -> u8 {
        u8::try_from(INPUT_DEVICES_PER_BANK).expect("a short address count")
    }

    fn booted(slices: &ProbeStore) -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        store.hydrate_from_store(slices, 1);
        store
    }

    #[test]
    fn an_over_full_stored_bank_moves_its_excess_to_the_first_bank_with_room() {
        let slices = ProbeStore::default();
        store_named_bank(&slices, 0, 0..per_bank() + 1);

        let store = booted(&slices);
        assert_eq!(bank_sizes(&store), vec![INPUT_DEVICES_PER_BANK, 1, 0, 0]);
        assert_eq!(store.read_inner().input_devices[&(0, per_bank())].bank, 1, "the highest key moves");

        store.flush_dirty_slices(&slices);
        assert_eq!(stored_bank(&slices, 0).devices.len(), INPUT_DEVICES_PER_BANK);
        let rebooted = booted(&slices);
        assert_eq!(bank_sizes(&rebooted), vec![INPUT_DEVICES_PER_BANK, 1, 0, 0]);
        assert!(!rebooted.dirty.any_dirty(), "banks within the cap are not rewritten");
    }

    #[test]
    fn stored_devices_beyond_the_device_cap_are_refused_on_load() {
        let slices = ProbeStore::default();
        store_named_bank(&slices, 0, 0..per_bank() + 1);
        for bank in 1..SliceKey::INPUT_DEVICE_BANKS {
            let first = 1 + bank * per_bank();
            store_named_bank(&slices, bank, first..first + per_bank());
        }

        let store = booted(&slices);
        let full = vec![INPUT_DEVICES_PER_BANK; usize::from(SliceKey::INPUT_DEVICE_BANKS)];
        assert_eq!(bank_sizes(&store), full);
        assert_eq!(store.read_inner().input_devices.len(), MAX_INPUT_DEVICES);
        assert!(
            !store.read_inner().input_devices.contains_key(&(0, per_bank())),
            "the excess of the over-full bank is what the cap refuses"
        );
    }

    #[test]
    fn a_reload_beside_an_unread_bank_keeps_each_bank_within_the_cap() {
        let slices = ProbeStore::default();
        let first_stored = 10;
        store_named_bank(&slices, 0, first_stored..first_stored + per_bank());
        let store = RegistryStore::with_adapter_count(1);
        assert!(scan_input_device(&store, 0), "short 0 admitted");

        slices.fail_reads_of(Some(SliceKey::InputDevices { bank: 3 }));
        store.hydrate_from_store(&slices, 1);
        assert_eq!(bank_sizes(&store), vec![INPUT_DEVICES_PER_BANK, 1, 0, 0]);
        assert_eq!(store.read_inner().input_devices[&(0, 0)].bank, 1, "stored members keep their bank");
    }

    #[test]
    fn a_device_stored_in_two_banks_takes_the_lower_banks_copy() {
        let slices = ProbeStore::default();
        let short = 5;
        let mut stale = stored_device(short, "old");
        stale.instance_count = 2;
        stale.instances.push(PersistedInputInstance::pack(&InstanceRecord::empty(1)));
        store_bank(&slices, 0, vec![stored_device(short, "new")]);
        store_bank(&slices, 2, vec![stale]);

        let store = booted(&slices);
        let inner = store.read_inner();
        let device = &inner.input_devices[&(0, short)];
        assert_eq!((device.bank, device.name.as_deref()), (0, Some("new")));
        assert_eq!((device.instance_count, device.instances.len()), (1, 1), "nothing of the old copy");
        drop(inner);

        store.flush_dirty_slices(&slices);
        assert!(stored_bank(&slices, 2).devices.is_empty(), "the stale copy is rewritten away");
    }
}
