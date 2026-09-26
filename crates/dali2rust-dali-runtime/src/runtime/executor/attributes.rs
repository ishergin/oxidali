use dali2rust_contracts::msg::{
    AttributeGroupReadOutcome, ColorMode, DaliAttributeGroup, DeviceTypeSet, Dt6ReadSnapshot,
    ExtendedVersionEntry, LightSetpoint, PowerState, RuntimeObservation, StatusFlags,
    MAX_EXTENDED_VERSIONS,
};
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::device::ACTUAL_LEVEL_MASK;
use dali2rust_domain::dali::devices::dt6_led::{decode_failure_status, Dt6Command};
use dali2rust_domain::dali::devices::DeviceType;
use dali2rust_domain::dali::devices::dt8_color::{
    colour_value_is_wide, Dt8Command, TC_LIMIT_SELECTOR_COOLEST, TC_LIMIT_SELECTOR_WARMEST,
};
use dali2rust_domain::dali::pres::extended::ExtendedCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_domain::lighting::service::FadeParams;

use crate::runtime::config::ContentConfirmPolicy;
use crate::runtime::executor::discovery::{
    color_mode_from_status, declared_device_types_from_single_answer,
    probe_device_identity_with_policy, Dt8Capabilities, Dt8ProbePolicy, Dt8Status,
};
use crate::runtime::executor::helpers::{
    send_mandatory_query, send_mandatory_query_stable, MandatorySilenceBreaker,
    dali_short_address, fade_time_dtr0_from_ms, fade_time_ms_from_dtr0, send_dtr0_backed_extended,
    send_dtr0_backed_standard,
    confirm_observed_query, send_extended_command, send_extended_query_stable,
    send_raw_query_once_observed,
    send_special, send_standard_query, send_standard_query_stable, send_standard_response,
    SemanticDaliError, DEVICE_ABSENT_MESSAGE, DT8_COLOUR_VALUE_AMBER, DT8_COLOUR_VALUE_BLUE,
    DT8_COLOUR_VALUE_FREECOLOUR, DT8_COLOUR_VALUE_GREEN, DT8_COLOUR_VALUE_WHITE,
    DT8_COLOUR_VALUE_LEVEL_MASK, DT8_COLOUR_VALUE_MASK, DT8_COLOUR_VALUE_RED, DT8_COLOUR_VALUE_TC,
    DT8_COLOUR_VALUE_TC_COOLEST,
    DT8_COLOUR_VALUE_TC_PHYSICAL_COOLEST, DT8_COLOUR_VALUE_TC_PHYSICAL_WARMEST,
    DT8_COLOUR_VALUE_TC_WARMEST, DT8_COLOUR_VALUE_X, DT8_COLOUR_VALUE_Y, PROGRAM_VERIFY_REPAIRS,
    DT8_COLOUR_TYPE_RGBWAF, DT8_COLOUR_TYPE_TC, DT8_COLOUR_TYPE_XY,
    DT8_COLOUR_VALUE_REPORT_COLOUR_TYPE, DT8_COLOUR_VALUE_REPORT_RED,
    DT8_COLOUR_VALUE_REPORT_TC, DT8_COLOUR_VALUE_REPORT_X, DT8_COLOUR_VALUE_REPORT_Y,
    VERIFY_UNANSWERED_MESSAGE,
};

mod common_102;
mod dt6;
mod dt8;

pub use common_102::{Common102Fields, read_group_membership_mask};
use common_102::{
    read_common_102_attributes, read_groups_membership, read_runtime_setpoint,
    read_scene_levels, RuntimeSectionRead,
};
#[cfg(test)]
use common_102::read_light_source_type;
use dt6::{read_dt6_failure_byte, read_dt6_led_snapshot, write_dimming_curve};
pub use dt8::Dt8ColorFields;
use dt8::{
    dt8_narrow_value_sample, read_dt8_color_value_u16, read_dt8_color_value_u8,
    read_dt8_color_values, write_tc_limits,
};
#[cfg(test)]
use dt8::read_dt8_dim_level_trio;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfirmedWritableAttributes {
    pub fade_time_ms: Option<u32>,
    pub fade_rate: Option<u8>,
    pub power_on_level: Option<u8>,
    pub system_failure_level: Option<u8>,
    pub extended_fade_time_ms: Option<u16>,
    pub tc_coolest_mirek: Option<u16>,
    pub tc_warmest_mirek: Option<u16>,
    pub min_level: Option<u8>,
    pub max_level: Option<u8>,
    pub dimming_curve: Option<u8>,
    pub physical_minimum_readback: Option<u8>,
}

