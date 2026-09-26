use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::net::address::DaliAddress;
use dali2rust_contracts::msg::{CommissioningStep, InitialiseScope};
use dali2rust_domain::dali::pres::command::DaliResponse;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;

use crate::runtime::executor::discovery::{detect_device, DiscoveredDevice};
use crate::runtime::executor::helpers::{
    send_special, send_special_response, send_standard, send_standard_query,
    send_standard_response, SemanticDaliError, RANDOMISE_SETTLE_MS,
};

const MAX_RANDOM_ADDRESS: u32 = 0x00FF_FFFF;
const SHORT_ADDRESS_MASK: u8 = 0x3F;
const SHORT_ADDRESS_SHIFT: u8 = 0x01;
const MAX_SHORT_ADDRESS: u8 = 63;

pub fn begin_unaddressed_commissioning(
    controller: &mut impl DaliApplicationController,
) -> Result<(), SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        send_special(controller, SpecialCommand::Initialise(0xFF))?;
        send_special(controller, SpecialCommand::Randomise)?;
        Ok(())
    })?;
    // sleep-ok: documented protocol wait (RANDOMISE settling), not a poll
    std::thread::sleep(std::time::Duration::from_millis(RANDOMISE_SETTLE_MS));
    Ok(())
}

pub fn terminate_unaddressed_commissioning(
    controller: &mut impl DaliApplicationController,
) -> Result<(), SemanticDaliError> {
    send_special(controller, SpecialCommand::Terminate)
}

pub fn commission_next_unaddressed(
    controller: &mut impl DaliApplicationController,
    used_short_addresses: &[u8],
) -> Result<Option<DiscoveredDevice>, SemanticDaliError> {
    commission_next_unaddressed_once(controller, used_short_addresses, 0)
        .map_err(AttemptFailure::error)
}

enum AttemptFailure {
    SearchRepeatable(SemanticDaliError),
    GearWithdrawn(SemanticDaliError),
}

impl AttemptFailure {
    fn error(self) -> SemanticDaliError {
        match self {
            Self::SearchRepeatable(e) | Self::GearWithdrawn(e) => e,
        }
    }
}

pub fn commission_next_unaddressed_with_retry_budget(
    controller: &mut impl DaliApplicationController,
    used_short_addresses: &[u8],
    discovery_step_retries: u8,
) -> Result<Option<DiscoveredDevice>, SemanticDaliError> {
    let attempts = discovery_step_retries.saturating_add(1);
    for attempt in 0..attempts {
        match commission_next_unaddressed_once(controller, used_short_addresses, discovery_step_retries)
        {
            Ok(result) => return Ok(result),
            Err(AttemptFailure::GearWithdrawn(error)) => return Err(error),
            Err(AttemptFailure::SearchRepeatable(_)) if attempt + 1 < attempts => continue,
            Err(AttemptFailure::SearchRepeatable(_)) => {
                return Err(SemanticDaliError::OperationFailed("bus_contended"))
            }
        }
    }
    Err(SemanticDaliError::OperationFailed("bus_contended"))
}

fn commission_next_unaddressed_once(
    controller: &mut impl DaliApplicationController,
    used_short_addresses: &[u8],
    detect_retries: u8,
) -> Result<Option<DiscoveredDevice>, AttemptFailure> {
    let Some(random_address) = find_lowest_random_address(controller)
        .map_err(AttemptFailure::SearchRepeatable)?
    else {
        return Ok(None);
    };
    let Some(short_address) = next_free_short_address(controller, used_short_addresses)
        .map_err(AttemptFailure::SearchRepeatable)?
    else {
        return Err(AttemptFailure::GearWithdrawn(SemanticDaliError::Conflict(
            "no_free_short_address",
        )));
    };

    set_search_address(controller, random_address).map_err(AttemptFailure::SearchRepeatable)?;
    let encoded_short = encode_program_short_address(short_address);
    send_special(
        controller,
        SpecialCommand::ProgramShortAddress(encoded_short),
    )
    .map_err(AttemptFailure::SearchRepeatable)?;
    if !send_special_response(
        controller,
        SpecialCommand::VerifyShortAddress(encoded_short),
    )
    .map_err(AttemptFailure::SearchRepeatable)?
    .is_yes()
    {
        return Err(AttemptFailure::SearchRepeatable(
            SemanticDaliError::OperationFailed("verify_short_address_failed"),
        ));
    }
    send_special(controller, SpecialCommand::Withdraw).map_err(AttemptFailure::SearchRepeatable)?;

    detect_programmed_device(controller, short_address, detect_retries)
        .map(Some)
        .map_err(AttemptFailure::GearWithdrawn)
}

