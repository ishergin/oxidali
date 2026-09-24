use dali2rust_domain::dali::dev103::{
    feedback_capability, feedback_colour_capability, Device103Address, ForwardFrame24,
    InstanceAddress,
};
use dali2rust_platform::dali::TransferOutcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackDialect {
    DiiaCorrected,
    Ed1,
}

#[derive(Debug, Clone, Copy)]
pub struct FeedbackSpec {
    pub dialect: FeedbackDialect,
    pub capability: u8,
    pub colour_capability: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct InstanceSpec {
    pub instance_type: u8,
    pub feedback: Option<FeedbackSpec>,
}

#[derive(Debug, Clone)]
pub struct InputDeviceSpec {
    pub short_address: Option<u8>,
    pub random_address: u32,
    pub instances: Vec<InstanceSpec>,
}

#[derive(Debug, Clone, Copy)]
struct FeedbackState {
    spec: FeedbackSpec,
    active: bool,
    timing: u8,
    active_brightness: u8,
    active_colour: u8,
    inactive_brightness: u8,
    inactive_colour: u8,
}

impl FeedbackState {
    fn new(spec: FeedbackSpec) -> Self {
        Self {
            spec,
            active: false,
            timing: 255,
            active_brightness: 255,
            active_colour: 63,
            inactive_brightness: 0,
            inactive_colour: 63,
        }
    }
}

#[derive(Debug)]
struct InstanceState {
    instance_type: u8,
    enabled: bool,
    scheme: u8,
    filter: [u8; 3],
    priority: u8,
    groups: [Option<u8>; 3],
    timers: [u8; 4],
    feedback: Option<FeedbackState>,
    occupancy: u8,
}

pub const T_SHORT_MIN_UNITS: u8 = 5;
pub const T_DOUBLE_MIN_UNITS: u8 = 5;

impl InstanceState {
    fn new(spec: &InstanceSpec) -> Self {
        Self {
            instance_type: spec.instance_type,
            enabled: true,
            scheme: 0,
            filter: [0xF4, 0, 0],
            priority: 3,
            groups: [None; 3],
            timers: [25, 0, 8, 20],
            feedback: spec.feedback.map(FeedbackState::new),
            occupancy: 0x00,
        }
    }
}

#[derive(Debug)]
struct DeviceState {
    short_address: Option<u8>,
    random_address: u32,
    initialise: bool,
    withdrawn: bool,
    device_groups: u32,
    instances: Vec<InstanceState>,
}

#[derive(Debug)]
pub struct DeviceFleet {
    devices: Vec<DeviceState>,
    dtr: [u8; 3],
    search: [u8; 3],
    quiescent: bool,
    pending_twice: Option<[u8; 3]>,
}

impl DeviceFleet {
    #[must_use]
    pub fn new(specs: Vec<InputDeviceSpec>) -> Self {
        let devices = specs
            .into_iter()
            .map(|spec| DeviceState {
                short_address: spec.short_address,
                random_address: spec.random_address & 0x00FF_FFFF,
                initialise: false,
                withdrawn: false,
                device_groups: 0,
                instances: spec.instances.iter().map(InstanceState::new).collect(),
            })
            .collect();
        Self {
            devices,
            dtr: [0; 3],
            search: [0xFF; 3],
            quiescent: false,
            pending_twice: None,
        }
    }

    pub fn exchange24(&mut self, bytes: [u8; 3], _expects_backward: bool) -> TransferOutcome {
        let frame = ForwardFrame24::from_bytes(bytes);
        if !frame.is_command() {
            return TransferOutcome::NoAnswer;
        }
        let twice_armed = self.pending_twice == Some(bytes);
        let outcome = if let Some(Device103Address::Special(_)) = frame.address() {
            self.on_special(bytes[1], bytes[2], twice_armed)
        } else {
            self.on_addressed(frame, bytes, twice_armed)
        };
        self.pending_twice = if twice_armed { None } else { Some(bytes) };
        outcome
    }

