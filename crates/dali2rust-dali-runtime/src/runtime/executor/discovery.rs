use std::collections::BTreeSet;

use dali2rust_contracts::msg::{ColorMode, DeviceType, DeviceTypeSet};
use dali2rust_domain::dali::commands::DaliResponse;
use dali2rust_domain::dali::controller::{DaliApplicationController, ReadbackWorkaround};
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use log::warn;

use crate::runtime::config::ContentConfirmPolicy;
use crate::runtime::executor::commissioning::set_search_address;
use crate::runtime::executor::helpers::{
    dali_short_address, send_dt8_raw_query_stable, send_special, send_special_query,
    send_special_response,
    send_standard_query_once, send_standard_query_stable,
    send_standard_response,
    SemanticDaliError, DT8_DEVICE_TYPE, READ_CONTENDED_MESSAGE,
    DT8_QUERY_COLOUR_STATUS, DT8_QUERY_COLOUR_TYPE_FEATURES,
};
use crate::runtime::executor::majority::majority_byte_from_u32_samples;

const MAX_DEVICE_TYPES: u8 = 16;
const DEVICE_TYPE_NONE: u8 = 254;
const DEVICE_TYPE_MASK: u8 = 255;
const DEVICE_TYPE_ENUM_RERUNS: u8 = 1;
pub const DEVICE_TYPE_ENUM_INCOMPLETE: &str = "device_type_enumeration_incomplete";
const DT8_XY_CAP: u8 = 0x01;
const DT8_TC_CAP: u8 = 0x02;
const DT8_RGBWAF_CHANNELS_MASK: u8 = 0xE0;
const DT8_RGBWAF_CHANNELS_SHIFT: u8 = 5;
const DT8_RGB_CHANNELS: u8 = 3;
const MAX_SHORT_ADDRESS: u8 = 63;
const MAX_RANDOM_ADDRESS: u32 = 0x00FF_FFFF;
const RANDOM_ADDRESS_MAX_SAMPLES: usize = 5;
const DISCOVERY_CONTENT_CONFIRM: ContentConfirmPolicy = ContentConfirmPolicy::new(false, 2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub short_address: u8,
    pub random_address: Option<u32>,
    pub device_type: DeviceType,
    pub color_mode: ColorMode,
    pub dt8_xy_capable: bool,
    pub dt8_tc_capable: bool,
    pub dt8_rgb_capable: bool,
    pub dt8_rgbwaf_capable: bool,
    pub type_enum_degraded: bool,
    pub supported_device_types: Option<DeviceTypeSet>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dt8Capabilities {
    pub xy_capable: bool,
    pub tc_capable: bool,
    pub rgb_capable: bool,
    pub rgbwaf_capable: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dt8Status {
    pub supported: bool,
    pub capabilities: Option<Dt8Capabilities>,
    pub color_mode: ColorMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dt8ProbePolicy {
    DiscoveryScan,
    ExplicitRead,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SupportedDeviceTypesQuery {
    answered: bool,
    supported: BTreeSet<u8>,
    enum_degraded: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub detected: bool,
    pub device_type: DeviceType,
    pub color_mode: ColorMode,
    pub dt8_status: Dt8Status,
    pub type_enum_degraded: bool,
    pub supported_device_types: Option<DeviceTypeSet>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryScanSummary {
    pub described: u16,
    pub error: Option<SemanticDaliError>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryScanReport {
    pub devices: Vec<DiscoveredDevice>,
    pub error: Option<SemanticDaliError>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct VerifiedShortRandomPairs {
    verified_pairs: Vec<(u8, u32)>,
    error: Option<SemanticDaliError>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ReadRandomAddressReport {
    pairs: Vec<(u8, u32)>,
    error: Option<SemanticDaliError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifyPairOutcome {
    Verified,
    Skipped(SemanticDaliError),
}

pub fn detect_device(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
) -> Result<Option<DiscoveredDevice>, SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    let identity = probe_device_identity(controller, address, Dt8ProbePolicy::DiscoveryScan)?;
    if !identity.detected {
        return Ok(None);
    }

    Ok(Some(DiscoveredDevice {
        short_address,
        random_address: None,
        device_type: identity.device_type,
        color_mode: identity.color_mode,
        dt8_xy_capable: identity
            .dt8_status
            .capabilities
            .map(|caps| caps.xy_capable)
            .unwrap_or(false),
        dt8_tc_capable: identity
            .dt8_status
            .capabilities
            .map(|caps| caps.tc_capable)
            .unwrap_or(false),
        dt8_rgb_capable: identity
            .dt8_status
            .capabilities
            .map(|caps| caps.rgb_capable)
            .unwrap_or(false),
        dt8_rgbwaf_capable: identity
            .dt8_status
            .capabilities
            .map(|caps| caps.rgbwaf_capable)
            .unwrap_or(false),
        type_enum_degraded: identity.type_enum_degraded,
        supported_device_types: identity.supported_device_types,
    }))
}

#[cfg(test)]
pub fn discover_known_control_gear(
    controller: &mut impl DaliApplicationController,
) -> Result<DiscoveryScanReport, SemanticDaliError> {
    let mut devices = Vec::new();
    let summary = discover_known_control_gear_with_retry_budget(
        controller,
        crate::runtime::config::DEFAULT_DISCOVERY_STEP_RETRIES,
        &mut |found: &DiscoveredDevice| devices.push(*found),
    )?;
    Ok(DiscoveryScanReport {
        devices,
        error: summary.error,
    })
}

pub fn discover_known_control_gear_with_retry_budget(
    controller: &mut impl DaliApplicationController,
    discovery_step_retries: u8,
    on_device: &mut dyn FnMut(&DiscoveredDevice),
) -> Result<DiscoveryScanSummary, SemanticDaliError> {
    let present_shorts = query_present_control_gear(controller)?;
    let read_report = read_random_addresses(controller, &present_shorts)?;
    let verified =
        verify_short_random_pairs(controller, &read_report.pairs, discovery_step_retries)?;
    Ok(describe_verified_pairs(
        controller,
        verified.verified_pairs.as_slice(),
        read_report.error.or(verified.error),
        on_device,
    ))
}

fn query_present_control_gear(
    controller: &mut impl DaliApplicationController,
) -> Result<Vec<u8>, SemanticDaliError> {
    let mut shorts = Vec::new();
    for short_address in 0..=MAX_SHORT_ADDRESS {
        let address = dali_short_address(short_address)?;
        if send_standard_response(controller, address, StandardCommand::QueryControlGearPresent)?
            .is_yes()
        {
            shorts.push(short_address);
        }
    }
    Ok(shorts)
}

fn read_random_addresses(
    controller: &mut impl DaliApplicationController,
    shorts: &[u8],
) -> Result<ReadRandomAddressReport, SemanticDaliError> {
    let mut report = ReadRandomAddressReport::default();
    for &short_address in shorts {
        match read_random_address_stable(controller, short_address) {
            Ok(random_address) => report.pairs.push((short_address, random_address)),
            Err(error) => record_discovery_skip(&mut report.error, short_address, None, error),
        }
    }
    Ok(report)
}

fn verify_short_random_pairs(
    controller: &mut impl DaliApplicationController,
    short_random_pairs: &[(u8, u32)],
    discovery_step_retries: u8,
) -> Result<VerifiedShortRandomPairs, SemanticDaliError> {
    begin_verify_session(controller)?;
    let result =
        verify_short_random_pairs_body(controller, short_random_pairs, discovery_step_retries);
    let cleanup = end_verify_session(controller);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(report), Ok(())) => Ok(report),
    }
}

fn begin_verify_session(
    controller: &mut impl DaliApplicationController,
) -> Result<(), SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        send_special(controller, SpecialCommand::Terminate)?;
        send_special(controller, SpecialCommand::Initialise(0x00))
    })
}

fn end_verify_session(controller: &mut impl DaliApplicationController) -> Result<(), SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        let _ = set_search_address(controller, MAX_RANDOM_ADDRESS);
        let _ = send_special_query(controller, SpecialCommand::Compare);
        send_special(controller, SpecialCommand::Terminate)
    })
}

