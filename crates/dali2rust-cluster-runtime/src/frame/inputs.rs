use dali2rust_contracts::msg::{HomeAssistantControllerIdUpdateCommand, HomeAssistantTopicsUpdateCommand, InputEventKind};
use dali2rust_domain::registry::{CLUSTER_INPUT_INSTANCES, INPUT_INSTANCE_GROUPS, MAX_SHORT_ADDRESSES};
use serde::{Deserialize, Serialize};

use super::codec::FrameKind;
use super::limits::{is_adapter, MAX_INSTALLATION_BYTES, MAX_NAME_BYTES};
use super::page::{InRange, Page, PageElements, PageKind, PageWriter};

pub const MAX_INPUT_PREFIX_BYTES: usize = HomeAssistantTopicsUpdateCommand::PREFIX_MAX_BYTES;

pub const MAX_INPUT_SCHEME: u8 = 4;

pub const INPUT_FIELD_MASK: u16 = 0x3FF;

const INSTANCE_TYPES: u8 = 32;

const INSTANCE_GROUPS: u8 = 32;

const fn is_device(device: u8) -> bool {
    (device as usize) < MAX_SHORT_ADDRESSES
}

const fn is_instance(instance: u8) -> bool {
    (instance as usize) < CLUSTER_INPUT_INSTANCES
}

const fn is_type(instance_type: Option<u8>) -> bool {
    match instance_type {
        Some(instance_type) => instance_type < INSTANCE_TYPES,
        None => true,
    }
}

const fn are_groups(groups: &[Option<u8>; INPUT_INSTANCE_GROUPS]) -> bool {
    let mut index = 0;
    while index < INPUT_INSTANCE_GROUPS {
        if let Some(group) = groups[index] {
            if group >= INSTANCE_GROUPS {
                return false;
            }
        }
        index += 1;
    }
    true
}

const fn is_field(field: u16) -> bool {
    field <= INPUT_FIELD_MASK
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputAddress {
    pub adapter: u8,
    pub device: u8,
    pub instance: Option<u8>,
}

impl InputAddress {
    pub(crate) const fn in_range(&self) -> bool {
        let instance = match self.instance {
            Some(instance) => is_instance(instance),
            None => true,
        };
        is_adapter(self.adapter) && is_device(self.device) && instance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputEvent {
    pub address: InputAddress,
    pub instance_type: Option<u8>,
    pub groups: [Option<u8>; INPUT_INSTANCE_GROUPS],
    pub scheme: u8,
    pub kind: InputEventKind,
    pub event_info: u16,
    pub value: u16,
}

impl InputEvent {
    pub(crate) const fn in_range(&self) -> bool {
        let fields = is_field(self.event_info) && is_field(self.value) && self.scheme <= MAX_INPUT_SCHEME;
        self.address.in_range() && is_type(self.instance_type) && are_groups(&self.groups) && fields
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputOwner<'a> {
    pub controller_id: &'a str,
    pub discovery_prefix: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDevice<'a> {
    pub adapter: u8,
    pub device: u8,
    pub name: &'a str,
    pub ha_entity: bool,
    pub present: bool,
    pub instances: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputValue {
    pub event_info: u16,
    pub value: Option<u16>,
    pub age_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputInstance {
    pub adapter: u8,
    pub device: u8,
    pub instance: u8,
    pub instance_type: Option<u8>,
    pub groups: [Option<u8>; INPUT_INSTANCE_GROUPS],
    pub last: Option<InputValue>,
}

impl InputInstance {
    const fn in_range(&self) -> bool {
        let last = match self.last {
            Some(last) => is_field(last.event_info) && matches!(last.value, None | Some(0..=INPUT_FIELD_MASK)),
            None => true,
        };
        let place = is_adapter(self.adapter) && is_device(self.device) && is_instance(self.instance);
        place && is_type(self.instance_type) && are_groups(&self.groups) && last
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputInfoElement<'a> {
    Owner(#[serde(borrow)] InputOwner<'a>),
    Device(#[serde(borrow)] InputDevice<'a>),
    Instance(InputInstance),
}

impl InRange for InputInfoElement<'_> {
    fn in_range(&self) -> bool {
        match self {
            Self::Owner(owner) => {
                owner.controller_id.len() <= MAX_INSTALLATION_BYTES
                    && owner.discovery_prefix.len() <= MAX_INPUT_PREFIX_BYTES
            }
            Self::Device(device) => {
                let instances = usize::from(device.instances) <= CLUSTER_INPUT_INSTANCES;
                is_adapter(device.adapter) && is_device(device.device) && device.name.len() <= MAX_NAME_BYTES && instances
            }
            Self::Instance(instance) => instance.in_range(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputInfoKind;

impl PageKind for InputInfoKind {
    const KIND: FrameKind = FrameKind::InputInfo;
    type Scope = ();
    type Element<'a> = InputInfoElement<'a>;
}

pub type InputInfoPage<'a> = Page<'a, InputInfoKind>;

pub type InputInfoElements<'a> = PageElements<'a, InputInfoKind>;

pub type InputInfoWriter<'b> = PageWriter<'b, InputInfoKind>;

const _: () = assert!(HomeAssistantControllerIdUpdateCommand::CONTROLLER_ID_MAX_BYTES <= MAX_INSTALLATION_BYTES);
