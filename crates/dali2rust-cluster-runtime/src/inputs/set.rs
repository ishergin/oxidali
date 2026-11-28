use core::ptr;

use dali2rust_contracts::msg::{fixed_text_32, fixed_text_64, FixedText32, FixedText64, InputEventKind};
use dali2rust_domain::registry::{CLUSTER_INPUT_DEVICES, CLUSTER_INPUT_INSTANCES, INPUT_INSTANCE_GROUPS};

use crate::frame::{InputDevice, InputEvent, InputInfoElement, InputInstance, InputOwner};
use crate::presence::{in_reach, later};

pub const DEVICE_SLOTS: usize = CLUSTER_INPUT_DEVICES;

pub const INSTANCE_SLOTS: usize = CLUSTER_INPUT_INSTANCES;

const NONE: u8 = u8::MAX;

const NO_VALUE: u16 = u16::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldOwner {
    pub controller_id: FixedText32,
    pub discovery_prefix: FixedText32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldDevice {
    pub adapter: u8,
    pub device: u8,
    pub name: FixedText64,
    pub ha_entity: bool,
    pub present: bool,
    pub instances: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldValue {
    pub event_info: u16,
    pub value: Option<u16>,
    pub heard_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldInstance {
    pub instance: u8,
    pub instance_type: Option<u8>,
    pub groups: [Option<u8>; INPUT_INSTANCE_GROUPS],
    pub last: Option<HeldValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Packed {
    instance_type: u8,
    groups: [u8; INPUT_INSTANCE_GROUPS],
    event_info: u16,
    value: u16,
    heard_ms: u32,
    known: bool,
    heard: bool,
}

const fn pack_byte(value: Option<u8>) -> u8 {
    match value {
        Some(byte) => byte,
        None => NONE,
    }
}

const fn unpack_byte(byte: u8) -> Option<u8> {
    if byte == NONE {
        None
    } else {
        Some(byte)
    }
}

impl Packed {
    const VACANT: Self = Self {
        instance_type: NONE,
        groups: [NONE; INPUT_INSTANCE_GROUPS],
        event_info: 0,
        value: NO_VALUE,
        heard_ms: 0,
        known: false,
        heard: false,
    };

    const fn last(&self) -> Option<HeldValue> {
        if !self.heard {
            return None;
        }
        let value = if self.value == NO_VALUE { None } else { Some(self.value) };
        Some(HeldValue { event_info: self.event_info, value, heard_ms: self.heard_ms })
    }

    fn hear(&mut self, last: Option<HeldValue>) {
        let Some(last) = last else {
            return;
        };
        self.event_info = last.event_info;
        self.value = last.value.unwrap_or(NO_VALUE);
        self.heard_ms = last.heard_ms;
        self.heard = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Received {
    pub(crate) now_ms: u32,
}

impl Received {
    pub(crate) const fn heard_ms(self, age_ms: u32) -> u32 {
        self.now_ms.wrapping_sub(age_ms)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InputSet {
    owner: Option<HeldOwner>,
    devices: [Option<HeldDevice>; DEVICE_SLOTS],
    instances: [[Packed; INSTANCE_SLOTS]; DEVICE_SLOTS],
    pub(crate) skipped: u16,
}

impl InputSet {
    pub(crate) unsafe fn init(set: *mut Self) {
        // SAFETY: the caller hands a place of `InputSet`; every field is written once here and nothing is read.
        unsafe {
            ptr::addr_of_mut!((*set).owner).write(None);
            let devices = ptr::addr_of_mut!((*set).devices).cast::<Option<HeldDevice>>();
            for index in 0..DEVICE_SLOTS {
                devices.add(index).write(None);
            }
            let instances = ptr::addr_of_mut!((*set).instances).cast::<Packed>();
            for index in 0..DEVICE_SLOTS * INSTANCE_SLOTS {
                instances.add(index).write(Packed::VACANT);
            }
            ptr::addr_of_mut!((*set).skipped).write(0);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.owner = None;
        self.devices.iter_mut().for_each(|slot| *slot = None);
        self.instances.iter_mut().flatten().for_each(|slot| *slot = Packed::VACANT);
        self.skipped = 0;
    }

    fn slot_of(&self, adapter: u8, device: u8) -> Option<usize> {
        self.devices.iter().position(|slot| slot.as_ref().is_some_and(|held| (held.adapter, held.device) == (adapter, device)))
    }

    pub(crate) fn take(&mut self, element: &InputInfoElement<'_>, received: Received) -> bool {
        match element {
            InputInfoElement::Owner(owner) => {
                self.owner = Some(held_owner(owner));
                true
            }
            InputInfoElement::Device(device) => self.take_device(device),
            InputInfoElement::Instance(instance) => self.take_instance(instance, received),
        }
    }

    fn take_device(&mut self, device: &InputDevice<'_>) -> bool {
        let slot = self.slot_of(device.adapter, device.device).or_else(|| self.devices.iter().position(Option::is_none));
        let Some(place) = slot.and_then(|slot| self.devices.get_mut(slot)) else {
            return false;
        };
        *place = Some(HeldDevice {
            adapter: device.adapter,
            device: device.device,
            name: fixed_text_64(device.name),
            ha_entity: device.ha_entity,
            present: device.present,
            instances: device.instances,
        });
        true
    }

    fn take_instance(&mut self, instance: &InputInstance, received: Received) -> bool {
        let Some(slot) = self.slot_of(instance.adapter, instance.device) else {
            return false;
        };
        let Some(packed) = self.instances.get_mut(slot).and_then(|row| row.get_mut(usize::from(instance.instance))) else {
            return false;
        };
        *packed = Packed {
            instance_type: pack_byte(instance.instance_type),
            groups: instance.groups.map(pack_byte),
            known: true,
            ..Packed::VACANT
        };
        packed.hear(instance.last.map(|last| HeldValue {
            event_info: last.event_info,
            value: last.value,
            heard_ms: received.heard_ms(last.age_ms),
        }));
        true
    }

    pub(crate) fn keep_newer_from(&mut self, shown: &Self, now_ms: u32) {
        for (slot, device) in self.devices.iter().enumerate() {
            let Some(device) = device else { continue };
            let Some(from) = shown.slot_of(device.adapter, device.device) else { continue };
            let (Some(row), Some(old)) = (self.instances.get_mut(slot), shown.instances.get(from)) else { continue };
            for (packed, earlier) in row.iter_mut().zip(old.iter()) {
                let newer = earlier.last().filter(|old| packed.last().is_none_or(|new| later(now_ms, old.heard_ms, new.heard_ms)));
                if packed.known && newer.is_some() {
                    packed.hear(newer);
                }
            }
        }
    }

    pub(crate) fn hear_event(&mut self, event: &InputEvent, heard_ms: u32) -> bool {
        self.hear_event_unless_older(event, heard_ms, None)
    }

    pub(crate) fn replay_event(&mut self, event: &InputEvent, heard_ms: u32, now_ms: u32) -> bool {
        self.hear_event_unless_older(event, heard_ms, Some(now_ms))
    }

    fn hear_event_unless_older(&mut self, event: &InputEvent, heard_ms: u32, replayed_at_ms: Option<u32>) -> bool {
        let address = event.address;
        let (Some(slot), Some(instance)) = (self.slot_of(address.adapter, address.device), address.instance) else {
            return false;
        };
        let Some(packed) = self.instances.get_mut(slot).and_then(|row| row.get_mut(usize::from(instance))) else {
            return false;
        };
        let older = |now_ms: u32| packed.last().is_some_and(|last| !later(now_ms, heard_ms, last.heard_ms));
        if !packed.known || replayed_at_ms.is_some_and(older) {
            return false;
        }
        let value = carries_value(event).then_some(event.value).or_else(|| packed.last().and_then(|last| last.value));
        packed.hear(Some(HeldValue { event_info: event.event_info, value, heard_ms }));
        true
    }

    pub(crate) fn hold_in_reach(&mut self, now_ms: u32) {
        for packed in self.instances.iter_mut().flatten().filter(|packed| packed.heard) {
            packed.heard_ms = in_reach(now_ms, packed.heard_ms);
        }
    }

    pub fn owner(&self) -> Option<&HeldOwner> {
        self.owner.as_ref()
    }

    pub fn device_at(&self, index: usize) -> Option<&HeldDevice> {
        self.devices.iter().flatten().nth(index)
    }

    pub fn device_count(&self) -> u8 {
        u8::try_from(self.devices.iter().flatten().count()).unwrap_or(u8::MAX)
    }

    pub fn instance_at(&self, device_index: usize, index: usize) -> Option<HeldInstance> {
        let slot = self.devices.iter().enumerate().filter(|(_, held)| held.is_some()).nth(device_index)?.0;
        let row = self.instances.get(slot)?;
        let (instance, packed) = row.iter().enumerate().filter(|(_, packed)| packed.known).nth(index)?;
        Some(HeldInstance {
            instance: u8::try_from(instance).ok()?,
            instance_type: unpack_byte(packed.instance_type),
            groups: packed.groups.map(unpack_byte),
            last: packed.last(),
        })
    }

    pub fn same_but_values(&self, other: &Self) -> bool {
        let shape = |packed: &Packed| (packed.known, packed.instance_type, packed.groups);
        let instances = self.instances.iter().flatten().map(shape).eq(other.instances.iter().flatten().map(shape));
        self.owner == other.owner && self.devices == other.devices && instances
    }
}

pub(crate) const fn carries_value(event: &InputEvent) -> bool {
    !matches!(event.kind, InputEventKind::Button | InputEventKind::Generic)
}

fn held_owner(owner: &InputOwner<'_>) -> HeldOwner {
    HeldOwner { controller_id: fixed_text_32(owner.controller_id), discovery_prefix: fixed_text_32(owner.discovery_prefix) }
}