    fn on_addressed(
        &mut self,
        frame: ForwardFrame24,
        bytes: [u8; 3],
        twice_armed: bool,
    ) -> TransferOutcome {
        let (Some(address), Some(instance)) = (frame.address(), frame.instance()) else {
            return TransferOutcome::NoAnswer;
        };
        let opcode = frame.opcode();
        let dtr = self.dtr;
        let mut answers: Vec<u8> = Vec::new();
        let mut yes_without_value = 0u32;
        for device in self.devices.iter_mut().filter(|d| addressed(d, address)) {
            match device_or_instance(device, instance, opcode, dtr, twice_armed) {
                Reply::Value(v) => answers.push(v),
                Reply::Yes => yes_without_value += 1,
                Reply::Silent => {}
            }
        }
        let _ = bytes;
        match (answers.as_slice(), yes_without_value) {
            ([], 0) => TransferOutcome::NoAnswer,
            ([only], 0) => TransferOutcome::Answer(*only),
            // IEC 62386-101 §8.2.5
            _ => TransferOutcome::CorruptedInWindow,
        }
    }

    fn on_special(&mut self, selector: u8, data: u8, twice_armed: bool) -> TransferOutcome {
        match selector {
            0x30 => self.dtr[0] = data,
            0x31 => self.dtr[1] = data,
            0x32 => self.dtr[2] = data,
            0x05 => self.search[0] = data,
            0x06 => self.search[1] = data,
            0x07 => self.search[2] = data,
            0x00 => self.terminate(),
            // IEC 62386-103 Table 23
            0x01 if twice_armed => self.initialise(data),
            0x02 if twice_armed => {}
            0x03 => return self.compare(),
            0x04 => self.withdraw(),
            0x08 => self.program_short_address(data),
            0x09 => return self.verify_short_address(data),
            0x0A => return self.query_short_address(),
            _ => {}
        }
        TransferOutcome::NoAnswer
    }

    fn terminate(&mut self) {
        for device in &mut self.devices {
            device.initialise = false;
            device.withdrawn = false;
        }
    }

    fn initialise(&mut self, scope: u8) {
        for device in &mut self.devices {
            device.initialise = match scope {
                0x7F => device.short_address.is_none(),
                0xFF => true,
                raw => device.short_address == Some(raw & 0x3F),
            };
            device.withdrawn = false;
        }
    }

    fn compare(&mut self) -> TransferOutcome {
        let search = u32::from(self.search[0]) << 16
            | u32::from(self.search[1]) << 8
            | u32::from(self.search[2]);
        let yes = self
            .devices
            .iter()
            .filter(|d| d.initialise && !d.withdrawn && d.random_address <= search)
            .count();
        match yes {
            0 => TransferOutcome::NoAnswer,
            1 => TransferOutcome::Answer(0xFF),
            _ => TransferOutcome::CorruptedInWindow,
        }
    }

    fn withdraw(&mut self) {
        let search = u32::from(self.search[0]) << 16
            | u32::from(self.search[1]) << 8
            | u32::from(self.search[2]);
        for device in &mut self.devices {
            if device.initialise && device.random_address == search {
                device.withdrawn = true;
            }
        }
    }

    // IEC 62386-103 §11.10.10
    fn program_short_address(&mut self, raw: u8) {
        let search = u32::from(self.search[0]) << 16
            | u32::from(self.search[1]) << 8
            | u32::from(self.search[2]);
        for device in &mut self.devices {
            if device.initialise && !device.withdrawn && device.random_address == search {
                device.short_address = (raw <= 0x3F).then_some(raw & 0x3F);
                if raw > 0x3F {
                    device.short_address = None;
                }
                revert_schemes_needing_address(device);
            }
        }
    }