fn verify_short_random_pairs_body(
    controller: &mut impl DaliApplicationController,
    short_random_pairs: &[(u8, u32)],
    discovery_step_retries: u8,
) -> Result<VerifiedShortRandomPairs, SemanticDaliError> {
    let mut report = VerifiedShortRandomPairs::default();
    for &(short_address, random_address) in short_random_pairs {
        match verify_short_random_pair(
            controller,
            short_address,
            random_address,
            discovery_step_retries,
        )? {
            VerifyPairOutcome::Verified => report.verified_pairs.push((short_address, random_address)),
            VerifyPairOutcome::Skipped(error) => {
                record_discovery_skip(&mut report.error, short_address, Some(random_address), error)
            }
        }
    }
    Ok(report)
}

fn verify_short_random_pair(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    random_address: u32,
    discovery_step_retries: u8,
) -> Result<VerifyPairOutcome, SemanticDaliError> {
    let attempts = discovery_step_retries.saturating_add(1);
    let mut last_skip = None;
    for attempt in 0..attempts {
        let candidate_random_address = if attempt == 0 {
            random_address
        } else {
            match read_random_address_stable(controller, short_address) {
                Ok(value) => value,
                Err(error) => {
                    last_skip = Some(error);
                    continue;
                }
            }
        };
        match verify_short_random_pair_once(controller, short_address, candidate_random_address) {
            Ok(VerifyPairOutcome::Verified) => return Ok(VerifyPairOutcome::Verified),
            Ok(VerifyPairOutcome::Skipped(error)) | Err(error) => last_skip = Some(error),
        }
    }
    Ok(VerifyPairOutcome::Skipped(last_skip.unwrap_or(
        SemanticDaliError::OperationFailed("bus_contended"),
    )))
}