impl ConfirmedWritableAttributes {
    pub const fn is_empty(&self) -> bool {
        self.fade_time_ms.is_none()
            && self.fade_rate.is_none()
            && self.power_on_level.is_none()
            && self.system_failure_level.is_none()
            && self.extended_fade_time_ms.is_none()
            && self.tc_coolest_mirek.is_none()
            && self.tc_warmest_mirek.is_none()
            && self.min_level.is_none()
            && self.max_level.is_none()
            && self.dimming_curve.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAttributesExecution {
    pub confirmed: ConfirmedWritableAttributes,
    pub error: Option<SemanticDaliError>,
}

// IEC 62386-102 §3.13
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadBack {
    Proved(u8),
    Refused,
    Unanswered,
}

#[derive(Debug, Default)]
struct WriteTally {
    confirmed: ConfirmedWritableAttributes,
    unanswered: bool,
}

impl WriteTally {
    fn proved(&mut self, read_back: ReadBack) -> Option<u8> {
        match read_back {
            ReadBack::Proved(value) => Some(value),
            ReadBack::Refused => None,
            ReadBack::Unanswered => {
                self.unanswered = true;
                None
            }
        }
    }

    fn into_execution(self, error: Option<SemanticDaliError>) -> WriteAttributesExecution {
        let unanswered = SemanticDaliError::OperationFailed(VERIFY_UNANSWERED_MESSAGE);
        WriteAttributesExecution {
            confirmed: self.confirmed,
            error: error.or(self.unanswered.then_some(unanswered)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeReadSection {
    Identity,
    RuntimeStatus,
    Common102,
    Dt8Color,
    Dt6Led,
    Groups,
    Scenes,
    Extended,
    MemoryBanks,
    SceneColours,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeReadOutcomes {
    pub identity: AttributeGroupReadOutcome,
    pub runtime_status: AttributeGroupReadOutcome,
    pub common_102: AttributeGroupReadOutcome,
    pub dt8_color: AttributeGroupReadOutcome,
    pub dt6_led: AttributeGroupReadOutcome,
    pub groups: AttributeGroupReadOutcome,
    pub scenes: AttributeGroupReadOutcome,
    pub extended: AttributeGroupReadOutcome,
    pub memory_banks: AttributeGroupReadOutcome,
    pub scene_colours: AttributeGroupReadOutcome,
}

impl AttributeReadOutcomes {
    pub fn for_request(groups: &[DaliAttributeGroup], memory_banks_requested: bool) -> Self {
        let requested = |group: DaliAttributeGroup| {
            if wants_attr_group(groups, group) {
                AttributeGroupReadOutcome::NotAttempted
            } else {
                AttributeGroupReadOutcome::NotRequested
            }
        };
        Self {
            identity: AttributeGroupReadOutcome::NotAttempted,
            runtime_status: requested(DaliAttributeGroup::RuntimeStatus),
            common_102: requested(DaliAttributeGroup::Common102),
            dt8_color: requested(DaliAttributeGroup::Dt8Color),
            dt6_led: requested(DaliAttributeGroup::Dt6Led),
            groups: requested(DaliAttributeGroup::Groups),
            scenes: requested(DaliAttributeGroup::Scenes),
            extended: requested(DaliAttributeGroup::Extended),
            scene_colours: requested(DaliAttributeGroup::SceneColours),
            memory_banks: if memory_banks_requested {
                AttributeGroupReadOutcome::NotAttempted
            } else {
                AttributeGroupReadOutcome::NotRequested
            },
        }
    }

    pub fn record(&mut self, section: AttributeReadSection, outcome: AttributeGroupReadOutcome) {
        let slot = match section {
            AttributeReadSection::Identity => &mut self.identity,
            AttributeReadSection::RuntimeStatus => &mut self.runtime_status,
            AttributeReadSection::Common102 => &mut self.common_102,
            AttributeReadSection::Dt8Color => &mut self.dt8_color,
            AttributeReadSection::Dt6Led => &mut self.dt6_led,
            AttributeReadSection::Groups => &mut self.groups,
            AttributeReadSection::Scenes => &mut self.scenes,
            AttributeReadSection::Extended => &mut self.extended,
            AttributeReadSection::MemoryBanks => &mut self.memory_banks,
            AttributeReadSection::SceneColours => &mut self.scene_colours,
        };
        *slot = outcome;
    }
}

pub fn classify_read_abort(error: SemanticDaliError) -> AttributeGroupReadOutcome {
    if error.is_preempted() {
        return AttributeGroupReadOutcome::Preempted;
    }
    match error {
        SemanticDaliError::OperationFailed("bus_contended") => {
            AttributeGroupReadOutcome::ContendedAbort
        }
        SemanticDaliError::OperationFailed(
            crate::runtime::executor::discovery::DEVICE_TYPE_ENUM_INCOMPLETE,
        ) => AttributeGroupReadOutcome::SequenceIncomplete,
        _ if error.is_device_absent() => AttributeGroupReadOutcome::DeviceAbsent,
        _ => AttributeGroupReadOutcome::TransportAbort,
    }
}

fn track<T>(
    outcomes: &mut AttributeReadOutcomes,
    section: AttributeReadSection,
    run: impl FnOnce() -> Result<T, SemanticDaliError>,
) -> Result<T, SemanticDaliError> {
    match run() {
        Ok(value) => {
            outcomes.record(section, AttributeGroupReadOutcome::Success);
            Ok(value)
        }
        Err(error) => {
            outcomes.record(section, classify_read_abort(error));
            Err(error)
        }
    }
}

fn track_gated<T>(
    gate: bool,
    outcomes: &mut AttributeReadOutcomes,
    section: AttributeReadSection,
    run: impl FnOnce() -> Result<T, SemanticDaliError>,
) -> Result<Option<T>, SemanticDaliError> {
    if !gate {
        return Ok(None);
    }
    track(outcomes, section, run).map(Some)
}

#[derive(Debug, Clone)]
pub struct AttributeReadExecution {
    pub runtime_setpoint: Option<LightSetpoint>,
    pub runtime_observation: Option<RuntimeObservation>,
    pub random_address: Option<u32>,
    pub has_common_102: bool,
    pub has_dt8_color: bool,
    pub dt8_supported: Option<bool>,
    pub has_groups: bool,
    pub has_scenes: bool,
    pub has_dt6_led: bool,
    pub has_extended: bool,
    pub dt8_color_mode: ColorMode,
    pub dt8_xy_capable: bool,
    pub dt8_tc_capable: bool,
    pub dt8_rgb_capable: bool,
    pub c102: Common102Fields,
    pub groups_membership: Option<u16>,
    pub scene_levels: Option<Vec<u8>>,
    pub dt8_color: Dt8ColorFields,
    pub dt8_rgbwaf_capable: bool,
    pub dt6: Option<Dt6ReadSnapshot>,
    pub extended_fade_time_ms: Option<u16>,
    pub extended_version_number: Option<u8>,
    pub extended_versions: [Option<ExtendedVersionEntry>; MAX_EXTENDED_VERSIONS],
    pub has_scene_colours: bool,
    pub scene_colours: Option<Vec<dali2rust_contracts::msg::DaliAttributeReadChunk>>,
}

#[allow(clippy::too_many_arguments, reason = "mirrors the public write surface")]
pub fn write_short_attributes(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    fade_time_ms: Option<u32>,
    fade_rate: Option<u8>,
    power_on_level: Option<u8>,
    system_failure_level: Option<u8>,
    extended_fade_time_ms: Option<u16>,
    tc_limits_mirek: (Option<u16>, Option<u16>),
    min_max_levels: (Option<u8>, Option<u8>),
    dimming_curve: Option<u8>,
) -> WriteAttributesExecution {
    let mut tally = WriteTally::default();
    let error = apply_short_attribute_writes(
        controller,
        short_address,
        &mut tally,
        fade_time_ms,
        fade_rate,
        power_on_level,
        system_failure_level,
        extended_fade_time_ms,
        tc_limits_mirek,
        min_max_levels,
        dimming_curve,
    )
    .err();
    tally.into_execution(error)
}

#[allow(clippy::too_many_arguments, reason = "mirrors the public write surface")]
fn apply_short_attribute_writes(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    tally: &mut WriteTally,
    fade_time_ms: Option<u32>,
    fade_rate: Option<u8>,
    power_on_level: Option<u8>,
    system_failure_level: Option<u8>,
    extended_fade_time_ms: Option<u16>,
    tc_limits_mirek: (Option<u16>, Option<u16>),
    min_max_levels: (Option<u8>, Option<u8>),
    dimming_curve: Option<u8>,
) -> Result<(), SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    if let Some(error) = write_fade_params(controller, address, tally, fade_time_ms, fade_rate) {
        return Err(error);
    }
    if let Some(error) =
        write_levels(controller, address, tally, power_on_level, system_failure_level)
    {
        return Err(error);
    }
    write_min_max_levels(controller, address, tally, min_max_levels)?;
    write_extended_fade_time(controller, address, tally, extended_fade_time_ms)?;
    write_tc_limits(controller, address, &mut tally.confirmed, tc_limits_mirek)?;
    write_dimming_curve(controller, address, tally, dimming_curve)
}

fn write_extended_fade_time(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    extended_fade_time_ms: Option<u16>,
) -> Result<(), SemanticDaliError> {
    let Some(ms) = extended_fade_time_ms else {
        return Ok(());
    };
    let dtr0 = extended_fade_time_byte_from_ms(ms);
    let verify = |c: &mut _| send_standard_query(c, address, StandardCommand::QueryExtendedFadeTime);
    let read_back =
        send_dtr0_config_verified(controller, address, dtr0, StandardCommand::SetExtendedFadeTime, verify)?;
    if tally.proved(read_back).is_some() {
        tally.confirmed.extended_fade_time_ms = extended_fade_time_ms_from_byte(dtr0);
    }
    Ok(())
}

fn reread_physical_minimum(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    confirmed: &mut ConfirmedWritableAttributes,
) -> Result<(), SemanticDaliError> {
    if let Ok(v) = send_standard_query(controller, address, StandardCommand::QueryPhysicalMinimum)
    {
        confirmed.physical_minimum_readback = v;
    }
    Ok(())
}

const EXTENDED_FADE_UNITS_MS: [(u32, u8); 4] = [(100, 1), (1_000, 2), (10_000, 3), (60_000, 4)];

fn extended_fade_time_byte_from_ms(ms: u16) -> u8 {
    if ms == 0 {
        return 0;
    }
    for (unit, mult) in EXTENDED_FADE_UNITS_MS {
        if u32::from(ms) <= unit * 16 {
            let base = ((u32::from(ms) + unit / 2) / unit).clamp(1, 16) as u8;
            let contract_max_base = (u32::from(u16::MAX) / unit).clamp(1, 16) as u8;
            let base = base.min(contract_max_base);
            return (mult << 4) | (base - 1);
        }
    }
    (4 << 4) | 0x0F
}

fn extended_fade_time_ms_from_byte(byte: u8) -> Option<u16> {
    let mult = (byte >> 4) & 0x07;
    if mult == 0 {
        return Some(0);
    }
    let unit = EXTENDED_FADE_UNITS_MS
        .iter()
        .find(|(_, m)| *m == mult)
        .map(|(u, _)| *u)?;
    let base = u32::from(byte & 0x0F) + 1;
    u16::try_from(unit * base).ok()
}

fn send_dtr0_config_verified<C: DaliApplicationController>(
    controller: &mut C,
    address: DaliAddress,
    dtr0: u8,
    command: StandardCommand,
    mut read_back: impl FnMut(&mut C) -> Result<Option<u8>, SemanticDaliError>,
) -> Result<ReadBack, SemanticDaliError> {
    controller.step_boundary();
    for _ in 0..=PROGRAM_VERIFY_REPAIRS {
        let verified = controller.transaction(|controller| {
            send_dtr0_backed_standard(controller, address, dtr0, command)?;
            read_back(controller)
        })?;
        match verified {
            Some(v) if v == dtr0 => return Ok(ReadBack::Proved(v)),
            None => return Ok(ReadBack::Unanswered),
            Some(_) => {}
        }
    }
    Ok(ReadBack::Refused)
}

fn send_dtr0_config_accepted<C: DaliApplicationController>(
    controller: &mut C,
    address: DaliAddress,
    dtr0: u8,
    command: StandardCommand,
    mut read_back: impl FnMut(&mut C) -> Result<Option<u8>, SemanticDaliError>,
) -> Result<ReadBack, SemanticDaliError> {
    controller.step_boundary();
    let mut previous: Option<u8> = None;
    for _ in 0..=PROGRAM_VERIFY_REPAIRS {
        let answer = controller.transaction(|controller| {
            send_dtr0_backed_standard(controller, address, dtr0, command)?;
            read_back(controller)
        })?;
        match answer {
            Some(v) if v == dtr0 => return Ok(ReadBack::Proved(v)),
            Some(v) if previous == Some(v) => return Ok(ReadBack::Proved(v)),
            Some(v) => previous = Some(v),
            None => return Ok(ReadBack::Unanswered),
        }
    }
    Ok(ReadBack::Refused)
}

// IEC 62386-102 §9.6
fn write_min_max_levels(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    (min_level, max_level): (Option<u8>, Option<u8>),
) -> Result<(), SemanticDaliError> {
    if let Some(min) = min_level {
        tally.confirmed.min_level = tally.proved(write_level_bound(controller, address, min, true)?);
    }
    if let Some(max) = max_level {
        tally.confirmed.max_level = tally.proved(write_level_bound(controller, address, max, false)?);
    }
    if let (Some(min), Some(_)) = (min_level, max_level) {
        if matches!(tally.confirmed.min_level, Some(accepted) if accepted < min) {
            tally.confirmed.min_level =
                tally.proved(write_level_bound(controller, address, min, true)?);
        }
    }
    Ok(())
}

fn write_level_bound(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    dtr0: u8,
    min_bound: bool,
) -> Result<ReadBack, SemanticDaliError> {
    let (set, query) = if min_bound {
        (StandardCommand::SetMinLevel, StandardCommand::QueryMinLevel)
    } else {
        (StandardCommand::SetMaxLevel, StandardCommand::QueryMaxLevel)
    };
    let verify = |c: &mut _| send_standard_query(c, address, query);
    send_dtr0_config_accepted(controller, address, dtr0, set, verify)
}

fn write_fade_params(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    fade_time_ms: Option<u32>,
    fade_rate: Option<u8>,
) -> Option<SemanticDaliError> {
    if let Some(ms) = fade_time_ms {
        let dtr0 = fade_time_dtr0_from_ms(ms);
        let verify = |c: &mut _| {
            Ok(send_standard_query(c, address, StandardCommand::QueryFadeTimeFadeRate)?
                .map(|b| b >> 4))
        };
        match send_dtr0_config_verified(controller, address, dtr0, StandardCommand::SetFadeTime, verify) {
            Err(error) => return Some(error),
            Ok(read_back) => {
                if let Some(code) = tally.proved(read_back) {
                    tally.confirmed.fade_time_ms = Some(fade_time_ms_from_dtr0(code));
                }
            }
        }
    }
    if let Some(rate) = fade_rate {
        let verify = |c: &mut _| {
            Ok(send_standard_query(c, address, StandardCommand::QueryFadeTimeFadeRate)?
                .map(|b| b & 0x0F))
        };
        match send_dtr0_config_verified(controller, address, rate, StandardCommand::SetFadeRate, verify) {
            Err(error) => return Some(error),
            Ok(read_back) => tally.confirmed.fade_rate = tally.proved(read_back),
        }
    }
    None
}

fn write_levels(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    power_on_level: Option<u8>,
    system_failure_level: Option<u8>,
) -> Option<SemanticDaliError> {
    if let Some(level) = power_on_level {
        let verify =
            |c: &mut _| send_standard_query(c, address, StandardCommand::QueryPowerOnLevel);
        match send_dtr0_config_verified(controller, address, level, StandardCommand::SetPowerOnLevel, verify)
        {
            Err(error) => return Some(error),
            Ok(read_back) => tally.confirmed.power_on_level = tally.proved(read_back),
        }
    }
    if let Some(level) = system_failure_level {
        let verify =
            |c: &mut _| send_standard_query(c, address, StandardCommand::QuerySystemFailureLevel);
        match send_dtr0_config_verified(
            controller,
            address,
            level,
            StandardCommand::SetSystemFailureLevel,
            verify,
        ) {
            Err(error) => return Some(error),
            Ok(read_back) => tally.confirmed.system_failure_level = tally.proved(read_back),
        }
    }
    None
}

fn wants_attr_group(groups: &[DaliAttributeGroup], target: DaliAttributeGroup) -> bool {
    groups.contains(&target)
}

fn read_random_address(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    breaker: &mut MandatorySilenceBreaker,
) -> Result<Option<u32>, SemanticDaliError> {
    let high = send_mandatory_query(breaker, controller, address, StandardCommand::QueryRandomAddressH)?;
    let middle = send_mandatory_query(breaker, controller, address, StandardCommand::QueryRandomAddressM)?;
    let low = send_mandatory_query(breaker, controller, address, StandardCommand::QueryRandomAddressL)?;
    Ok(match (high, middle, low) {
        (Some(high), Some(middle), Some(low)) => {
            Some((u32::from(high) << 16) | (u32::from(middle) << 8) | u32::from(low))
        }
        _ => None,
    })
}

struct ProbedIdentity {
    dt8_probe: Option<Dt8Status>,
    has_dt8_color: bool,
    has_common_102: bool,
    supported_device_types: Option<DeviceTypeSet>,
}

const PRESENCE_PROBE_RETRIES: u8 = 2;

fn probe_presence(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    outcomes: &mut AttributeReadOutcomes,
) -> Result<(), SemanticDaliError> {
    track(outcomes, AttributeReadSection::Identity, || {
        confirm_presence(controller, address)
    })
}

fn confirm_presence(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
) -> Result<(), SemanticDaliError> {
    for _ in 0..=PRESENCE_PROBE_RETRIES {
        if send_standard_response(
            controller,
            address,
            StandardCommand::QueryControlGearPresent,
        )?
        .is_yes()
        {
            return Ok(());
        }
    }
    Err(SemanticDaliError::OperationFailed(DEVICE_ABSENT_MESSAGE))
}

fn probe_identity_and_dt8(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    groups: &[DaliAttributeGroup],
    content_confirm: ContentConfirmPolicy,
    outcomes: &mut AttributeReadOutcomes,
) -> Result<ProbedIdentity, SemanticDaliError> {
    probe_presence(controller, address, outcomes)?;
    let wants_dt8 = wants_attr_group(groups, DaliAttributeGroup::Dt8Color);
    let mut supported_device_types = None;
    let dt8_status = if wants_dt8 {
        let identity = track(outcomes, AttributeReadSection::Dt8Color, || {
            probe_device_identity_with_policy(
                controller,
                address,
                Dt8ProbePolicy::ExplicitRead,
                content_confirm,
            )
        })?;
        supported_device_types = identity.supported_device_types;
        identity.detected.then_some(identity.dt8_status)
    } else {
        None
    };
    Ok(ProbedIdentity {
        has_dt8_color: dt8_status.as_ref().is_some_and(|s| s.supported),
        has_common_102: wants_attr_group(groups, DaliAttributeGroup::Common102),
        dt8_probe: dt8_status,
        supported_device_types,
    })
}

fn read_runtime_section(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    groups: &[DaliAttributeGroup],
    outcomes: &mut AttributeReadOutcomes,
) -> Result<Option<RuntimeSectionRead>, SemanticDaliError> {
    if !wants_attr_group(groups, DaliAttributeGroup::RuntimeStatus) {
        return Ok(None);
    }
    track(outcomes, AttributeReadSection::RuntimeStatus, || {
        read_runtime_setpoint(controller, address)
    })
}

struct CollectedAttributeScalars {
    probed: ProbedIdentity,
    runtime_setpoint: Option<LightSetpoint>,
    runtime_observation: Option<RuntimeObservation>,
    random_address: Option<u32>,
    c102: Common102Fields,
    dt8_color: Dt8ColorFields,
    failure_suspected: bool,
}

fn read_dt8_section(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    probed: &ProbedIdentity,
    content_confirm: ContentConfirmPolicy,
    outcomes: &mut AttributeReadOutcomes,
) -> Result<Dt8ColorFields, SemanticDaliError> {
    if !probed.has_dt8_color {
        return Ok(Dt8ColorFields::default());
    }
    let capability = |pick: fn(&Dt8Capabilities) -> bool| {
        probed
            .dt8_probe
            .as_ref()
            .and_then(|s| s.capabilities)
            .is_some_and(|c| pick(&c))
    };
    let rgb = capability(|c| c.rgb_capable);
    let rgbwaf = capability(|c| c.rgbwaf_capable);
    track(outcomes, AttributeReadSection::Dt8Color, || {
        read_dt8_color_values(controller, address, content_confirm, rgb, rgbwaf)
    })
}

fn split_runtime(
    runtime: Option<RuntimeSectionRead>,
) -> (bool, Option<LightSetpoint>, Option<RuntimeObservation>) {
    match runtime {
        Some(r) => (r.failure_suspected, Some(r.setpoint), Some(r.observation)),
        None => (false, None, None),
    }
}

#[allow(clippy::too_many_arguments, reason = "one sequential sweep; the breaker rides beside the controller it guards")]
fn collect_scalar_sections(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    probed: ProbedIdentity,
    runtime: Option<RuntimeSectionRead>,
    content_confirm: ContentConfirmPolicy,
    outcomes: &mut AttributeReadOutcomes,
    breaker: &mut MandatorySilenceBreaker,
) -> Result<CollectedAttributeScalars, SemanticDaliError> {
    let c102 = if probed.has_common_102 {
        track(outcomes, AttributeReadSection::Common102, || {
            read_common_102_attributes(controller, address, content_confirm, breaker)
        })?
    } else {
        Common102Fields::default()
    };
    let dt8_color = read_dt8_section(controller, address, &probed, content_confirm, outcomes)?;
    let random_address = track(outcomes, AttributeReadSection::Identity, || {
        read_random_address(controller, address, breaker)
    })?;
    let (failure_suspected, runtime_setpoint, runtime_observation) = split_runtime(runtime);
    Ok(CollectedAttributeScalars {
        probed,
        runtime_setpoint,
        runtime_observation,
        random_address,
        c102,
        dt8_color,
        failure_suspected,
    })
}

pub fn read_attributes(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    groups: &[DaliAttributeGroup],
    content_confirm: ContentConfirmPolicy,
    outcomes: &mut AttributeReadOutcomes,
) -> Result<AttributeReadExecution, SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    let probed = probe_identity_and_dt8(controller, address, groups, content_confirm, outcomes)?;
    let mut breaker = MandatorySilenceBreaker::new();
    let runtime = read_runtime_section(controller, address, groups, outcomes)?;
    let scalars = collect_scalar_sections(
        controller,
        address,
        probed,
        runtime,
        content_confirm,
        outcomes,
        &mut breaker,
    )?;
    build_attribute_read_execution(
        controller,
        address,
        groups,
        scalars,
        content_confirm,
        outcomes,
        &mut breaker,
    )
}

struct MembershipSections {
    has_groups: bool,
    groups_membership: Option<u16>,
    has_scenes: bool,
    scene_levels: Option<Vec<u8>>,
    has_dt6_led: bool,
    dt6: Option<Dt6ReadSnapshot>,
    has_extended: bool,
    extended: Option<ExtendedReadSnapshot>,
    has_scene_colours: bool,
    scene_colours: Option<Vec<dali2rust_contracts::msg::DaliAttributeReadChunk>>,
}

impl MembershipSections {
    fn gates(groups: &[DaliAttributeGroup]) -> Self {
        Self {
            has_groups: wants_attr_group(groups, DaliAttributeGroup::Groups),
            has_scenes: wants_attr_group(groups, DaliAttributeGroup::Scenes),
            has_dt6_led: wants_attr_group(groups, DaliAttributeGroup::Dt6Led),
            has_extended: wants_attr_group(groups, DaliAttributeGroup::Extended),
            has_scene_colours: wants_attr_group(groups, DaliAttributeGroup::SceneColours),
            groups_membership: None,
            scene_levels: None,
            dt6: None,
            extended: None,
            scene_colours: None,
        }
    }
}

struct ExtendedReadSnapshot {
    fade_time_ms: Option<u16>,
    version_number: Option<u8>,
    versions: [Option<ExtendedVersionEntry>; MAX_EXTENDED_VERSIONS],
}

#[derive(Clone, Copy)]
struct KnownVersions {
    device_types: Option<DeviceTypeSet>,
    dt6: Option<u8>,
}

fn read_extended_snapshot(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    known: KnownVersions,
) -> Result<ExtendedReadSnapshot, SemanticDaliError> {
    let mut fade_time_ms = read_extended_fade_time(controller, address, content_confirm)?;
    if fade_time_ms.is_none() {
        log::info!("extended fade time: unrepresentable readback byte, re-reading");
        fade_time_ms = read_extended_fade_time(controller, address, content_confirm)?;
    }
    let device_types = match known.device_types {
        Some(types) => Some(types),
        None => super::discovery::read_declared_device_types(controller, address, content_confirm)?,
    };
    let versions = read_extended_versions(controller, address, content_confirm, device_types, known.dt6)?;
    let version_number = versions
        .iter()
        .flatten()
        .find(|e| e.device_type == DeviceType::Led.code())
        .and_then(|e| e.version_number);
    Ok(ExtendedReadSnapshot {
        fade_time_ms,
        version_number,
        versions,
    })
}

// IEC 62386-102 §9.18, §11.6.2
fn read_extended_versions(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    device_types: Option<DeviceTypeSet>,
    dt6_version: Option<u8>,
) -> Result<[Option<ExtendedVersionEntry>; MAX_EXTENDED_VERSIONS], SemanticDaliError> {
    let mut versions = [None; MAX_EXTENDED_VERSIONS];
    let Some(device_types) = device_types else {
        return Ok(versions);
    };
    for (slot, device_type) in versions.iter_mut().zip(device_types.iter()) {
        let version_number = match dt6_version.filter(|_| device_type == DeviceType::Led.code()) {
            Some(already_read) => Some(already_read),
            None => send_extended_query_stable(
                controller,
                content_confirm,
                address,
                ExtendedCommand::ExtendedVersion { device_type },
            )?,
        };
        *slot = Some(ExtendedVersionEntry { device_type, version_number });
    }
    Ok(versions)
}

fn read_extended_fade_time(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<u16>, SemanticDaliError> {
    Ok(send_standard_query_stable(
        controller,
        content_confirm,
        address,
        StandardCommand::QueryExtendedFadeTime,
    )?
    .and_then(extended_fade_time_ms_from_byte))
}

fn known_device_types(scalars: &CollectedAttributeScalars) -> Option<DeviceTypeSet> {
    scalars
        .probed
        .supported_device_types
        .or(scalars.c102.supported_device_types)
}

fn dt6_failure_escalation(scalars: &CollectedAttributeScalars) -> bool {
    if !scalars.failure_suspected {
        return false;
    }
    known_device_types(scalars).is_none_or(|types| types.contains(DeviceType::Led.code()))
}

#[allow(clippy::too_many_arguments, reason = "one sequential sweep; the known device types ride beside the breaker")]
fn read_membership_and_dt_sections(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    groups: &[DaliAttributeGroup],
    content_confirm: ContentConfirmPolicy,
    outcomes: &mut AttributeReadOutcomes,
    breaker: &mut MandatorySilenceBreaker,
    escalate_dt6_failure: bool,
    known_device_types: Option<DeviceTypeSet>,
) -> Result<MembershipSections, SemanticDaliError> {
    let mut s = MembershipSections::gates(groups);
    s.groups_membership = track_gated(s.has_groups, outcomes, AttributeReadSection::Groups, || {
        read_groups_membership(controller, address, breaker)
    })?
    .flatten();
    s.scene_levels = track_gated(s.has_scenes, outcomes, AttributeReadSection::Scenes, || {
        read_scene_levels(controller, address, breaker)
    })?
    .flatten();
    s.dt6 = track_gated(s.has_dt6_led, outcomes, AttributeReadSection::Dt6Led, || {
        read_dt6_led_snapshot(controller, address, content_confirm)
    })?;
    if s.dt6.is_none() && escalate_dt6_failure {
        s.dt6 = read_dt6_failure_byte(controller, address, content_confirm)?;
    }
    let known = KnownVersions {
        device_types: known_device_types,
        dt6: s.dt6.as_ref().and_then(|d| d.extended_version_number),
    };
    s.extended = track_gated(s.has_extended, outcomes, AttributeReadSection::Extended, || {
        read_extended_snapshot(controller, address, content_confirm, known)
    })?;
    s.scene_colours = track_gated(
        s.has_scene_colours,
        outcomes,
        AttributeReadSection::SceneColours,
        || read_scene_colours(controller, address, content_confirm),
    )?;
    Ok(s)
}

fn read_scene_colours(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Vec<dali2rust_contracts::msg::DaliAttributeReadChunk>, SemanticDaliError> {
    let mut chunks = Vec::with_capacity(16);
    for scene in 0..16u8 {
        chunks.push(read_one_scene_colour(
            controller,
            address,
            scene,
            content_confirm,
        )?);
    }
    Ok(chunks)
}

pub fn read_scene_colour_readback(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    scene: u8,
) -> Result<dali2rust_contracts::msg::DaliAttributeReadChunk, SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    read_one_scene_colour(controller, address, scene, ContentConfirmPolicy::default())
}

// IEC 62386-209 §9.11.5, §9.12.6
fn read_one_scene_colour(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    scene: u8,
    content_confirm: ContentConfirmPolicy,
) -> Result<dali2rust_contracts::msg::DaliAttributeReadChunk, SemanticDaliError> {
    use dali2rust_contracts::msg::DaliAttributeReadChunk as Chunk;
    controller.transaction_exempt(|controller| {
        let level = send_standard_query(
            controller,
            address,
            StandardCommand::QuerySceneLevel { scene },
        )?;
        if level.is_none() {
            return Ok(Chunk::SceneColour {
                scene,
                level: None,
                colour_type: None,
                values: [None; 6],
            });
        }
        let colour_type = confirm_observed_query(controller, content_confirm, |c| {
            dt8_narrow_value_sample(c, address, DT8_COLOUR_VALUE_REPORT_COLOUR_TYPE)
        })?;
        let values = read_report_values(controller, address, colour_type, content_confirm)?;
        Ok(Chunk::SceneColour {
            scene,
            level,
            colour_type,
            values,
        })
    })
}

fn read_report_values(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    colour_type: Option<u8>,
    content_confirm: ContentConfirmPolicy,
) -> Result<[Option<u16>; 6], SemanticDaliError> {
    let mut values = [None; 6];
    match colour_type {
        Some(DT8_COLOUR_TYPE_TC) => {
            values[0] = read_dt8_color_value_u16(
                controller,
                address,
                DT8_COLOUR_VALUE_REPORT_TC,
                content_confirm,
            )?;
        }
        Some(DT8_COLOUR_TYPE_XY) => {
            for (slot, id) in [DT8_COLOUR_VALUE_REPORT_X, DT8_COLOUR_VALUE_REPORT_Y]
                .into_iter()
                .enumerate()
            {
                values[slot] = read_dt8_color_value_u16(controller, address, id, content_confirm)?;
            }
        }
        Some(DT8_COLOUR_TYPE_RGBWAF) => {
            for (slot, value) in values.iter_mut().enumerate() {
                *value = read_dt8_color_value_u8(
                    controller,
                    address,
                    DT8_COLOUR_VALUE_REPORT_RED + slot as u8,
                    content_confirm,
                )?
                .map(u16::from);
            }
        }
        _ => {}
    }
    Ok(values)
}

fn dt8_capability_flags(dt8_status: Option<&Dt8Status>) -> (bool, bool, bool, bool) {
    let caps = dt8_status.and_then(|s| s.capabilities);
    (
        caps.map(|c| c.xy_capable).unwrap_or(false),
        caps.map(|c| c.tc_capable).unwrap_or(false),
        caps.map(|c| c.rgb_capable).unwrap_or(false),
        caps.map(|c| c.rgbwaf_capable).unwrap_or(false),
    )
}

#[allow(clippy::too_many_arguments, reason = "one sequential sweep; the breaker rides beside the controller it guards")]
fn build_attribute_read_execution(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    groups: &[DaliAttributeGroup],
    scalars: CollectedAttributeScalars,
    content_confirm: ContentConfirmPolicy,
    outcomes: &mut AttributeReadOutcomes,
    breaker: &mut MandatorySilenceBreaker,
) -> Result<AttributeReadExecution, SemanticDaliError> {
    let sections = read_membership_and_dt_sections(
        controller,
        address,
        groups,
        content_confirm,
        outcomes,
        breaker,
        dt6_failure_escalation(&scalars),
        known_device_types(&scalars),
    )?;
    let caps = dt8_capability_flags(scalars.probed.dt8_probe.as_ref());
    Ok(scalars.into_execution(sections, caps))
}

impl CollectedAttributeScalars {
    fn into_execution(
        self,
        sections: MembershipSections,
        caps: (bool, bool, bool, bool),
    ) -> AttributeReadExecution {
        let probe = self.probed.dt8_probe.as_ref();
        let mut c102 = self.c102;
        c102.supported_device_types = self
            .probed
            .supported_device_types
            .or(c102.supported_device_types);
        AttributeReadExecution {
            runtime_setpoint: self.runtime_setpoint,
            runtime_observation: self.runtime_observation,
            random_address: self.random_address,
            has_common_102: self.probed.has_common_102,
            has_dt8_color: self.probed.has_dt8_color,
            dt8_supported: probe.map(|s| s.supported),
            has_groups: sections.has_groups,
            has_scenes: sections.has_scenes,
            has_dt6_led: sections.has_dt6_led,
            has_extended: sections.has_extended,
            dt8_color_mode: probe.map_or(ColorMode::Unknown, |s| s.color_mode),
            dt8_xy_capable: caps.0,
            dt8_tc_capable: caps.1,
            dt8_rgb_capable: caps.2,
            c102,
            groups_membership: sections.groups_membership,
            scene_levels: sections.scene_levels,
            dt8_rgbwaf_capable: caps.3,
            dt8_color: self.dt8_color,
            dt6: sections.dt6,
            extended_fade_time_ms: sections.extended.as_ref().and_then(|e| e.fade_time_ms),
            extended_version_number: sections.extended.as_ref().and_then(|e| e.version_number),
            extended_versions: sections
                .extended
                .as_ref()
                .map_or([None; MAX_EXTENDED_VERSIONS], |e| e.versions),
            has_scene_colours: sections.has_scene_colours,
            scene_colours: sections.scene_colours,
        }
    }
}

#[cfg(test)]
mod tests;