    fn verify_short_address(&mut self, raw: u8) -> TransferOutcome {
        let yes = self
            .devices
            .iter()
            .filter(|d| d.initialise && d.short_address == Some(raw & 0x3F))
            .count();
        match yes {
            0 => TransferOutcome::NoAnswer,
            1 => TransferOutcome::Answer(0xFF),
            _ => TransferOutcome::CorruptedInWindow,
        }
    }

    fn query_short_address(&mut self) -> TransferOutcome {
        let mut answers = self
            .devices
            .iter()
            .filter(|d| d.initialise && !d.withdrawn)
            .filter_map(|d| d.short_address);
        match (answers.next(), answers.next()) {
            (None, _) => TransferOutcome::NoAnswer,
            (Some(a), None) => TransferOutcome::Answer(a),
            _ => TransferOutcome::CorruptedInWindow,
        }
    }

    pub fn commission_instance(
        &mut self,
        device: usize,
        instance: usize,
        scheme: u8,
        event_filter: u8,
    ) -> bool {
        let Some(dev) = self.devices.get_mut(device) else {
            return false;
        };
        let Some(inst) = dev.instances.get_mut(instance) else {
            return false;
        };
        inst.scheme = scheme;
        inst.filter[0] = event_filter;
        true
    }

    pub fn set_quiescent(&mut self, on: bool) {
        self.quiescent = on;
    }

    pub fn button_edge(&mut self, device: usize, instance: usize, pressed: bool) -> Vec<[u8; 3]> {
        let quiescent = self.quiescent;
        let Some(dev) = self.devices.get_mut(device) else { return Vec::new() };
        let short = dev.short_address;
        let Some(inst) = dev.instances.get_mut(instance) else { return Vec::new() };
        if quiescent || !inst.enabled || inst.instance_type != 1 {
            return Vec::new();
        }
        let code: u16 = if pressed { 0x001 } else { 0x000 };
        let bit = if pressed { 1 << 1 } else { 1 << 0 };
        if inst.filter[0] & bit == 0 {
            return Vec::new();
        }
        encode_event(inst, short, instance as u8, code)
            .map(|f| vec![f])
            .unwrap_or_default()
    }