fn verify_short_random_pair_once(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    random_address: u32,
) -> Result<VerifyPairOutcome, SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        verify_short_random_pair_unit(controller, short_address, random_address)
    })
}

fn verify_short_random_pair_unit(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    random_address: u32,
) -> Result<VerifyPairOutcome, SemanticDaliError> {
    set_search_address(controller, random_address)?;
    let expected = (short_address << 1) | 0x01;
    let reply = match send_special_response(controller, SpecialCommand::QueryShortAddress)? {
        DaliResponse::Answer(reply) => reply,
        DaliResponse::Violation => {
            return Ok(VerifyPairOutcome::Skipped(
                SemanticDaliError::OperationFailed("query_short_address_multiple"),
            ))
        }
        DaliResponse::NoAnswer => {
            return Ok(VerifyPairOutcome::Skipped(
                SemanticDaliError::OperationFailed("query_short_address_no_answer"),
            ))
        }
    };
    if reply != expected {
        return Ok(VerifyPairOutcome::Skipped(
            SemanticDaliError::OperationFailed("query_short_address_mismatch"),
        ));
    }
    send_special(controller, SpecialCommand::Withdraw)?;
    Ok(VerifyPairOutcome::Verified)
}

const RANDOM_ADDRESS_H_NO_ANSWER: &str = "query_random_address_h_no_answer";
const RANDOM_ADDRESS_M_NO_ANSWER: &str = "query_random_address_m_no_answer";
const RANDOM_ADDRESS_L_NO_ANSWER: &str = "query_random_address_l_no_answer";

const RANDOM_ADDRESS_NO_ANSWER: [&str; 3] = [
    RANDOM_ADDRESS_H_NO_ANSWER,
    RANDOM_ADDRESS_M_NO_ANSWER,
    RANDOM_ADDRESS_L_NO_ANSWER,
];

fn is_random_address_no_answer(error: &SemanticDaliError) -> bool {
    matches!(error, SemanticDaliError::OperationFailed(name)
        if RANDOM_ADDRESS_NO_ANSWER.contains(name))
}

