use dali2rust_contracts::msg::{Dali103ScanProgressEvent, DaliInputEventObservedEvent};
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
    pub(crate) manual_config_active: bool,
    pub(crate) instance_status: Option<u8>,
    pub(crate) resolution: Option<u8>,
    pub(crate) last_event_info: Option<u16>,
    pub(crate) last_event_at_ms: Option<u64>,
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
            manual_config_active: false,
            last_event_info: None,
            last_event_at_ms: None,
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
    pub(crate) present: bool,
    pub(crate) last_seen_ms: Option<u64>,
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
            present: false,
            last_seen_ms: None,
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
        instance.event_count = instance.event_count.saturating_add(1);
        self.present = true;
        self.last_seen_ms = Some(event.observed_at_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::InputEventKind;

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
    fn an_event_is_attributed_only_when_the_scheme_named_an_instance() {
        let mut record = InputDeviceRecord::empty(0, 3);
        record.apply_scan_progress(&progress(1), 500);

        assert!(record.apply_input_event(&event(Some(0), 0x002)));
        assert_eq!(record.instances[0].event_count, 1);
        assert_eq!(record.instances[0].last_event_info, Some(0x002));

        assert!(!record.apply_input_event(&event(None, 0x002)));
        assert_eq!(record.instances[0].event_count, 1);
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

    #[test]
    fn the_instance_ceiling_is_the_standards_own() {
        assert_eq!(MAX_INSTANCES_PER_DEVICE, 32);
    }
}

use crate::runtime::registry::RegistryStore;

impl RegistryStore {
    pub(crate) fn apply_input_scan_progress(
        &self,
        progress: &Dali103ScanProgressEvent,
        now_ms: u64,
    ) -> bool {
        let key = (progress.registry_adapter_id, progress.short_address);
        let mut g = self.write_inner();
        let created = !g.input_devices.contains_key(&key);
        if created && g.input_devices.len() >= MAX_INPUT_DEVICES {
            return false;
        }
        let record = g
            .input_devices
            .entry(key)
            .or_insert_with(|| InputDeviceRecord::empty(key.0, key.1));
        if created && progress.presence_unproven {
            record.ha_expose = false;
        }
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
        let mut g = self.write_inner();
        let removed = g.input_devices.remove(&(adapter_id, short_address)).is_some();
        if removed {
            g.input_devices_revision = g.input_devices_revision.saturating_add(1);
        drop(g);
        self.dirty.mark_input_devices_dirty();
        }
        removed
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
    if let Some(active) = readback.manual_config_active {
        instance.manual_config_active = active;
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
    use dali2rust_domain::dali::dev103::{feedback_capability, feedback_colour_capability};
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
    pub manual_config_active: bool,
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

pub(crate) fn persistable_snapshot_bank(inner: &Inner, bank: u8) -> PersistableInputDevicesSlice {
    let mut keys: Vec<(u8, u8)> = inner.input_devices.keys().copied().collect();
    keys.sort_unstable();
    let devices = keys
        .iter()
        .skip(usize::from(bank) * INPUT_DEVICES_PER_BANK)
        .take(INPUT_DEVICES_PER_BANK)
        .filter_map(|key| inner.input_devices.get(key))
        .map(persist_device)
        .collect();
    PersistableInputDevicesSlice { devices }
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
    _bank: u8,
    slice: &PersistableInputDevicesSlice,
) {
    hydrate_input_devices_inner(inner, slice);
}

pub(crate) fn hydrate_input_devices_inner(inner: &mut Inner, slice: &PersistableInputDevicesSlice) {
    for device in &slice.devices {
        let record = inner
            .input_devices
            .entry((device.adapter_id, device.short_address))
            .or_insert_with(|| {
                InputDeviceRecord::empty(device.adapter_id, device.short_address)
            });
        record.name = device.name.clone();
        record.notes = device.notes.clone();
        record.ha_expose = device.ha_expose;
        record.instance_count = device.instance_count;
        for persisted in &device.instances {
            hydrate_instance(record, persisted);
        }
    }
    inner.input_devices_revision = inner.input_devices_revision.saturating_add(1);
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
    use crate::runtime::registry::persistence_slices::{
        decode_versioned_slice, encode_persistence_blob, PersistenceEnvelope,
        INPUT_DEVICES_SLICE_VERSION,
    };

    fn worst_case_inner() -> Inner {
        let mut inner = Inner::default();
        for short in 0u8..32 {
            let mut record = InputDeviceRecord::empty(0, short);
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

    #[test]
    fn the_worst_case_bank_fits_its_slot() {
        let inner = worst_case_inner();
        for bank in 0..4u8 {
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
        for bank in 0..4u8 {
            let snapshot = persistable_snapshot_bank(&inner, bank);
            let envelope = PersistenceEnvelope::new(INPUT_DEVICES_SLICE_VERSION, &snapshot);
            let bytes = encode_persistence_blob(&envelope).expect("encode");
            let decoded: PersistableInputDevicesSlice =
                decode_versioned_slice(&bytes, INPUT_DEVICES_SLICE_VERSION).expect("decode");
            assert_eq!(decoded, snapshot, "postcard round-trip must be identity");
            hydrate_input_devices_inner(&mut hydrated, &decoded);
        }
        assert_eq!(hydrated.input_devices.len(), inner.input_devices.len());
        let original = inner.input_devices.get(&(0, 7)).expect("device 7");
        let back = hydrated.input_devices.get(&(0, 7)).expect("hydrated 7");
        assert_eq!(back.name, original.name);
        assert_eq!(back.instance_count, original.instance_count);
        assert_eq!(back.instances.len(), original.instances.len());
        let instance = back.instances.first().expect("instance 0");
        assert_eq!(instance.event_scheme.value, Some(4));
        assert_eq!(instance.event_scheme.read_at_ms, None);
        assert!(!back.present);
    }
}