fn detect_programmed_device(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    detect_retries: u8,
) -> Result<DiscoveredDevice, SemanticDaliError> {
    let attempts = detect_retries.saturating_add(1);
    let mut last = SemanticDaliError::OperationFailed("programmed_device_not_present");
    for _ in 0..attempts {
        match detect_device(controller, short_address) {
            Ok(Some(device)) => return Ok(device),
            Ok(None) => continue,
            Err(error) => last = error,
        }
    }
    Err(last)
}

fn find_lowest_random_address(
    controller: &mut impl DaliApplicationController,
) -> Result<Option<u32>, SemanticDaliError> {
    if !probe_search_address(controller, MAX_RANDOM_ADDRESS)? {
        if any_unaddressed_gear(controller)? {
            return Err(SemanticDaliError::OperationFailed(
                "commissioning_unaddressed_not_reached",
            ));
        }
        return Ok(None);
    }

    let mut low = 0u32;
    let mut high = MAX_RANDOM_ADDRESS;
    while low < high {
        let mid = low + ((high - low) / 2);
        if probe_search_address(controller, mid)? {
            high = mid;
        } else {
            low = mid + 1;
        }
    }

    if !probe_search_address(controller, low)? {
        return Err(SemanticDaliError::OperationFailed(
            "commissioning_search_inconsistent",
        ));
    }
    Ok(Some(low))
}

// IEC 62386-102 §11.5.9
fn any_unaddressed_gear(
    controller: &mut impl DaliApplicationController,
) -> Result<bool, SemanticDaliError> {
    Ok(send_standard_response(
        controller,
        DaliAddress::Broadcast,
        StandardCommand::QueryMissingShortAddress,
    )?
    .is_yes())
}

fn probe_search_address(
    controller: &mut impl DaliApplicationController,
    random_address: u32,
) -> Result<bool, SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        set_search_address(controller, random_address)?;
        compare_yes(controller)
    })
}

// IEC 62386-101 §8.2.5
fn compare_yes(controller: &mut impl DaliApplicationController) -> Result<bool, SemanticDaliError> {
    Ok(send_special_response(controller, SpecialCommand::Compare)?.is_yes())
}

pub(crate) fn set_search_address(
    controller: &mut impl DaliApplicationController,
    random_address: u32,
) -> Result<(), SemanticDaliError> {
    send_special(
        controller,
        SpecialCommand::SearchAddrH(((random_address >> 16) & 0xFF) as u8),
    )?;
    send_special(
        controller,
        SpecialCommand::SearchAddrM(((random_address >> 8) & 0xFF) as u8),
    )?;
    send_special(
        controller,
        SpecialCommand::SearchAddrL((random_address & 0xFF) as u8),
    )
}

fn next_free_short_address(
    controller: &mut impl DaliApplicationController,
    used_short_addresses: &[u8],
) -> Result<Option<u8>, SemanticDaliError> {
    let mut used: u64 = 0;
    for short in used_short_addresses {
        if *short <= MAX_SHORT_ADDRESS {
            used |= 1u64 << short;
        }
    }
    for short in 0..=MAX_SHORT_ADDRESS {
        if used & (1u64 << short) != 0 {
            continue;
        }
        let occupied = send_standard_response(
            controller,
            DaliAddress::Short(short),
            StandardCommand::QueryControlGearPresent,
        )?
        .is_yes();
        if !occupied {
            return Ok(Some(short));
        }
    }
    Ok(None)
}