fn read_random_address_stable(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
) -> Result<u32, SemanticDaliError> {
    let mut samples = Vec::new();
    let mut unanswered = None;
    for _ in 0..RANDOM_ADDRESS_MAX_SAMPLES {
        match read_random_address_once(controller, short_address) {
            Ok(sample) => {
                samples.push(sample);
                if let Some(random_address) = majority_random_address(&samples) {
                    return Ok(random_address);
                }
            }
            Err(error) if is_random_address_no_answer(&error) => {
                let _ = unanswered.get_or_insert(error);
            }
            Err(error) => return Err(error),
        }
    }
    match unanswered {
        Some(error) if samples.is_empty() => Err(error),
        _ => Err(SemanticDaliError::OperationFailed(
            "query_random_address_unstable",
        )),
    }
}
fn read_random_address_once(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
) -> Result<u32, SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    controller.transaction(|controller| {
        let high = read_random_address_byte(
            controller,
            address,
            StandardCommand::QueryRandomAddressH,
            RANDOM_ADDRESS_H_NO_ANSWER,
        )?;
        let middle = read_random_address_byte(
            controller,
            address,
            StandardCommand::QueryRandomAddressM,
            RANDOM_ADDRESS_M_NO_ANSWER,
        )?;
        let low = read_random_address_byte(
            controller,
            address,
            StandardCommand::QueryRandomAddressL,
            RANDOM_ADDRESS_L_NO_ANSWER,
        )?;
        Ok(pack_random_address(high, middle, low))
    })
}

fn read_random_address_byte(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: StandardCommand,
    unanswered: &'static str,
) -> Result<u8, SemanticDaliError> {
    match send_standard_response(controller, address, command)? {
        DaliResponse::Answer(value) => Ok(value),
        DaliResponse::Violation => Err(SemanticDaliError::OperationFailed(READ_CONTENDED_MESSAGE)),
        DaliResponse::NoAnswer => Err(SemanticDaliError::OperationFailed(unanswered)),
    }
}

fn pack_random_address(high: u8, middle: u8, low: u8) -> u32 {
    (u32::from(high) << 16) | (u32::from(middle) << 8) | u32::from(low)
}

fn majority_random_address(samples: &[u32]) -> Option<u32> {
    Some(pack_random_address(
        majority_random_address_byte(samples, 16)?,
        majority_random_address_byte(samples, 8)?,
        majority_random_address_byte(samples, 0)?,
    ))
}

fn majority_random_address_byte(samples: &[u32], shift: u32) -> Option<u8> {
    majority_byte_from_u32_samples(samples, shift)
}

fn describe_verified_pairs(
    controller: &mut impl DaliApplicationController,
    verified_pairs: &[(u8, u32)],
    error: Option<SemanticDaliError>,
    on_device: &mut dyn FnMut(&DiscoveredDevice),
) -> DiscoveryScanSummary {
    let mut summary = DiscoveryScanSummary { described: 0, error };
    for &(short_address, random_address) in verified_pairs {
        match describe_verified_pair(controller, short_address, random_address) {
            Ok(found) => {
                summary.described = summary.described.saturating_add(1);
                on_device(&found);
            }
            Err(error) => {
                record_discovery_skip(&mut summary.error, short_address, Some(random_address), error)
            }
        }
    }
    summary
}

fn describe_verified_pair(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    random_address: u32,
) -> Result<DiscoveredDevice, SemanticDaliError> {
    let detected = detect_device(controller, short_address)?;
    Ok(detected.unwrap_or(DiscoveredDevice {
        short_address,
        random_address: Some(random_address),
        device_type: DeviceType::Unknown,
        color_mode: ColorMode::Unknown,
        dt8_xy_capable: false,
        dt8_tc_capable: false,
        dt8_rgb_capable: false,
        dt8_rgbwaf_capable: false,
        type_enum_degraded: false,
        supported_device_types: None,
    }))
    .map(|mut found| {
        found.random_address = Some(random_address);
        found
    })
}

