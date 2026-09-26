use std::collections::BTreeSet;

use dali2rust_contracts::msg::{ColorMode, DeviceType, DeviceTypeSet};
use dali2rust_domain::dali::commands::DaliResponse;
use dali2rust_domain::dali::controller::DaliApplicationController;
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
    let mut features = query_colour_type_features(controller, address, content_confirm)?;
    if features.is_none_or(|f| f == 0) {
        features = query_colour_type_features(controller, address, content_confirm)?.or(features);
    }
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
mod tests {
    use super::*;
    use crate::runtime::executor::test_helpers::shared::{
        assert_script_consumed, setup_controller, short_address, short_raw_query_frame,
    };
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_domain::dali::commands::DaliCommand;
    use dali2rust_domain::dali::pres::special::SpecialCommand;
    use dali2rust_contracts::msg::ErrorCode;
    use dali2rust_platform::dali::DaliWireCounters;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn declared(types: &[u8]) -> Option<DeviceTypeSet> {
        let mut set = DeviceTypeSet::default();
        for device_type in types {
            assert!(set.insert(*device_type), "type {device_type} is not declarable");
        }
        Some(set)
    }

    #[test]
    fn an_unfinished_enumeration_declares_nothing_rather_than_an_empty_set() {
        let answered_254 = SupportedDeviceTypesQuery {
            answered: true,
            supported: BTreeSet::new(),
            enum_degraded: false,
        };
        assert_eq!(
            declared_device_types(&answered_254),
            Some(DeviceTypeSet::default())
        );

        let never_answered = SupportedDeviceTypesQuery::default();
        assert_eq!(declared_device_types(&never_answered), None);

        let degraded = SupportedDeviceTypesQuery {
            answered: true,
            supported: BTreeSet::new(),
            enum_degraded: true,
        };
        assert_eq!(declared_device_types(&degraded), None);
    }

    #[test]
    fn one_query_device_type_frame_settles_every_case_but_mask() {
        assert_eq!(
            declared_device_types_from_single_answer(Some(DEVICE_TYPE_NONE)),
            Some(DeviceTypeSet::default())
        );
        assert_eq!(
            declared_device_types_from_single_answer(Some(6)),
            declared(&[6])
        );
        assert_eq!(
            declared_device_types_from_single_answer(Some(DEVICE_TYPE_MASK)),
            None
        );
        assert_eq!(declared_device_types_from_single_answer(None), None);
    }

    fn script_present_short_addresses(mock: &MockDaliTransport, present: &[u8]) {
        for short in 0..=63u8 {
            let response = present.contains(&short).then_some(0xFF);
            mock.expect_forward_frame_with_backward(
                DaliCommand::Standard {
                    address: short_address(short),
                    command: StandardCommand::QueryControlGearPresent,
                }
                .to_forward_frame()
                .raw(),
                response,
            );
        }
    }

    fn script_random_address_once(mock: &MockDaliTransport, short: u8, random_address: u32) {
        for (command, reply) in [
            (
                StandardCommand::QueryRandomAddressH,
                ((random_address >> 16) & 0xFF) as u8,
            ),
            (
                StandardCommand::QueryRandomAddressM,
                ((random_address >> 8) & 0xFF) as u8,
            ),
            (
                StandardCommand::QueryRandomAddressL,
                (random_address & 0xFF) as u8,
            ),
        ] {
            mock.expect_forward_frame_with_backward(
                DaliCommand::Standard {
                    address: short_address(short),
                    command,
                }
                .to_forward_frame()
                .raw(),
                Some(reply),
            );
        }
    }

    fn script_random_address_unanswered_h(mock: &MockDaliTransport, short: u8) {
        script_random_address_query(mock, short, StandardCommand::QueryRandomAddressH, None);
    }

    fn script_random_address_unanswered_m(mock: &MockDaliTransport, short: u8, high: u8) {
        script_random_address_query(
            mock,
            short,
            StandardCommand::QueryRandomAddressH,
            Some(high),
        );
        script_random_address_query(mock, short, StandardCommand::QueryRandomAddressM, None);
    }

