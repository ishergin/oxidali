use core::hash::{Hash, Hasher};
use core::mem::MaybeUninit;
use core::ptr;
use std::collections::hash_map::DefaultHasher;

use dali2rust_bsp::psram::PsramBox;

use dali2rust_contracts::msg::{fixed_text_32, FixedText32};
use dali2rust_domain::registry::{ClusterInputReadPort, InputDeviceView, InputInstanceView, CLUSTER_INPUT_DEVICES};

use crate::frame::{InputDevice, InputInfoElement, InputInstance, InputOwner, InputValue, INPUT_FIELD_MASK};
use crate::presence::elapsed_ms;

use super::set::{DEVICE_SLOTS, INSTANCE_SLOTS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnIdentity {
    pub controller_id: FixedText32,
    pub discovery_prefix: FixedText32,
    pub expose: bool,
}

impl OwnIdentity {
    pub fn new(controller_id: &str, discovery_prefix: &str, expose: bool) -> Self {
        Self { controller_id: fixed_text_32(controller_id), discovery_prefix: fixed_text_32(discovery_prefix), expose }
    }
}

pub struct OwnInputs {
    identity: Option<OwnIdentity>,
    devices: [Option<InputDeviceView>; DEVICE_SLOTS],
    instances: [[Option<InputInstanceView>; INSTANCE_SLOTS]; DEVICE_SLOTS],
}

impl OwnInputs {
    pub(crate) unsafe fn init(place: *mut Self) {
        // SAFETY: the caller hands a place of `OwnInputs`; every field is written once here and nothing is read.
        unsafe {
            ptr::addr_of_mut!((*place).identity).write(None);
            let devices = ptr::addr_of_mut!((*place).devices).cast::<Option<InputDeviceView>>();
            for index in 0..DEVICE_SLOTS {
                devices.add(index).write(None);
            }
            let instances = ptr::addr_of_mut!((*place).instances).cast::<Option<InputInstanceView>>();
            for index in 0..DEVICE_SLOTS * INSTANCE_SLOTS {
                instances.add(index).write(None);
            }
        }
    }

    pub(crate) fn fill(&mut self, identity: OwnIdentity, port: &dyn ClusterInputReadPort) {
        self.identity = Some(identity);
        let mut keys = port.input_device_keys().iter();
        for (device, row) in self.devices.iter_mut().zip(self.instances.iter_mut()) {
            *device = keys.by_ref().find_map(|(adapter, short)| port.input_device_view(adapter, short));
            let found = device.as_ref().map(|view| (view.adapter_id, view.short_address));
            for (index, slot) in row.iter_mut().enumerate() {
                *slot = found.and_then(|(adapter, short)| port.input_instance_view(adapter, short, index));
            }
        }
    }

    pub(crate) fn elements(&self, now_ms: u32) -> impl Iterator<Item = InputInfoElement<'_>> + '_ {
        let owner = self.identity.iter().map(|identity| {
            InputInfoElement::Owner(InputOwner {
                controller_id: identity.controller_id.as_str(),
                discovery_prefix: identity.discovery_prefix.as_str(),
            })
        });
        let expose = self.identity.as_ref().is_some_and(|identity| identity.expose);
        let devices = self.devices.iter().zip(self.instances.iter()).filter_map(move |(device, row)| {
            let device = device.as_ref()?;
            let head = core::iter::once(device_element(device, expose));
            Some(head.chain(row.iter().flatten().filter_map(move |view| instance_element(device, view, now_ms))))
        });
        owner.chain(devices.flatten())
    }
}

fn device_element(view: &InputDeviceView, expose: bool) -> InputInfoElement<'_> {
    InputInfoElement::Device(InputDevice {
        adapter: view.adapter_id,
        device: view.short_address,
        name: view.name.as_str(),
        ha_entity: expose && view.ha_expose,
        present: view.present,
        instances: view.instance_count.min(INSTANCE_SLOTS as u8),
    })
}

fn instance_element(device: &InputDeviceView, view: &InputInstanceView, now_ms: u32) -> Option<InputInfoElement<'static>> {
    let instance = Some(view.instance_number).filter(|number| usize::from(*number) < INSTANCE_SLOTS)?;
    let last = view.last_event_info.zip(view.last_event_mono_ms).map(|(event_info, at_ms)| InputValue {
        event_info: event_info & INPUT_FIELD_MASK,
        value: view.input_value.map(|value| value & INPUT_FIELD_MASK),
        age_ms: elapsed_ms(now_ms, at_ms),
    });
    Some(InputInfoElement::Instance(InputInstance {
        adapter: device.adapter_id,
        device: device.short_address,
        instance,
        instance_type: view.instance_type,
        groups: view.groups,
        last,
    }))
}

pub(crate) fn fingerprint(identity: &OwnIdentity, port: &dyn ClusterInputReadPort) -> u64 {
    let mut hasher = DefaultHasher::new();
    (identity.controller_id.as_str(), identity.discovery_prefix.as_str(), identity.expose).hash(&mut hasher);
    for (adapter, short) in port.input_device_keys().iter().take(CLUSTER_INPUT_DEVICES) {
        let device = port.input_device_view(adapter, short);
        device.as_ref().map(|view| (view.name.as_str(), view.ha_expose, view.present, view.instance_count)).hash(&mut hasher);
        let instances = (0..INSTANCE_SLOTS).map_while(|index| port.input_instance_view(adapter, short, index));
        for view in instances {
            (adapter, short, view.without_value()).hash(&mut hasher);
        }
    }
    hasher.finish()
}

pub(crate) fn placed_own_inputs() -> PsramBox<OwnInputs> {
    // SAFETY: `OwnInputs::init` writes every field of the place once.
    unsafe { PsramBox::new_with(|place: &mut MaybeUninit<OwnInputs>| OwnInputs::init(place.as_mut_ptr())) }
}