fn record_discovery_skip(
    first_error: &mut Option<SemanticDaliError>,
    short_address: u8,
    random_address: Option<u32>,
    error: SemanticDaliError,
) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
    match random_address {
        Some(random_address) => warn!(
            "discovery: skipping short {} random 0x{:06X}: {}",
            short_address,
            random_address,
            error.message()
        ),
        None => warn!(
            "discovery: skipping short {} before random-address verification: {}",
            short_address,
            error.message()
        ),
    }
}

pub fn probe_device_identity(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    dt8_probe_policy: Dt8ProbePolicy,
) -> Result<DeviceIdentity, SemanticDaliError> {
    probe_device_identity_with_policy(
        controller,
        address,
        dt8_probe_policy,
        DISCOVERY_CONTENT_CONFIRM,
    )
}

fn query_types_with_scan_degrade(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    dt8_probe_policy: Dt8ProbePolicy,
    content_confirm: ContentConfirmPolicy,
) -> Result<SupportedDeviceTypesQuery, SemanticDaliError> {
    match query_supported_device_types_with_policy(controller, address, content_confirm) {
        Ok(supported) => Ok(supported),
        Err(error)
            if matches!(dt8_probe_policy, Dt8ProbePolicy::DiscoveryScan)
                && matches!(
                    error,
                    SemanticDaliError::OperationFailed(DEVICE_TYPE_ENUM_INCOMPLETE)
                ) =>
        {
            warn!(
                "discovery: device-type enumeration incomplete at {address:?}; \
                 listing as unknown type"
            );
            Ok(SupportedDeviceTypesQuery {
                answered: true,
                supported: BTreeSet::new(),
                enum_degraded: true,
            })
        }
        Err(error) => Err(error),
    }
}

pub fn probe_device_identity_with_policy(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    dt8_probe_policy: Dt8ProbePolicy,
    content_confirm: ContentConfirmPolicy,
) -> Result<DeviceIdentity, SemanticDaliError> {
    let supported_device_types =
        query_types_with_scan_degrade(controller, address, dt8_probe_policy, content_confirm)?;
    let device_type = classify_supported_device_types(&supported_device_types.supported);
    let allow_dt8_fallback = device_type == DeviceType::Unknown;
    let should_probe_dt8 = match dt8_probe_policy {
        Dt8ProbePolicy::DiscoveryScan => {
            device_type == DeviceType::Dt8Color
                || (allow_dt8_fallback && supported_device_types.answered)
        }
        Dt8ProbePolicy::ExplicitRead => device_type == DeviceType::Dt8Color || allow_dt8_fallback,
    };
    let dt8_status = if should_probe_dt8 {
        query_dt8_status(
            controller,
            address,
            allow_dt8_fallback,
            &supported_device_types.supported,
            content_confirm,
        )?
    } else {
        Dt8Status::default()
    };
    if !supported_device_types.answered && !dt8_status.supported {
        return Ok(DeviceIdentity::default());
    }

    let (device_type, color_mode, declared) = fold_dt8_probe_into_identity(
        device_type,
        &dt8_status,
        declared_device_types(&supported_device_types),
    );
    Ok(DeviceIdentity {
        detected: true,
        device_type,
        color_mode,
        dt8_status,
        type_enum_degraded: supported_device_types.enum_degraded,
        supported_device_types: declared,
    })
}

fn fold_dt8_probe_into_identity(
    device_type: DeviceType,
    dt8_status: &Dt8Status,
    declared: Option<DeviceTypeSet>,
) -> (DeviceType, ColorMode, Option<DeviceTypeSet>) {
    if device_type == DeviceType::Dt8Color {
        return (device_type, dt8_status.color_mode, declared);
    }
    if !dt8_status.supported {
        return (device_type, ColorMode::Brightness, declared);
    }
    let declared = declared.map(|mut set| {
        set.insert(DT8_DEVICE_TYPE);
        set
    });
    (DeviceType::Dt8Color, dt8_status.color_mode, declared)
}

fn declared_device_types(query: &SupportedDeviceTypesQuery) -> Option<DeviceTypeSet> {
    if !query.answered || query.enum_degraded {
        return None;
    }
    let mut declared = DeviceTypeSet::default();
    for device_type in &query.supported {
        if !declared.insert(*device_type) {
            warn!("discovery: device type {device_type} is past the declarable set; not reporting");
            return None;
        }
    }
    Some(declared)
}