    fn script_random_address_query(
        mock: &MockDaliTransport,
        short: u8,
        command: StandardCommand,
        reply: Option<u8>,
    ) {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            reply,
        );
    }

    fn script_random_address_reads(
        mock: &MockDaliTransport,
        short: u8,
        random_addresses: &[u32],
    ) {
        for &random_address in random_addresses {
            script_random_address_once(mock, short, random_address);
        }
    }

    fn script_verify_session_begin(mock: &MockDaliTransport) {
        mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Terminate).to_forward_frame().raw());
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::Initialise(0x00))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::Initialise(0x00))
                .to_forward_frame()
                .raw(),
        );
    }

    fn script_verify_attempt(
        mock: &MockDaliTransport,
        random_address: u32,
        reply: Option<u8>,
    ) {
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::SearchAddrH(((random_address >> 16) & 0xFF) as u8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::SearchAddrM(((random_address >> 8) & 0xFF) as u8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::SearchAddrL((random_address & 0xFF) as u8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            DaliCommand::Special(SpecialCommand::QueryShortAddress)
                .to_forward_frame()
                .raw(),
            reply,
        );
    }

    fn script_verify_session_end(mock: &MockDaliTransport) {
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::SearchAddrH(0xFF))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::SearchAddrM(0xFF))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::SearchAddrL(0xFF))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            DaliCommand::Special(SpecialCommand::Compare)
                .to_forward_frame()
                .raw(),
            None,
        );
        mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Terminate).to_forward_frame().raw());
    }

    fn script_describe_dt6(mock: &MockDaliTransport, short: u8) {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(6),
        );
    }

    #[test]
    fn read_random_address_stable_reconstructs_per_byte_majority() {
        let mock = MockDaliTransport::new();
        let short = 17;
        script_random_address_reads(&mock, short, &[0x5C_1D_C2, 0x5C_1D_1D, 0x5C_5C_C2]);

        let (transport, mut controller) = setup_controller(mock);
        let random_address =
            read_random_address_stable(&mut controller, short).expect("random address majority");

        assert_eq!(random_address, 0x5C_1D_C2);
        assert_script_consumed(&transport);
    }

    #[test]
    fn read_random_address_stable_returns_unstable_without_majority() {
        let mock = MockDaliTransport::new();
        let short = 17;
        script_random_address_reads(
            &mock,
            short,
            &[0x11_22_33, 0x44_55_66, 0x77_88_99, 0xAA_BB_CC, 0xDD_EE_FF],
        );

        let (transport, mut controller) = setup_controller(mock);
        let error = read_random_address_stable(&mut controller, short).expect_err("unstable");

        assert_eq!(
            error,
            SemanticDaliError::OperationFailed("query_random_address_unstable")
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn read_random_address_stable_names_the_byte_when_no_sample_ever_answers() {
        let mock = MockDaliTransport::new();
        let short = 17;
        for _ in 0..RANDOM_ADDRESS_MAX_SAMPLES {
            script_random_address_unanswered_h(&mock, short);
        }

        let (transport, mut controller) = setup_controller(mock);
        let error = read_random_address_stable(&mut controller, short).expect_err("no answer");

        assert_eq!(
            error,
            SemanticDaliError::OperationFailed("query_random_address_h_no_answer")
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn read_random_address_stable_survives_one_lost_byte_on_the_first_sample() {
        let mock = MockDaliTransport::new();
        let short = 17;
        script_random_address_unanswered_m(&mock, short, 0x5C);
        script_random_address_reads(&mock, short, &[0x5C_1D_C2, 0x5C_1D_C2]);

        let (transport, mut controller) = setup_controller(mock);
        let random_address =
            read_random_address_stable(&mut controller, short).expect("later samples answer");

        assert_eq!(random_address, 0x5C_1D_C2);
        assert_script_consumed(&transport);
    }

    #[test]
    fn one_random_address_sample_goes_out_as_a_single_transaction() {
        let mock = MockDaliTransport::new();
        let short = 17;
        script_random_address_once(&mock, short, 0x5C_1D_C2);

        let (transport, mut controller) = setup_controller(mock);
        let counters = Arc::new(DaliWireCounters::default());
        controller.set_wire_counters(Arc::clone(&counters));

        let random_address = read_random_address_once(&mut controller, short).expect("sample");

        assert_eq!(random_address, 0x5C_1D_C2);
        assert_eq!(
            counters.transactions_started.load(Ordering::Relaxed),
            1,
            "three frames are one unit, not three"
        );
        assert_eq!(
            counters.frames_sent_by_priority[0].load(Ordering::Relaxed),
            2,
            "H opens the transaction at its own class; M and L follow at priority 1"
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_contended_random_address_byte_is_terminal_and_not_silence() {
        let mock = MockDaliTransport::new();
        let short = 17;
        script_random_address_query(
            &mock,
            short,
            StandardCommand::QueryRandomAddressH,
            Some(0x5C),
        );
        mock.expect_forward_frame_corrupted_in_window(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryRandomAddressM,
            }
            .to_forward_frame()
            .raw(),
        );

        let (transport, mut controller) = setup_controller(mock);
        let error = read_random_address_stable(&mut controller, short).expect_err("contended");

        assert_eq!(
            error,
            SemanticDaliError::OperationFailed(READ_CONTENDED_MESSAGE)
        );
        assert_eq!(error.code(), ErrorCode::ReadContended);
        assert_script_consumed(&transport);
    }

    #[test]
    fn detect_device_uses_device_type_queries_for_presence() {
        let mock = MockDaliTransport::new();
        let short = 17;
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(6),
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("detect");

        assert_eq!(
            detected,
            Some(DiscoveredDevice {
                short_address: short,
                random_address: None,
                device_type: DeviceType::Dt6Led,
                color_mode: ColorMode::Brightness,
                dt8_xy_capable: false,
                dt8_tc_capable: false,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[6]),
            })
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn detect_device_recognizes_dt8_behind_concrete_first_via_fallback_probe() {
        let mock = MockDaliTransport::new();
        let short = 17;
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(0),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x02),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            Some(0x20),
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("detect");

        assert_eq!(
            detected,
            Some(DiscoveredDevice {
                short_address: short,
                random_address: None,
                device_type: DeviceType::Dt8Color,
                color_mode: ColorMode::Cct,
                dt8_xy_capable: false,
                dt8_tc_capable: true,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[0, 8]),
            })
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn detect_device_recognizes_masked_multi_dt_advertisement() {
        let mock = MockDaliTransport::new();
        let short = 17;
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(255),
        );
        for response in [Some(6), Some(8), Some(254)] {
            mock.expect_forward_frame_with_backward(
                DaliCommand::Standard {
                    address: short_address(short),
                    command: StandardCommand::QueryNextDeviceType,
                }
                .to_forward_frame()
                .raw(),
                response,
            );
        }
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x02),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            Some(0x20),
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("detect");

        assert_eq!(
            detected,
            Some(DiscoveredDevice {
                short_address: short,
                random_address: None,
                device_type: DeviceType::Dt8Color,
                color_mode: ColorMode::Cct,
                dt8_xy_capable: false,
                dt8_tc_capable: true,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[6, 8]),
            })
        );
        assert_script_consumed(&transport);
    }

    fn expect_device_type_query(mock: &MockDaliTransport, short: u8, answer: Option<u8>) {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            answer,
        );
    }

    fn expect_next_device_type(mock: &MockDaliTransport, short: u8, answer: Option<u8>) {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryNextDeviceType,
            }
            .to_forward_frame()
            .raw(),
            answer,
        );
    }

    #[test]
    fn a_broken_mask_walk_is_rerun_once_and_the_full_set_survives() {
        let mock = MockDaliTransport::new();
        let short = 17;
        expect_device_type_query(&mock, short, Some(255));
        expect_next_device_type(&mock, short, Some(6));
        expect_next_device_type(&mock, short, None);
        expect_device_type_query(&mock, short, Some(255));
        for answer in [Some(6), Some(8), Some(254)] {
            expect_next_device_type(&mock, short, answer);
        }
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x02),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            Some(0x20),
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("detect");

        assert_eq!(
            detected.expect("device").device_type,
            DeviceType::Dt8Color,
            "the set from the SECOND, complete walk classifies the device"
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_collision_mid_walk_restarts_the_sequence_instead_of_reasking() {
        let mock = MockDaliTransport::new();
        let short = 17;
        expect_device_type_query(&mock, short, Some(255));
        expect_next_device_type(&mock, short, Some(6));
        mock.expect_forward_frame_collision(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryNextDeviceType,
            }
            .to_forward_frame()
            .raw(),
        );
        expect_device_type_query(&mock, short, Some(255));
        for answer in [Some(6), Some(8), Some(254)] {
            expect_next_device_type(&mock, short, answer);
        }
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x02),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            Some(0x20),
        );

        let (transport, mut controller) = setup_controller(mock);
        let identity = probe_device_identity(
            &mut controller,
            short_address(short),
            Dt8ProbePolicy::ExplicitRead,
        )
        .expect("the restarted walk completes");

        assert_eq!(identity.device_type, DeviceType::Dt8Color);
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_twice_broken_walk_degrades_to_unknown_in_a_scan() {
        let mock = MockDaliTransport::new();
        let short = 17;
        for _ in 0..2 {
            expect_device_type_query(&mock, short, Some(255));
            expect_next_device_type(&mock, short, None);
        }
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            None,
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("scan must not fail");

        let listed = detected.expect("listed");
        assert_eq!(
            listed.device_type,
            DeviceType::Unknown,
            "a gear whose enumeration cannot complete is listed, not lost"
        );
        assert!(
            listed.type_enum_degraded,
            "the degradation must be marked, or the counter stays silent"
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_twice_broken_walk_fails_an_explicit_read_with_the_named_error() {
        let mock = MockDaliTransport::new();
        let short = 17;
        for _ in 0..2 {
            expect_device_type_query(&mock, short, Some(255));
            expect_next_device_type(&mock, short, None);
        }

        let (transport, mut controller) = setup_controller(mock);
        let error = probe_device_identity(
            &mut controller,
            short_address(short),
            Dt8ProbePolicy::ExplicitRead,
        )
        .expect_err("strict path refuses the partial set");

        assert_eq!(
            error,
            SemanticDaliError::OperationFailed(DEVICE_TYPE_ENUM_INCOMPLETE)
        );
        assert_eq!(
            crate::runtime::executor::classify_read_abort(error),
            dali2rust_contracts::msg::AttributeGroupReadOutcome::SequenceIncomplete,
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_sixteen_type_gear_reaches_the_terminator() {
        let mock = MockDaliTransport::new();
        let short = 17;
        expect_device_type_query(&mock, short, Some(255));
        for value in 0..16 {
            expect_next_device_type(&mock, short, Some(value));
        }
        expect_next_device_type(&mock, short, Some(254));
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x02),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            Some(0x20),
        );

        let (transport, mut controller) = setup_controller(mock);
        let identity = probe_device_identity(
            &mut controller,
            short_address(short),
            Dt8ProbePolicy::ExplicitRead,
        )
        .expect("the full-ceiling walk completes");

        assert_eq!(identity.device_type, DeviceType::Dt8Color);
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_runaway_type_list_fails_honestly_after_one_rerun() {
        let mock = MockDaliTransport::new();
        let short = 17;
        for _ in 0..2 {
            expect_device_type_query(&mock, short, Some(255));
            for value in 0..17 {
                expect_next_device_type(&mock, short, Some(value));
            }
        }

        let (transport, mut controller) = setup_controller(mock);
        let error = probe_device_identity(
            &mut controller,
            short_address(short),
            Dt8ProbePolicy::ExplicitRead,
        )
        .expect_err("a runaway list must not commit");

        assert_eq!(
            error,
            SemanticDaliError::OperationFailed(DEVICE_TYPE_ENUM_INCOMPLETE)
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_twice_rearmed_walk_fails_as_bus_contended() {
        let mock = MockDaliTransport::new();
        let short = 17;
        for _ in 0..2 {
            expect_device_type_query(&mock, short, Some(255));
            expect_next_device_type(&mock, short, Some(6));
            expect_next_device_type(&mock, short, Some(6));
        }

        let (transport, mut controller) = setup_controller(mock);
        let error = probe_device_identity(
            &mut controller,
            short_address(short),
            Dt8ProbePolicy::ExplicitRead,
        )
        .expect_err("a re-armed walk must not commit");

        assert_eq!(error, SemanticDaliError::OperationFailed("bus_contended"));
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_transport_error_mid_walk_propagates_instead_of_laundering() {
        let mock = MockDaliTransport::new();
        let short = 17;
        expect_device_type_query(&mock, short, Some(255));
        mock.expect_forward_frame_send_error(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryNextDeviceType,
            }
            .to_forward_frame()
            .raw(),
        );

        let (transport, mut controller) = setup_controller(mock);
        let error = probe_device_identity(
            &mut controller,
            short_address(short),
            Dt8ProbePolicy::ExplicitRead,
        )
        .expect_err("a dead wire is not a walk break");

        assert_eq!(
            error,
            SemanticDaliError::OperationFailed("dali_transport_error")
        );
        assert_eq!(
            crate::runtime::executor::classify_read_abort(error),
            dali2rust_contracts::msg::AttributeGroupReadOutcome::TransportAbort,
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn detect_device_falls_back_to_dt8_queries_when_advertisement_is_missing() {
        let mock = MockDaliTransport::new();
        let short = 17;
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(0),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x02),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            Some(0x20),
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("detect");

        assert_eq!(
            detected,
            Some(DiscoveredDevice {
                short_address: short,
                random_address: None,
                device_type: DeviceType::Dt8Color,
                color_mode: ColorMode::Cct,
                dt8_xy_capable: false,
                dt8_tc_capable: true,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[0, 8]),
            })
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn detect_device_keeps_dt8_probe_when_control_gear_present_would_timeout() {
        let mock = MockDaliTransport::new();
        let short = 17;
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(0),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x62),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            Some(0x80),
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("detect");

        assert_eq!(
            detected,
            Some(DiscoveredDevice {
                short_address: short,
                random_address: None,
                device_type: DeviceType::Dt8Color,
                color_mode: ColorMode::Rgb,
                dt8_xy_capable: false,
                dt8_tc_capable: true,
                dt8_rgb_capable: true,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[0, 8]),
            })
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn detect_device_preserves_dt8_capabilities_when_color_status_times_out() {
        let mock = MockDaliTransport::new();
        let short = 17;
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(0),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x62),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            None,
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("detect");

        assert_eq!(
            detected,
            Some(DiscoveredDevice {
                short_address: short,
                random_address: None,
                device_type: DeviceType::Dt8Color,
                color_mode: ColorMode::Unknown,
                dt8_xy_capable: false,
                dt8_tc_capable: true,
                dt8_rgb_capable: true,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[0, 8]),
            })
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn discover_known_control_gear_verifies_random_address_before_publish() {
        let mock = MockDaliTransport::new();
        script_present_short_addresses(&mock, &[0]);
        script_random_address_reads(&mock, 0, &[0x5C_1D_C2, 0x5C_1D_C2]);
        script_verify_session_begin(&mock);
        script_verify_attempt(&mock, 0x5C_1D_C2, Some(0x01));
        mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
        script_verify_session_end(&mock);
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(0),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(0),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(short_raw_query_frame(short_address(0), 0xF9), Some(0x02));
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(short_raw_query_frame(short_address(0), 0xF8), Some(0x20));

        let (transport, mut controller) = setup_controller(mock);
        let discovered = discover_known_control_gear(&mut controller).expect("discover");

        assert_eq!(
            discovered,
            DiscoveryScanReport {
                devices: vec![DiscoveredDevice {
                    short_address: 0,
                    random_address: Some(0x5C_1D_C2),
                    device_type: DeviceType::Dt8Color,
                    color_mode: ColorMode::Cct,
                    dt8_xy_capable: false,
                    dt8_tc_capable: true,
                    dt8_rgb_capable: false,
                    dt8_rgbwaf_capable: false,
                    type_enum_degraded: false,
                    supported_device_types: declared(&[0, 8]),
                }],
                error: None,
            }
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn discover_known_control_gear_uses_third_random_read_to_resolve_mismatch() {
        let mock = MockDaliTransport::new();
        script_present_short_addresses(&mock, &[0]);
        script_random_address_reads(&mock, 0, &[0x5C_1D_C1, 0x5C_1D_C2, 0x5C_1D_C2]);
        script_verify_session_begin(&mock);
        script_verify_attempt(&mock, 0x5C_1D_C2, Some(0x01));
        mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
        script_verify_session_end(&mock);
        script_describe_dt6(&mock, 0);

        let (transport, mut controller) = setup_controller(mock);
        let discovered = discover_known_control_gear(&mut controller).expect("discover");

        assert_eq!(
            discovered,
            DiscoveryScanReport {
                devices: vec![DiscoveredDevice {
                    short_address: 0,
                    random_address: Some(0x5C_1D_C2),
                    device_type: DeviceType::Dt6Led,
                    color_mode: ColorMode::Brightness,
                    dt8_xy_capable: false,
                    dt8_tc_capable: false,
                    dt8_rgb_capable: false,
                    dt8_rgbwaf_capable: false,
                    type_enum_degraded: false,
                    supported_device_types: declared(&[6]),
                }],
                error: None,
            }
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn discover_known_control_gear_retries_verify_after_short_address_timeout() {
        let mock = MockDaliTransport::new();
        script_present_short_addresses(&mock, &[0]);
        script_random_address_reads(&mock, 0, &[0x5C_1D_C2, 0x5C_1D_C2]);
        script_verify_session_begin(&mock);
        script_verify_attempt(&mock, 0x5C_1D_C2, None);
        script_random_address_reads(&mock, 0, &[0x5C_1D_C2, 0x5C_1D_C2]);
        script_verify_attempt(&mock, 0x5C_1D_C2, Some(0x01));
        mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
        script_verify_session_end(&mock);
        script_describe_dt6(&mock, 0);

        let (transport, mut controller) = setup_controller(mock);
        let discovered = discover_known_control_gear(&mut controller).expect("discover");

        assert_eq!(
            discovered,
            DiscoveryScanReport {
                devices: vec![DiscoveredDevice {
                    short_address: 0,
                    random_address: Some(0x5C_1D_C2),
                    device_type: DeviceType::Dt6Led,
                    color_mode: ColorMode::Brightness,
                    dt8_xy_capable: false,
                    dt8_tc_capable: false,
                    dt8_rgb_capable: false,
                    dt8_rgbwaf_capable: false,
                    type_enum_degraded: false,
                    supported_device_types: declared(&[6]),
                }],
                error: None,
            }
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn discover_known_control_gear_keeps_verified_devices_when_later_verify_fails() {
        let mock = MockDaliTransport::new();
        script_present_short_addresses(&mock, &[0, 1]);
        script_random_address_reads(&mock, 0, &[0x5C_1D_C2, 0x5C_1D_C2]);
        script_random_address_reads(&mock, 1, &[0xC8_E7_31, 0xC8_E7_31]);
        script_verify_session_begin(&mock);
        script_verify_attempt(&mock, 0x5C_1D_C2, Some(0x01));
        mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
        script_verify_attempt(&mock, 0xC8_E7_31, None);
        script_random_address_reads(&mock, 1, &[0xC8_E7_31, 0xC8_E7_31]);
        script_verify_attempt(&mock, 0xC8_E7_31, None);
        script_random_address_reads(&mock, 1, &[0xC8_E7_31, 0xC8_E7_31]);
        script_verify_attempt(&mock, 0xC8_E7_31, None);
        script_verify_session_end(&mock);
        script_describe_dt6(&mock, 0);

        let (transport, mut controller) = setup_controller(mock);
        let discovered = discover_known_control_gear(&mut controller).expect("discover");

        assert_eq!(
            discovered,
            DiscoveryScanReport {
                devices: vec![DiscoveredDevice {
                    short_address: 0,
                    random_address: Some(0x5C_1D_C2),
                    device_type: DeviceType::Dt6Led,
                    color_mode: ColorMode::Brightness,
                    dt8_xy_capable: false,
                    dt8_tc_capable: false,
                    dt8_rgb_capable: false,
                    dt8_rgbwaf_capable: false,
                    type_enum_degraded: false,
                    supported_device_types: declared(&[6]),
                }],
                error: Some(SemanticDaliError::OperationFailed(
                    "query_short_address_no_answer",
                )),
            }
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn detect_device_rereads_dt8_features_when_first_read_is_empty() {
        let mock = MockDaliTransport::new();
        let short = 17;
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryDeviceType,
            }
            .to_forward_frame()
            .raw(),
            Some(0),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x00),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF9),
            Some(0x62),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            short_raw_query_frame(short_address(short), 0xF8),
            Some(0x80),
        );

        let (transport, mut controller) = setup_controller(mock);
        let detected = detect_device(&mut controller, short).expect("detect");

        let d = detected.expect("device detected");
        assert!(d.dt8_tc_capable, "cct capability recovered by the re-read");
        assert!(d.dt8_rgb_capable);
        assert!(!d.dt8_xy_capable);
        assert_script_consumed(&transport);
    }
}
