use dali2rust_domain::dali::controller::{DaliApplicationController, Frame24Fault};
use dali2rust_domain::dali::dev103::{
    Button301Command, Device103Address, Device103Command, EventScheme, InitialiseScope103,
    ForwardFrame24, Instance103Command, InstanceAddress, ShortAddressOperand, Special103Command,
    MAX_INSTANCE_INDEX,
    MAX_SHORT_ADDRESS,
};
use dali2rust_domain::dali::pres::DaliResponse;

use crate::runtime::executor::helpers::{
    PREEMPTED_MESSAGE, VERIFY_CONTENDED_MESSAGE, VERIFY_FAILED_MESSAGE, VERIFY_UNANSWERED_MESSAGE,
};
use crate::runtime::executor::SemanticDaliError;

pub const TRANSPORT_UNSUPPORTED_24BIT: &str = "transport_unsupported_24bit";

pub const FRAME24_CONTENDED: &str = "frame24_contended";

const RANDOM_ADDRESS_MAX: u32 = 0x00FF_FFFE;
const RANDOMISE_SETTLE_MS: u64 = 100;

const INSTANCE_STATUS_ACTIVE: u8 = 1 << 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedDevice {
    pub short_address: u8,
    pub instance_count: u8,
    pub presence_unproven: bool,
    pub instances: Vec<(u8, u8)>,
    pub feedback: Vec<super::dev103_feedback::FeedbackProbe>,
    pub declarations: DeviceDeclarations,
    pub instance_facts: Vec<InstanceFacts>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scan103Summary {
    pub devices_found: u16,
    pub instances_found: u16,
    pub addresses_contended: u16,
    pub addresses_reprobed: u16,
}

pub fn supports_frame24(controller: &impl DaliApplicationController) -> bool {
    controller.supports_frame24()
}

pub(super) fn send(
    controller: &mut impl DaliApplicationController,
    frame: ForwardFrame24,
    expects_backward: bool,
) -> Result<DaliResponse, SemanticDaliError> {
    controller
        .send_frame24(frame.as_bytes(), expects_backward)
        .map_err(fault_to_error)
}

pub(super) fn send_once(
    controller: &mut impl DaliApplicationController,
    frame: ForwardFrame24,
    expects_backward: bool,
) -> Result<DaliResponse, SemanticDaliError> {
    controller
        .send_frame24_once(frame.as_bytes(), expects_backward)
        .map_err(fault_to_error)
}

fn fault_to_error(fault: Frame24Fault) -> SemanticDaliError {
    match fault {
        Frame24Fault::Unsupported => SemanticDaliError::OperationFailed(TRANSPORT_UNSUPPORTED_24BIT),
        Frame24Fault::Contended => SemanticDaliError::OperationFailed(FRAME24_CONTENDED),
        Frame24Fault::Preempted => SemanticDaliError::OperationFailed(PREEMPTED_MESSAGE),
    }
}

pub(super) fn send_twice(
    controller: &mut impl DaliApplicationController,
    frame: ForwardFrame24,
) -> Result<(), SemanticDaliError> {
    controller.transaction(|c| {
        send(c, frame, false)?;
        send(c, frame, false)?;
        Ok(())
    })
}

pub(super) fn verify_readback(
    seen: DaliResponse,
    accepts: impl FnOnce(u8) -> bool,
) -> Result<u8, SemanticDaliError> {
    match seen {
        DaliResponse::Answer(actual) if accepts(actual) => Ok(actual),
        DaliResponse::Answer(_) => Err(SemanticDaliError::OperationFailed(VERIFY_FAILED_MESSAGE)),
        DaliResponse::NoAnswer => {
            Err(SemanticDaliError::OperationFailed(VERIFY_UNANSWERED_MESSAGE))
        }
        DaliResponse::Violation => {
            Err(SemanticDaliError::OperationFailed(VERIFY_CONTENDED_MESSAGE))
        }
    }
}

pub(super) fn stage_dtr0(
    controller: &mut impl DaliApplicationController,
    address: Device103Address,
    value: u8,
) -> Result<(), SemanticDaliError> {
    send(controller, Special103Command::Dtr0.frame(value), false)?;
    let readback = send(
        controller,
        Device103Command::QueryContentDtr0.frame(address),
        true,
    )?;
    verify_readback(readback, |seen| seen == value).map(|_| ())
}

pub fn scan_control_devices(
    controller: &mut impl DaliApplicationController,
    on_device: &mut dyn FnMut(&ScannedDevice),
) -> Result<Scan103Summary, SemanticDaliError> {
    let mut summary = Scan103Summary::default();
    for short_address in 0..=MAX_SHORT_ADDRESS {
        let address = Device103Address::Short(short_address);
        let answer = probe_presence(controller, address, &mut summary)?;
        if !answer.is_yes() {
            continue;
        }
        let presence_unproven = matches!(answer, DaliResponse::Violation);
        let instance_count = answer.value().unwrap_or(0);
        let mut contended = false;
        let mut device =
            read_one_device(controller, short_address, instance_count, &mut contended)?;
        device.presence_unproven = presence_unproven;
        if contended {
            summary.addresses_contended = summary.addresses_contended.saturating_add(1);
        }
        summary.devices_found = summary.devices_found.saturating_add(1);
        summary.instances_found = summary
            .instances_found
            .saturating_add(u16::try_from(device.instances.len()).unwrap_or(u16::MAX));
        on_device(&device);
    }
    Ok(summary)
}

fn probe_presence(
    controller: &mut impl DaliApplicationController,
    address: Device103Address,
    summary: &mut Scan103Summary,
) -> Result<DaliResponse, SemanticDaliError> {
    let query = Device103Command::QueryNumberOfInstances.frame(address);
    let first = send(controller, query, true)?;
    if !matches!(first, DaliResponse::Violation) {
        return Ok(first);
    }
    summary.addresses_reprobed = summary.addresses_reprobed.saturating_add(1);
    send(controller, query, true)
}

fn read_one_device(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_count: u8,
    contended: &mut bool,
) -> Result<ScannedDevice, SemanticDaliError> {
    let address = Device103Address::Short(short_address);
    let instances = enumerate_instances(controller, short_address, instance_count)?;
    let mut instance_facts = Vec::with_capacity(instances.len());
    for (number, ty) in &instances {
        instance_facts.push(read_instance_facts(
            controller, address, *number, *ty, contended,
        )?);
    }
    let feedback = instances
        .iter()
        .map(|(number, _)| {
            super::dev103_feedback::probe_feedback(controller, short_address, *number)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let declarations = read_device_facts(controller, address, contended)?;
    Ok(ScannedDevice {
        short_address,
        instance_count,
        presence_unproven: false,
        instances,
        feedback,
        declarations,
        instance_facts,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceDeclarations {
    pub capabilities: Option<u8>,
    pub status: Option<u8>,
    pub version_number: Option<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstanceFacts {
    pub status: Option<u8>,
    pub resolution: Option<u8>,
}

fn read_device_facts(
    controller: &mut impl DaliApplicationController,
    address: Device103Address,
    contended: &mut bool,
) -> Result<DeviceDeclarations, SemanticDaliError> {
    let capabilities = read_declared(
        controller,
        Device103Command::QueryDeviceCapabilities.frame(address),
        contended,
    )?;
    let status = read_declared(
        controller,
        Device103Command::QueryDeviceStatus.frame(address),
        contended,
    )?;
    let version_number = read_declared(
        controller,
        Device103Command::QueryVersionNumber.frame(address),
        contended,
    )?;
    Ok(DeviceDeclarations {
        capabilities,
        status,
        version_number,
    })
}

fn read_declared(
    controller: &mut impl DaliApplicationController,
    frame: ForwardFrame24,
    contended: &mut bool,
) -> Result<Option<u8>, SemanticDaliError> {
    match send(controller, frame, true)? {
        DaliResponse::Answer(value) => Ok(Some(value)),
        DaliResponse::NoAnswer => Ok(None),
        DaliResponse::Violation => {
            *contended = true;
            Ok(None)
        }
    }
}

fn read_instance_facts(
    controller: &mut impl DaliApplicationController,
    address: Device103Address,
    instance_number: u8,
    instance_type: u8,
    contended: &mut bool,
) -> Result<InstanceFacts, SemanticDaliError> {
    let instance = InstanceAddress::Number(instance_number);
    let status = read_declared(
        controller,
        Instance103Command::QueryInstanceStatus.frame(address, instance),
        contended,
    )?;
    let resolution = if instance_type_reports_a_value(instance_type) {
        read_declared(
            controller,
            Instance103Command::QueryResolution.frame(address, instance),
            contended,
        )?
    } else {
        None
    };
    Ok(InstanceFacts { status, resolution })
}

const fn instance_type_reports_a_value(instance_type: u8) -> bool {
    matches!(instance_type, 2 | 4)
}

fn enumerate_instances(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_count: u8,
) -> Result<Vec<(u8, u8)>, SemanticDaliError> {
    let mut found = Vec::new();
    let bound = instance_count.min(MAX_INSTANCE_INDEX.saturating_add(1));
    for instance_number in 0..bound {
        let answer = send(
            controller,
            Instance103Command::QueryInstanceType.frame(
                Device103Address::Short(short_address),
                InstanceAddress::Number(instance_number),
            ),
            true,
        )?;
        if let Some(instance_type) = answer.value() {
            found.push((instance_number, instance_type));
        }
    }
    Ok(found)
}

const BUTTON_TIMERS: [(Button301Command, Button301Command, Option<Button301Command>); 4] = [
    (
        Button301Command::SetShortTimer,
        Button301Command::QueryShortTimer,
        Some(Button301Command::QueryShortTimerMin),
    ),
    (
        Button301Command::SetDoubleTimer,
        Button301Command::QueryDoubleTimer,
        Some(Button301Command::QueryDoubleTimerMin),
    ),
    (
        Button301Command::SetRepeatTimer,
        Button301Command::QueryRepeatTimer,
        None,
    ),
    (
        Button301Command::SetStuckTimer,
        Button301Command::QueryStuckTimer,
        None,
    ),
];

pub fn set_button_timer_verified(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_number: u8,
    slot: usize,
    value: u8,
) -> Result<(), SemanticDaliError> {
    let (set, query, min_query) = BUTTON_TIMERS[slot];
    let address = Device103Address::Short(short_address);
    let instance = InstanceAddress::Number(instance_number);
    let frame = |cmd: Button301Command| {
        ForwardFrame24::command(address, instance, cmd.metadata().opcode)
    };
    let double_off = slot == 1 && value == 0;
    if let (Some(min_query), false) = (min_query, double_off) {
        let min = send(controller, frame(min_query), true)?.value().unwrap_or(0);
        if value < min {
            return Err(SemanticDaliError::OperationFailed("below_device_minimum"));
        }
    }
    controller.transaction(|c| {
        stage_dtr0(c, address, value)?;
        send_twice(c, frame(set))
    })?;
    let seen = send(controller, frame(query), true)?;
    verify_readback(seen, |actual| actual == value).map(|_| ())
}

// IEC 62386-103 §9.6.3
pub fn set_event_scheme_verified(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_number: u8,
    scheme: EventScheme,
) -> Result<(), SemanticDaliError> {
    let address = Device103Address::Short(short_address);
    let instance = InstanceAddress::Number(instance_number);
    controller.transaction(|c| {
        stage_dtr0(c, address, scheme.code())?;
        send_twice(c, Instance103Command::SetEventScheme.frame(address, instance))
    })?;
    let seen = send(
        controller,
        Instance103Command::QueryEventScheme.frame(address, instance),
        true,
    )?;
    verify_readback(seen, |actual| EventScheme::from_code(actual) == Some(scheme)).map(|_| ())
}

pub fn set_event_filter_verified(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_number: u8,
    filter: [u8; 3],
) -> Result<(), SemanticDaliError> {
    let address = Device103Address::Short(short_address);
    let instance = InstanceAddress::Number(instance_number);
    controller.transaction(|c| {
        send(c, Special103Command::Dtr2.frame(filter[2]), false)?;
        send(c, Special103Command::Dtr1.frame(filter[1]), false)?;
        stage_dtr0(c, address, filter[0])?;
        send_twice(c, Instance103Command::SetEventFilter.frame(address, instance))
    })?;
    let seen = send(
        controller,
        Instance103Command::QueryEventFilter0To7.frame(address, instance),
        true,
    )?;
    verify_readback(seen, |actual| actual == filter[0]).map(|_| ())
}

pub fn set_instance_enabled_verified(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_number: u8,
    enabled: bool,
) -> Result<u8, SemanticDaliError> {
    let address = Device103Address::Short(short_address);
    let instance = InstanceAddress::Number(instance_number);
    let set = if enabled {
        Instance103Command::EnableInstance
    } else {
        Instance103Command::DisableInstance
    };
    send_twice(controller, set.frame(address, instance))?;
    let seen = send(
        controller,
        Instance103Command::QueryInstanceStatus.frame(address, instance),
        true,
    )?;
    verify_readback(seen, |status| {
        (status & INSTANCE_STATUS_ACTIVE != 0) == enabled
    })
}

pub fn set_event_priority_verified(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_number: u8,
    priority: u8,
) -> Result<(), SemanticDaliError> {
    let address = Device103Address::Short(short_address);
    let instance = InstanceAddress::Number(instance_number);
    controller.transaction(|c| {
        stage_dtr0(c, address, priority)?;
        send_twice(c, Instance103Command::SetEventPriority.frame(address, instance))
    })?;
    let seen = send(
        controller,
        Instance103Command::QueryEventPriority.frame(address, instance),
        true,
    )?;
    verify_readback(seen, |actual| actual == priority).map(|_| ())
}

pub fn set_instance_group_verified(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_number: u8,
    slot: u8,
    group: Option<u8>,
) -> Result<(), SemanticDaliError> {
    let address = Device103Address::Short(short_address);
    let instance = InstanceAddress::Number(instance_number);
    let (set, query) = match slot {
        0 => (
            Instance103Command::SetPrimaryInstanceGroup,
            Instance103Command::QueryPrimaryInstanceGroup,
        ),
        1 => (
            Instance103Command::SetInstanceGroup1,
            Instance103Command::QueryInstanceGroup1,
        ),
        2 => (
            Instance103Command::SetInstanceGroup2,
            Instance103Command::QueryInstanceGroup2,
        ),
        _ => return Err(SemanticDaliError::OperationFailed("invalid_request")),
    };
    let operand = group.unwrap_or(0xFF);
    controller.transaction(|c| {
        stage_dtr0(c, address, operand)?;
        send_twice(c, set.frame(address, instance))
    })?;
    let seen = send(controller, query.frame(address, instance), true)?;
    verify_readback(seen, |actual| actual == operand).map(|_| ())
}

pub fn identify_device(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
) -> Result<(), SemanticDaliError> {
    send_twice(
        controller,
        Device103Command::IdentifyDevice.frame(Device103Address::Short(short_address)),
    )
}

// IEC 62386-103 §9.9.3
pub fn commission_control_devices(
    controller: &mut impl DaliApplicationController,
    include_addressed: bool,
    on_addressed: &mut dyn FnMut(u8, u32),
) -> Result<u16, SemanticDaliError> {
    let scope = if include_addressed {
        InitialiseScope103::All
    } else {
        InitialiseScope103::Unaddressed
    };
    controller.transaction_exempt(|c| {
        send_twice(c, Device103Command::StartQuiescentMode.frame(Device103Address::Broadcast))?;
        let result = commission_body(c, scope, on_addressed);
        let terminate = send(c, Special103Command::Terminate.frame(0x00), false);
        let unquiesce = send_twice(
            c,
            Device103Command::StopQuiescentMode.frame(Device103Address::Broadcast),
        );
        match (result, terminate, unquiesce) {
            (Err(e), _, _) | (Ok(_), Err(e), _) => Err(e),
            (Ok(_), Ok(_), Err(e)) => Err(e),
            (Ok(count), Ok(_), Ok(())) => Ok(count),
        }
    })
}

fn commission_body(
    controller: &mut impl DaliApplicationController,
    scope: InitialiseScope103,
    on_addressed: &mut dyn FnMut(u8, u32),
) -> Result<u16, SemanticDaliError> {
    send_twice(controller, Special103Command::Initialise.frame(scope.encode()))?;
    send_twice(controller, Special103Command::Randomise.frame(0x00))?;
    // sleep-ok: documented protocol wait (RANDOMISE settling), not a poll
    std::thread::sleep(std::time::Duration::from_millis(RANDOMISE_SETTLE_MS));

    let mut addressed = 0u16;
    let mut next_short = first_free_short_address(controller)?;
    while let Some(random) = find_lowest_random_address(controller)? {
        let Some(short_address) = next_short else {
            break;
        };
        program_short_address(controller, random, short_address)?;
        on_addressed(short_address, random);
        addressed = addressed.saturating_add(1);
        next_short = next_free_short_address(short_address);
    }
    Ok(addressed)
}

fn find_lowest_random_address(
    controller: &mut impl DaliApplicationController,
) -> Result<Option<u32>, SemanticDaliError> {
    set_search_address(controller, RANDOM_ADDRESS_MAX)?;
    if !send(controller, Special103Command::Compare.frame(0x00), true)?.is_yes() {
        return Ok(None);
    }

    let mut low = 0u32;
    let mut high = RANDOM_ADDRESS_MAX;
    while low < high {
        let mid = low + (high - low) / 2;
        set_search_address(controller, mid)?;
        if send(controller, Special103Command::Compare.frame(0x00), true)?.is_yes() {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    set_search_address(controller, low)?;
    Ok(Some(low))
}

fn set_search_address(
    controller: &mut impl DaliApplicationController,
    address: u32,
) -> Result<(), SemanticDaliError> {
    let bytes = address.to_be_bytes();
    send(controller, Special103Command::SearchAddrH.frame(bytes[1]), false)?;
    send(controller, Special103Command::SearchAddrM.frame(bytes[2]), false)?;
    send(controller, Special103Command::SearchAddrL.frame(bytes[3]), false)?;
    Ok(())
}

// IEC 62386-103 §11.10.10
fn program_short_address(
    controller: &mut impl DaliApplicationController,
    random_address: u32,
    short_address: u8,
) -> Result<(), SemanticDaliError> {
    let operand =
        ShortAddressOperand::new(short_address).ok_or(SemanticDaliError::OperationFailed("invalid_request"))?;
    send(
        controller,
        Special103Command::ProgramShortAddress.frame(operand.as_byte()),
        false,
    )?;
    let verified = send(
        controller,
        Special103Command::VerifyShortAddress.frame(operand.as_byte()),
        true,
    )?;
    if !verified.is_yes() {
        return Err(SemanticDaliError::OperationFailed("verify_failed"));
    }
    send(controller, Special103Command::Withdraw.frame(0x00), false)?;
    let _ = random_address;
    Ok(())
}

fn first_free_short_address(
    controller: &mut impl DaliApplicationController,
) -> Result<Option<u8>, SemanticDaliError> {
    for short_address in 0..=MAX_SHORT_ADDRESS {
        let answer = send(
            controller,
            Device103Command::QueryNumberOfInstances.frame(Device103Address::Short(short_address)),
            true,
        )?;
        if !answer.is_yes() {
            return Ok(Some(short_address));
        }
    }
    Ok(None)
}

const fn next_free_short_address(previous: u8) -> Option<u8> {
    if previous < MAX_SHORT_ADDRESS {
        Some(previous + 1)
    } else {
        None
    }
}