    pub fn occupancy_transition(
        &mut self,
        device: usize,
        instance: usize,
        input_value: u8,
        event_info: u16,
    ) -> Vec<[u8; 3]> {
        let quiescent = self.quiescent;
        let Some(dev) = self.devices.get_mut(device) else { return Vec::new() };
        let short = dev.short_address;
        let Some(inst) = dev.instances.get_mut(instance) else { return Vec::new() };
        if inst.instance_type != 3 {
            return Vec::new();
        }
        inst.occupancy = input_value;
        if quiescent || !inst.enabled {
            return Vec::new();
        }
        encode_event(inst, short, instance as u8, event_info)
            .map(|f| vec![f])
            .unwrap_or_default()
    }
}

fn addressed(device: &DeviceState, address: Device103Address) -> bool {
    match address {
        Device103Address::Short(short) => device.short_address == Some(short),
        Device103Address::Group(group) => group < 32 && device.device_groups & (1 << group) != 0,
        Device103Address::Broadcast => true,
        Device103Address::BroadcastUnaddressed => device.short_address.is_none(),
        Device103Address::Special(_) => false,
    }
}

enum Reply {
    Value(u8),
    Yes,
    Silent,
}

fn device_or_instance(
    device: &mut DeviceState,
    instance: InstanceAddress,
    opcode: u8,
    dtr: [u8; 3],
    twice_armed: bool,
) -> Reply {
    match instance {
        InstanceAddress::FeatureDevice => Reply::Silent,
        InstanceAddress::Device => on_device_command(device, opcode, dtr),
        InstanceAddress::Number(n) => {
            with_instances(device, |i| i.0 == usize::from(n), opcode, dtr, twice_armed, false)
        }
        InstanceAddress::Group(g) => with_instances(
            device,
            |i| i.1.groups.iter().flatten().any(|x| *x == g),
            opcode,
            dtr,
            twice_armed,
            false,
        ),
        InstanceAddress::Type(t) => {
            with_instances(device, |i| i.1.instance_type == t, opcode, dtr, twice_armed, false)
        }
        InstanceAddress::Broadcast => with_instances(device, |_| true, opcode, dtr, twice_armed, false),
        InstanceAddress::FeatureNumber(n) => {
            with_instances(device, |i| i.0 == usize::from(n), opcode, dtr, twice_armed, true)
        }
        InstanceAddress::FeatureGroup(g) => with_instances(
            device,
            |i| i.1.groups[0] == Some(g) || i.1.groups[1] == Some(g) || i.1.groups[2] == Some(g),
            opcode,
            dtr,
            twice_armed,
            true,
        ),
        InstanceAddress::FeatureType(t) => {
            with_instances(device, |i| i.1.instance_type == t, opcode, dtr, twice_armed, true)
        }
        InstanceAddress::FeatureBroadcast => {
            with_instances(device, |_| true, opcode, dtr, twice_armed, true)
        }
    }
}

fn on_device_command(device: &mut DeviceState, opcode: u8, dtr: [u8; 3]) -> Reply {
    match opcode {
        0x30 => Reply::Value(if device.short_address.is_none() { 1 << 2 } else { 0 }),
        0x34 => Reply::Value(2 << 2),
        0x35 => Reply::Value(u8::try_from(device.instances.len()).unwrap_or(u8::MAX)),
        0x36 => Reply::Value(dtr[0]),
        0x37 => Reply::Value(dtr[1]),
        0x38 => Reply::Value(dtr[2]),
        0x3A if device.short_address.is_none() => Reply::Value(0xFF),
        0x46 => Reply::Value(u8::from(!device.instances.is_empty()) << 1),
        _ => Reply::Silent,
    }
}

fn with_instances(
    device: &mut DeviceState,
    select: impl Fn((usize, &InstanceState)) -> bool,
    opcode: u8,
    dtr: [u8; 3],
    twice_armed: bool,
    feature: bool,
) -> Reply {
    let selected: Vec<usize> = device
        .instances
        .iter()
        .enumerate()
        .filter(|(n, i)| select((*n, i)))
        .map(|(n, _)| n)
        .collect();
    let mut answers: Vec<u8> = Vec::new();
    for n in selected {
        let reply = if feature {
            feedback_command(&mut device.instances, n, opcode, dtr[0], twice_armed)
        } else {
            instance_command(device, n, opcode, dtr[0], twice_armed)
        };
        match reply {
            Reply::Value(v) => answers.push(v),
            Reply::Yes => answers.push(0xFF),
            Reply::Silent => {}
        }
    }
    match answers.as_slice() {
        [] => Reply::Silent,
        [only] => Reply::Value(*only),
        _ => Reply::Yes,
    }
}

fn instance_command(
    device: &mut DeviceState,
    n: usize,
    opcode: u8,
    dtr0: u8,
    twice_armed: bool,
) -> Reply {
    let short = device.short_address;
    let inst = &mut device.instances[n];
    match opcode {
        0x61 if twice_armed && (2..=5).contains(&dtr0) => set(&mut inst.priority, dtr0),
        0x62 if twice_armed => set(&mut inst.enabled, true),
        0x63 if twice_armed => set(&mut inst.enabled, false),
        0x64 if twice_armed => set_group(&mut inst.groups[0], dtr0),
        0x65 if twice_armed => set_group(&mut inst.groups[1], dtr0),
        0x66 if twice_armed => set_group(&mut inst.groups[2], dtr0),
        0x67 if twice_armed && dtr0 <= 4 => {
            inst.scheme = dtr0;
            revert_scheme_if_unsupported(inst, short);
            Reply::Silent
        }
        0x68 if twice_armed => set(&mut inst.filter[0], dtr0),
        0x00 if twice_armed => set_timer(&mut inst.timers[0], dtr0, T_SHORT_MIN_UNITS, 255, false),
        0x01 if twice_armed => set_timer(&mut inst.timers[1], dtr0, T_DOUBLE_MIN_UNITS, 100, true),
        0x02 if twice_armed => set_timer(&mut inst.timers[2], dtr0, 5, 100, false),
        0x03 if twice_armed => set_timer(&mut inst.timers[3], dtr0, 5, 255, false),
        0x80 => Reply::Value(inst.instance_type),
        0x81 if matches!(inst.instance_type, 2 | 4) => Reply::Value(8),
        0x83 => Reply::Value(if inst.enabled { 0x02 } else { 0x00 }),
        0x84 => Reply::Value(inst.priority),
        0x86 => yes_no(inst.enabled),
        0x88 => group_answer(inst.groups[0]),
        0x89 => group_answer(inst.groups[1]),
        0x8A => group_answer(inst.groups[2]),
        0x8B => Reply::Value(inst.scheme),
        0x8C => Reply::Value(inst.occupancy),
        0x90 => Reply::Value(inst.filter[0]),
        0x91 | 0x92 => Reply::Silent,
        0x0A => Reply::Value(inst.timers[0]),
        0x0B => Reply::Value(T_SHORT_MIN_UNITS),
        0x0C => Reply::Value(inst.timers[1]),
        0x0D => Reply::Value(T_DOUBLE_MIN_UNITS),
        0x0E => Reply::Value(inst.timers[2]),
        0x0F => Reply::Value(inst.timers[3]),
        0x24 if inst.instance_type == 3 => set(&mut inst.occupancy, 0x00),
        _ => Reply::Silent,
    }
}

fn set<T>(slot: &mut T, value: T) -> Reply {
    *slot = value;
    Reply::Silent
}

fn set_group(slot: &mut Option<u8>, dtr0: u8) -> Reply {
    *slot = (dtr0 <= 31).then_some(dtr0);
    if dtr0 == 0xFF {
        *slot = None;
    }
    Reply::Silent
}

fn set_timer(slot: &mut u8, dtr0: u8, min: u8, max: u8, zero_ok: bool) -> Reply {
    if (dtr0 == 0 && zero_ok) || (min..=max).contains(&dtr0) {
        *slot = dtr0;
    }
    Reply::Silent
}

fn yes_no(yes: bool) -> Reply {
    if yes {
        Reply::Value(0xFF)
    } else {
        Reply::Silent
    }
}

fn group_answer(group: Option<u8>) -> Reply {
    Reply::Value(group.unwrap_or(0xFF))
}

fn revert_scheme_if_unsupported(inst: &mut InstanceState, short: Option<u8>) {
    let supported = match inst.scheme {
        1 | 2 => short.is_some(),
        4 => inst.groups[0].is_some(),
        _ => true,
    };
    if !supported {
        inst.scheme = 0;
    }
}

// IEC 62386-103 §9.6.3
fn revert_schemes_needing_address(device: &mut DeviceState) {
    let short = device.short_address;
    for inst in &mut device.instances {
        revert_scheme_if_unsupported(inst, short);
    }
}

fn feedback_command(
    instances: &mut [InstanceState],
    n: usize,
    opcode: u8,
    dtr0: u8,
    twice_armed: bool,
) -> Reply {
    let Some(fb) = instances[n].feedback else {
        return Reply::Silent;
    };
    match opcode {
        0x10 => return set_feedback_active(instances, n, true),
        0x11 => return set_feedback_active(instances, n, false),
        0x20..=0x3F
            if fb.spec.dialect == FeedbackDialect::DiiaCorrected
                || !(0x27..=0x2F).contains(&opcode) =>
        {
            let selected = opcode - 0x20;
            let matches = instances[n].groups[0] == Some(selected);
            return set_feedback_active(instances, n, matches);
        }
        _ => {}
    }
    if (0x12..=0x18).contains(&opcode) {
        if twice_armed {
            feedback_set(instances, n, opcode, dtr0);
        }
        return Reply::Silent;
    }
    feedback_query(&instances[n], opcode)
}

fn set_feedback_active(instances: &mut [InstanceState], n: usize, active: bool) -> Reply {
    if let Some(fb) = instances[n].feedback.as_mut() {
        fb.active = active;
    }
    Reply::Silent
}

fn feedback_set(instances: &mut [InstanceState], n: usize, opcode: u8, dtr0: u8) {
    let Some(fb) = instances[n].feedback else { return };
    let cap = fb.spec.capability;
    let colour_ok = cap & feedback_capability::COLOUR != 0 && (1..=63).contains(&dtr0);
    let visible = cap & feedback_capability::VISIBLE != 0;
    let common_brightness = cap & feedback_capability::COMMON_BRIGHTNESS != 0;
    let common_colour = fb.spec.colour_capability & feedback_colour_capability::COMMON_COLOUR != 0;
    let targets: Vec<usize> = match opcode {
        0x13 | 0x15 if common_brightness => all_with_capability(instances, feedback_capability::COMMON_BRIGHTNESS),
        0x14 | 0x16 if common_colour => (0..instances.len()).collect(),
        _ => vec![n],
    };
    for t in targets {
        let Some(fb) = instances[t].feedback.as_mut() else { continue };
        match opcode {
            0x12 => fb.timing = dtr0,
            0x13 if visible => fb.active_brightness = dtr0,
            0x14 if colour_ok => fb.active_colour = dtr0,
            0x15 if visible => fb.inactive_brightness = dtr0,
            0x16 if colour_ok => fb.inactive_colour = dtr0,
            _ => {}
        }
    }
}

fn all_with_capability(instances: &[InstanceState], bit: u8) -> Vec<usize> {
    instances
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            i.feedback
                .as_ref()
                .is_some_and(|fb| fb.spec.capability & bit != 0)
        })
        .map(|(n, _)| n)
        .collect()
}

