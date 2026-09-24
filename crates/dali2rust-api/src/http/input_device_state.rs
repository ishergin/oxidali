use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct InputDeviceSummaryDto {
    pub adapter_id: u8,
    pub short_address: u8,
    pub name: Option<String>,
    pub present: bool,
    pub instance_count: u8,
    pub first_instance_type: Option<u8>,
    pub ha_expose: bool,
    pub last_seen_ms: Option<u64>,
    pub last_event_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReadValueDto<T> {
    pub value: Option<T>,
    pub read_at_ms: Option<u64>,
}

impl<T> ReadValueDto<T> {
    pub fn new(value: Option<T>, read_at_ms: Option<u64>) -> Self {
        Self { value, read_at_ms }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InstanceDto {
    pub instance_number: u8,
    pub instance_type: Option<u8>,
    pub instance_type_name: Option<&'static str>,
    pub instance_status: Option<u8>,
    pub resolution: Option<u8>,
    pub event_scheme: ReadValueDto<u8>,
    pub event_scheme_confirmed: bool,
    pub event_filter: ReadValueDto<Vec<u8>>,
    pub event_priority: ReadValueDto<u8>,
    pub instance_groups: Vec<ReadValueDto<Option<u8>>>,
    pub timers: Vec<ReadValueDto<u8>>,
    pub manual_config_active: bool,
    pub feedback: FeedbackDto,
    pub runtime: InstanceRuntimeDto,
}

#[derive(Clone, Debug, Serialize)]
pub struct FeedbackDto {
    pub probed: bool,
    pub present: bool,
    pub opcode_map: Option<&'static str>,
    pub capability: Option<u8>,
    pub colour_capability: Option<u8>,
    pub timing: Option<u8>,
    pub active_brightness: Option<u8>,
    pub active_colour: Option<u8>,
    pub inactive_brightness: Option<u8>,
    pub inactive_colour: Option<u8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstanceRuntimeDto {
    pub last_event_info: Option<u16>,
    pub last_event_at_ms: Option<u64>,
    pub event_count: u32,
    pub input_value: Option<u16>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InputDeviceDto {
    #[serde(flatten)]
    pub summary: InputDeviceSummaryDto,
    pub notes: Option<String>,
    pub device_capabilities: Option<u8>,
    pub device_status: Option<u8>,
    pub version_number: Option<u8>,
    pub now_ms: u64,
    pub nvm_settling_until_ms: Option<u64>,
    pub instances: Vec<InstanceDto>,
}

#[must_use]
pub fn instance_type_name(instance_type: u8) -> Option<&'static str> {
    match instance_type {
        0 => Some("generic"),
        1 => Some("push_button"),
        2 => Some("absolute_input"),
        3 => Some("occupancy"),
        4 => Some("light_sensor"),
        _ => None,
    }
}

pub trait InputDeviceHttpState: Send + Sync {
    fn list(&self, adapter_id: u8) -> Vec<InputDeviceSummaryDto>;
    fn detail(&self, adapter_id: u8, short_address: u8) -> Option<InputDeviceDto>;
    fn revision(&self) -> u32;
}