fn encode_program_short_address(short_address: u8) -> u8 {
    ((short_address & SHORT_ADDRESS_MASK) << SHORT_ADDRESS_SHIFT) | 0x01
}

#[cfg(test)]
mod tests;

// IEC 62386-102 §9.14.3.1, §9.14.3.2
pub fn identify_device(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
) -> Result<(), SemanticDaliError> {
    let address = DaliAddress::short(short_address).map_err(|_| {
        SemanticDaliError::OperationFailed("identify: short address out of range")
    })?;
    send_standard(controller, address, StandardCommand::IdentifyDevice)?;
    Ok(())
}

pub fn change_short_address(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    new_short_address: u8,
    verify: bool,
) -> Result<(), SemanticDaliError> {
    if short_address == new_short_address {
        return Err(SemanticDaliError::OperationFailed(
            "address_change_source_equals_target",
        ));
    }
    let current = DaliAddress::short(short_address)
        .map_err(|_| SemanticDaliError::OperationFailed("address_change: source out of range"))?;
    if new_short_address > MAX_SHORT_ADDRESS {
        return Err(SemanticDaliError::OperationFailed(
            "address_change: target out of range",
        ));
    }

    let encoded = encode_program_short_address(new_short_address);
    set_short_address_proved(controller, current, encoded)?;

    if verify {
        let target = DaliAddress::short(new_short_address)
            .map_err(|_| SemanticDaliError::OperationFailed("address_change: target out of range"))?;
        if send_standard_query(controller, target, StandardCommand::QueryStatus)?.is_none() {
            return Err(SemanticDaliError::OperationFailed("verify_failed"));
        }
    }
    Ok(())
}

const ADDRESS_ARM_RETRIES: u8 = 2;

const ADDRESS_ARM_UNCONFIRMED: &str = "address_arm_unconfirmed";

