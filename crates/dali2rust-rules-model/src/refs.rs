use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct LampRef {
    pub adapter_id: u8,
    pub id: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct GroupRef {
    pub adapter_id: u8,
    pub id: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DeviceRef {
    pub adapter_id: u8,
    pub short_address: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InputRef {
    pub adapter_id: u8,
    pub device_short_address: u8,
    pub instance_number: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InputDeviceRef {
    pub adapter_id: u8,
    pub device_short_address: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InputGroupSelector {
    pub adapter_id: u8,
    pub instance_group: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_type: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum InputSelector {
    Instance(InputRef),
    Group(InputGroupSelector),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum LightTarget {
    #[serde(rename = "virtual_lamp")]
    Lamp(LampRef),
    Group(GroupRef),
    Broadcast { adapter_id: u8 },
}