fn feedback_query(inst: &InstanceState, opcode: u8) -> Reply {
    let Some(fb) = inst.feedback else { return Reply::Silent };
    let base = match fb.spec.dialect {
        FeedbackDialect::DiiaCorrected => 0x40,
        FeedbackDialect::Ed1 => 0x20,
    };
    if fb.spec.dialect == FeedbackDialect::DiiaCorrected && opcode == 0x46 {
        return Reply::Value(fb.spec.colour_capability);
    }
    let visible = fb.spec.capability & feedback_capability::VISIBLE != 0;
    let colour = fb.spec.capability & feedback_capability::COLOUR != 0;
    match opcode.checked_sub(base) {
        Some(0x0F) => Reply::Value(fb.spec.capability),
        Some(0x0E) => yes_no(fb.active),
        Some(0x0D) => Reply::Value(fb.timing),
        Some(0x0C) if visible => Reply::Value(fb.active_brightness),
        Some(0x0B) if colour => Reply::Value(fb.active_colour),
        Some(0x0A) if visible => Reply::Value(fb.inactive_brightness),
        Some(0x09) if colour => Reply::Value(fb.inactive_colour),
        _ => Reply::Silent,
    }
}

fn encode_event(
    inst: &InstanceState,
    short: Option<u8>,
    instance_number: u8,
    info: u16,
) -> Option<[u8; 3]> {
    let info_hi = u8::try_from((info >> 8) & 0x03).unwrap_or(0);
    let info_lo = u8::try_from(info & 0xFF).unwrap_or(0);
    let (b0, b1_top) = match inst.scheme {
        0 => (0x80 | ((inst.instance_type & 0x1F) << 1), 0x80 | ((instance_number & 0x1F) << 2)),
        1 => ((short? & 0x3F) << 1, (inst.instance_type & 0x1F) << 2),
        2 => ((short? & 0x3F) << 1, 0x80 | ((instance_number & 0x1F) << 2)),
        3 => (
            0x80 | ((inst.groups[0].unwrap_or(0) & 0x1F) << 1),
            (inst.instance_type & 0x1F) << 2,
        ),
        4 => (
            0xC0 | ((inst.groups[0]? & 0x1F) << 1),
            (inst.instance_type & 0x1F) << 2,
        ),
        _ => return None,
    };
    Some([b0, b1_top | info_hi, info_lo])
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_domain::dali::dev103::{decode_event, EventScheme, Feedback332Command, FeedbackOpcodeMap};

    fn panel(dialect: FeedbackDialect) -> DeviceFleet {
        DeviceFleet::new(vec![InputDeviceSpec {
            short_address: Some(0),
            random_address: 0x123456,
            instances: vec![
                InstanceSpec {
                    instance_type: 1,
                    feedback: Some(FeedbackSpec {
                        dialect,
                        capability: 0x07,
                        colour_capability: 0x1F,
                    }),
                },
                InstanceSpec {
                    instance_type: 1,
                    feedback: Some(FeedbackSpec {
                        dialect,
                        capability: 0x07,
                        colour_capability: 0x1F,
                    }),
                },
            ],
        }])
    }

    fn frame(cmd: Feedback332Command, feature: InstanceAddress, map: FeedbackOpcodeMap) -> [u8; 3] {
        cmd.frame(Device103Address::Short(0), feature, map).as_bytes()
    }

    #[test]
    fn every_scheme_round_trips_through_the_product_decoder() {
        let mut fleet = DeviceFleet::new(vec![InputDeviceSpec {
            short_address: Some(5),
            random_address: 1,
            instances: vec![InstanceSpec { instance_type: 1, feedback: None }],
        }]);
        fleet.devices[0].instances[0].filter[0] = 0x03;
        for (scheme, expect) in [
            (0u8, EventScheme::Instance),
            (1, EventScheme::Device),
            (2, EventScheme::DeviceInstance),
            (3, EventScheme::DeviceGroup),
            (4, EventScheme::InstanceGroup),
        ] {
            fleet.devices[0].instances[0].scheme = scheme;
            fleet.devices[0].instances[0].groups[0] = Some(9);
            let frames = fleet.button_edge(0, 0, true);
            assert_eq!(frames.len(), 1, "scheme {scheme}");
            let decoded = decode_event(ForwardFrame24::from_bytes(frames[0]))
                .unwrap_or_else(|| panic!("scheme {scheme} must decode"));
            let source = match decoded {
                dali2rust_domain::dali::dev103::InputEvent::Instance { source, info } => {
                    assert_eq!(info, 0x001, "press code");
                    source
                }
                other => panic!("unexpected decode: {other:?}"),
            };
            assert_eq!(source.scheme, expect, "scheme {scheme}");
            if matches!(expect, EventScheme::Device | EventScheme::DeviceInstance) {
                assert_eq!(source.short_address, Some(5));
            }
        }
    }

    #[test]
    fn select_feedback_is_the_radio_button_primitive() {
        let mut fleet = panel(FeedbackDialect::DiiaCorrected);
        fleet.devices[0].instances[0].groups[0] = Some(4);
        fleet.devices[0].instances[0].groups[1] = Some(7);
        fleet.devices[0].instances[1].groups[0] = Some(5);
        fleet.devices[0].instances[1].groups[1] = Some(7);
        fleet.devices[0].instances[0].feedback.as_mut().unwrap().active = true;
        let select = frame(
            Feedback332Command::Select(5),
            InstanceAddress::FeatureGroup(7),
            FeedbackOpcodeMap::DiiaCorrected,
        );
        fleet.exchange24(select, false);
        assert!(!fleet.devices[0].instances[0].feedback.unwrap().active, "option 4 goes dark");
        assert!(fleet.devices[0].instances[1].feedback.unwrap().active, "option 5 lights");
    }

    #[test]
    fn a_dialect_answers_only_its_own_query_map() {
        for (dialect, own, foreign) in [
            (FeedbackDialect::DiiaCorrected, FeedbackOpcodeMap::DiiaCorrected, FeedbackOpcodeMap::Ed1),
            (FeedbackDialect::Ed1, FeedbackOpcodeMap::Ed1, FeedbackOpcodeMap::DiiaCorrected),
        ] {
            let mut fleet = panel(dialect);
            let ask = |fleet: &mut DeviceFleet, map| {
                fleet.exchange24(
                    frame(Feedback332Command::QueryCapability, InstanceAddress::FeatureNumber(0), map),
                    true,
                )
            };
            assert_eq!(ask(&mut fleet, own), TransferOutcome::Answer(0x07));
            assert_eq!(ask(&mut fleet, foreign), TransferOutcome::NoAnswer);
        }
    }

    #[test]
    fn an_out_of_range_colour_write_is_discarded_without_a_word() {
        let mut fleet = panel(FeedbackDialect::DiiaCorrected);
        let map = FeedbackOpcodeMap::DiiaCorrected;
        for bad in [0u8, 64, 255] {
            fleet.exchange24(ForwardFrame24::special(0x30, bad).as_bytes(), false);
            let set = frame(Feedback332Command::SetActiveColour, InstanceAddress::FeatureNumber(0), map);
            fleet.exchange24(set, false);
            fleet.exchange24(set, false);
        }
        let seen = fleet.exchange24(
            frame(Feedback332Command::QueryActiveColour, InstanceAddress::FeatureNumber(0), map),
            true,
        );
        assert_eq!(seen, TransferOutcome::Answer(63), "the Table 4 default survived three bad writes");
    }

    #[test]
    fn losing_the_short_address_reverts_the_scheme_silently() {
        let mut fleet = panel(FeedbackDialect::DiiaCorrected);
        fleet.devices[0].instances[0].scheme = 2;
        let init = ForwardFrame24::special(0x01, 0xFF).as_bytes();
        fleet.exchange24(init, false);
        fleet.exchange24(init, false);
        fleet.exchange24(ForwardFrame24::special(0x05, 0x12).as_bytes(), false);
        fleet.exchange24(ForwardFrame24::special(0x06, 0x34).as_bytes(), false);
        fleet.exchange24(ForwardFrame24::special(0x07, 0x56).as_bytes(), false);
        fleet.exchange24(ForwardFrame24::special(0x08, 0xFF).as_bytes(), false);
        assert_eq!(fleet.devices[0].short_address, None);
        assert_eq!(fleet.devices[0].instances[0].scheme, 0, "scheme 2 cannot outlive its precondition");
    }

    #[test]
    fn a_common_brightness_write_spills_onto_the_siblings() {
        let mut fleet = DeviceFleet::new(vec![InputDeviceSpec {
            short_address: Some(0),
            random_address: 1,
            instances: (0..2)
                .map(|_| InstanceSpec {
                    instance_type: 1,
                    feedback: Some(FeedbackSpec {
                        dialect: FeedbackDialect::DiiaCorrected,
                        capability: 0x47,
                        colour_capability: 0x1F,
                    }),
                })
                .collect(),
        }]);
        let map = FeedbackOpcodeMap::DiiaCorrected;
        fleet.exchange24(ForwardFrame24::special(0x30, 200).as_bytes(), false);
        let set = frame(Feedback332Command::SetActiveBrightness, InstanceAddress::FeatureNumber(0), map);
        fleet.exchange24(set, false);
        fleet.exchange24(set, false);
        assert_eq!(fleet.devices[0].instances[1].feedback.unwrap().active_brightness, 200);
    }
}