fn set_short_address_proved(
    controller: &mut impl DaliApplicationController,
    current: DaliAddress,
    encoded: u8,
) -> Result<(), SemanticDaliError> {
    for _ in 0..=ADDRESS_ARM_RETRIES {
        let written = controller.transaction_exempt(|controller| {
            send_special(controller, SpecialCommand::Dtr0(encoded))?;
            let armed =
                send_standard_query(controller, current, StandardCommand::QueryContentDtr0)?;
            if armed != Some(encoded) {
                return Ok(false);
            }
            send_standard(controller, current, StandardCommand::SetShortAddress)?;
            Ok(true)
        })?;
        if written {
            return Ok(());
        }
    }
    Err(SemanticDaliError::OperationFailed(ADDRESS_ARM_UNCONFIRMED))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommissioningStepOutcome {
    pub backward_frame: Option<u8>,
    pub backward_violation: bool,
}

fn initialise_data(scope: InitialiseScope, short_address: Option<u8>) -> Result<u8, SemanticDaliError> {
    match scope {
        InitialiseScope::All => Ok(0x00),
        InitialiseScope::Unaddressed => Ok(0xFF),
        InitialiseScope::Short => short_address
            .filter(|s| *s <= MAX_SHORT_ADDRESS)
            .map(encode_program_short_address)
            .ok_or(SemanticDaliError::OperationFailed(
                "initialise scope=short needs a valid short_address",
            )),
    }
}

fn run_write_step(
    controller: &mut impl DaliApplicationController,
    step: CommissioningStep,
    scope: Option<InitialiseScope>,
    short_address: Option<u8>,
    search_address: Option<u32>,
) -> Result<(), SemanticDaliError> {
    match step {
        CommissioningStep::Initialise => {
            let scope = scope.ok_or(SemanticDaliError::OperationFailed(
                "initialise needs a scope",
            ))?;
            let data = initialise_data(scope, short_address)?;
            send_special(controller, SpecialCommand::Initialise(data))
        }
        CommissioningStep::Randomise => send_special(controller, SpecialCommand::Randomise),
        CommissioningStep::SearchAddress => {
            let addr = search_address.ok_or(SemanticDaliError::OperationFailed(
                "search-address needs a search_address",
            ))?;
            if addr > MAX_RANDOM_ADDRESS {
                return Err(SemanticDaliError::OperationFailed(
                    "search_address does not fit in 24 bits",
                ));
            }
            set_search_address(controller, addr)
        }
        CommissioningStep::ProgramShortAddress => {
            let short = require_short(short_address, "program-short-address")?;
            send_special(
                controller,
                SpecialCommand::ProgramShortAddress(encode_program_short_address(short)),
            )
        }
        CommissioningStep::Withdraw => send_special(controller, SpecialCommand::Withdraw),
        CommissioningStep::Terminate => send_special(controller, SpecialCommand::Terminate),
        CommissioningStep::Compare
        | CommissioningStep::VerifyShortAddress
        | CommissioningStep::QueryShortAddress => Err(SemanticDaliError::OperationFailed(
            "step expects a reply and must not take the write path",
        )),
    }
}

fn require_short(short_address: Option<u8>, step: &'static str) -> Result<u8, SemanticDaliError> {
    match short_address {
        Some(short) if short <= MAX_SHORT_ADDRESS => Ok(short),
        _ => Err(SemanticDaliError::OperationFailed(match step {
            "program-short-address" => "program-short-address needs a short_address",
            _ => "verify-short-address needs a short_address",
        })),
    }
}

fn run_query_step(
    controller: &mut impl DaliApplicationController,
    step: CommissioningStep,
    short_address: Option<u8>,
) -> Result<CommissioningStepOutcome, SemanticDaliError> {
    let command = match step {
        CommissioningStep::Compare => SpecialCommand::Compare,
        CommissioningStep::VerifyShortAddress => SpecialCommand::VerifyShortAddress(
            encode_program_short_address(require_short(short_address, "verify-short-address")?),
        ),
        CommissioningStep::QueryShortAddress => SpecialCommand::QueryShortAddress,
        _ => {
            return Err(SemanticDaliError::OperationFailed(
                "step expects no reply and must not take the query path",
            ))
        }
    };
    Ok(match send_special_response(controller, command)? {
        DaliResponse::Answer(byte) => CommissioningStepOutcome {
            backward_frame: Some(byte),
            backward_violation: false,
        },
        DaliResponse::Violation => CommissioningStepOutcome {
            backward_frame: None,
            backward_violation: true,
        },
        DaliResponse::NoAnswer => CommissioningStepOutcome::default(),
    })
}

pub fn run_commissioning_step(
    controller: &mut impl DaliApplicationController,
    step: CommissioningStep,
    scope: Option<InitialiseScope>,
    short_address: Option<u8>,
    search_address: Option<u32>,
) -> Result<CommissioningStepOutcome, SemanticDaliError> {
    match step {
        CommissioningStep::Compare
        | CommissioningStep::VerifyShortAddress
        | CommissioningStep::QueryShortAddress => {
            run_query_step(controller, step, short_address)
        }
        _ => run_write_step(controller, step, scope, short_address, search_address)
            .map(|()| CommissioningStepOutcome::default()),
    }
}

pub fn replace_device(
    controller: &mut impl DaliApplicationController,
    failed_short_address: u8,
    replacement_short_address: u8,
) -> Result<(), SemanticDaliError> {
    if failed_short_address == replacement_short_address {
        return Err(SemanticDaliError::OperationFailed(
            "replace: failed and replacement are the same address",
        ));
    }
    if detect_device(controller, failed_short_address)?.is_some() {
        return Err(SemanticDaliError::OperationFailed("verify_failed"));
    }
    change_short_address(
        controller,
        replacement_short_address,
        failed_short_address,
        true,
    )
}