pub(crate) fn read_declared_device_types(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<DeviceTypeSet>, SemanticDaliError> {
    let query = query_supported_device_types_with_policy(controller, address, content_confirm)?;
    Ok(declared_device_types(&query))
}

fn query_supported_device_types_with_policy(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<SupportedDeviceTypesQuery, SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        let mut contended = false;
        for _ in 0..=DEVICE_TYPE_ENUM_RERUNS {
            match enumerate_device_types(controller, address, content_confirm)? {
                EnumerationOutcome::Done(supported) => return Ok(supported),
                EnumerationOutcome::Broken { contended: c } => contended |= c,
            }
        }
        Err(SemanticDaliError::OperationFailed(if contended {
            "bus_contended"
        } else {
            DEVICE_TYPE_ENUM_INCOMPLETE
        }))
    })
}

enum EnumerationOutcome {
    Done(SupportedDeviceTypesQuery),
    Broken { contended: bool },
}

fn enumerate_device_types(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<EnumerationOutcome, SemanticDaliError> {
    let mut supported = SupportedDeviceTypesQuery::default();
    let Some(first) = send_standard_query_stable(
        controller,
        content_confirm,
        address,
        StandardCommand::QueryDeviceType,
    )?
    else {
        return Ok(EnumerationOutcome::Done(supported));
    };
    supported.answered = true;
    if first == DEVICE_TYPE_NONE {
        return Ok(EnumerationOutcome::Done(supported));
    }
    if first != DEVICE_TYPE_MASK {
        supported.supported.insert(first);
        return Ok(EnumerationOutcome::Done(supported));
    }
    Ok(match walk_mask_sequence(controller, address)? {
        MaskWalk::Complete(set) => {
            supported.supported = set;
            EnumerationOutcome::Done(supported)
        }
        MaskWalk::Broken { contended } => EnumerationOutcome::Broken { contended },
    })
}

enum MaskWalk {
    Complete(BTreeSet<u8>),
    Broken { contended: bool },
}

// IEC 62386-102 §11.5.13
fn walk_mask_sequence(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
) -> Result<MaskWalk, SemanticDaliError> {
    let mut set = BTreeSet::new();
    for _ in 0..=MAX_DEVICE_TYPES {
        match send_standard_query_once(controller, address, StandardCommand::QueryNextDeviceType) {
            Ok(Some(DEVICE_TYPE_NONE)) => return Ok(MaskWalk::Complete(set)),
            Ok(Some(DEVICE_TYPE_MASK)) | Ok(None) => {
                return Ok(MaskWalk::Broken { contended: false })
            }
            Ok(Some(value)) => {
                if !set.insert(value) {
                    return Ok(MaskWalk::Broken { contended: true });
                }
                if set.len() > usize::from(MAX_DEVICE_TYPES) {
                    return Ok(MaskWalk::Broken { contended: false });
                }
            }
            Err(error) if error.is_preempted() => return Err(error),
            Err(SemanticDaliError::OperationFailed("bus_contended")) => {
                return Ok(MaskWalk::Broken { contended: true })
            }
            Err(error) => return Err(error),
        }
    }
    Ok(MaskWalk::Broken { contended: false })
}

// IEC 62386-102 §9.18, §11.5.12
pub fn declared_device_types_from_single_answer(answer: Option<u8>) -> Option<DeviceTypeSet> {
    let answer = answer?;
    if answer == DEVICE_TYPE_MASK {
        return None;
    }
    let mut declared = DeviceTypeSet::default();
    if answer == DEVICE_TYPE_NONE {
        return Some(declared);
    }
    declared.insert(answer).then_some(declared)
}

fn classify_supported_device_types(supported_device_types: &BTreeSet<u8>) -> DeviceType {
    if supported_device_types.contains(&DT8_DEVICE_TYPE) {
        DeviceType::Dt8Color
    } else if supported_device_types.contains(&6) {
        DeviceType::Dt6Led
    } else {
        DeviceType::Unknown
    }
}

fn query_dt8_capabilities(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<Dt8Capabilities>, SemanticDaliError> {
    let first = query_colour_type_features(controller, address, content_confirm)?;
    let features = if first.is_none_or(|f| f == 0) {
        reread_colour_type_features(controller, address, content_confirm, first)?
    } else {
        first
    };
    Ok(features.map(|features| {
        let channels = (features & DT8_RGBWAF_CHANNELS_MASK) >> DT8_RGBWAF_CHANNELS_SHIFT;
        Dt8Capabilities {
            xy_capable: features & DT8_XY_CAP != 0,
            tc_capable: features & DT8_TC_CAP != 0,
            rgb_capable: channels > 0,
            rgbwaf_capable: channels > DT8_RGB_CHANNELS,
        }
    }))
}

fn reread_colour_type_features(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    first: Option<u8>,
) -> Result<Option<u8>, SemanticDaliError> {
    let reread = query_colour_type_features(controller, address, content_confirm)?;
    if first == Some(0) && reread.is_some_and(|f| f != 0) {
        controller.note_workaround(ReadbackWorkaround::ColourFeaturesCorrected);
    }
    Ok(reread.or(first))
}

fn query_colour_type_features(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<u8>, SemanticDaliError> {
    send_dt8_raw_query_stable(
        controller,
        content_confirm,
        address,
        DT8_QUERY_COLOUR_TYPE_FEATURES,
    )
}

fn query_dt8_status(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    allow_fallback: bool,
    supported_device_types: &BTreeSet<u8>,
    content_confirm: ContentConfirmPolicy,
) -> Result<Dt8Status, SemanticDaliError> {
    let advertised_dt8 = supported_device_types.contains(&DT8_DEVICE_TYPE);
    if !advertised_dt8 && !allow_fallback {
        return Ok(Dt8Status::default());
    }

    let capabilities = query_dt8_capabilities(controller, address, content_confirm)?;
    let color_mode_response = query_dt8_color_mode_response(controller, address, content_confirm)?;
    let supported = advertised_dt8 || capabilities.is_some() || color_mode_response.is_some();
    if !supported {
        return Ok(Dt8Status::default());
    }

    Ok(Dt8Status {
        supported: true,
        capabilities,
        color_mode: infer_dt8_discovered_color_mode(
            capabilities,
            color_mode_response.unwrap_or(ColorMode::Unknown),
        ),
    })
}

pub(crate) fn color_mode_from_status(status: u8) -> ColorMode {
    if status & 0x80 != 0 {
        ColorMode::Rgb
    } else if status & 0x40 != 0 {
        ColorMode::Unknown
    } else if status & 0x20 != 0 {
        ColorMode::Cct
    } else if status & 0x10 != 0 {
        ColorMode::Xy
    } else {
        ColorMode::Unknown
    }
}

fn query_dt8_color_mode_response(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<ColorMode>, SemanticDaliError> {
    Ok(
        send_dt8_raw_query_stable(controller, content_confirm, address, DT8_QUERY_COLOUR_STATUS)?
            .map(color_mode_from_status),
    )
}

fn infer_dt8_discovered_color_mode(
    capabilities: Option<Dt8Capabilities>,
    active_color_mode: ColorMode,
) -> ColorMode {
    if active_color_mode != ColorMode::Unknown {
        return active_color_mode;
    }
    match capabilities.unwrap_or_default() {
        Dt8Capabilities {
            xy_capable: true,
            tc_capable: false,
            rgb_capable: false,
            ..
        } => ColorMode::Xy,
        Dt8Capabilities {
            xy_capable: false,
            tc_capable: true,
            rgb_capable: false,
            ..
        } => ColorMode::Cct,
        Dt8Capabilities {
            xy_capable: false,
            tc_capable: false,
            rgb_capable: true,
            ..
        } => ColorMode::Rgb,
        _ => ColorMode::Unknown,
    }
}

#[cfg(test)]
mod tests;
