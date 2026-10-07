use dali2rust_contracts::msg::{
    AttributeGroupReadOutcome, ColorMode, DaliAttributeGroup, DeviceTypeSet, Dt6ReadSnapshot,
    ExtendedVersionEntry, LightSetpoint, RuntimeObservation, StatusFlags,
    MAX_EXTENDED_VERSIONS,
};
use dali2rust_domain::dali::controller::{DaliApplicationController, ReadbackWorkaround};
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
    ArmedCommand,
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
mod extended;
mod outcomes;
mod scene_colours;
mod write;

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
use extended::{read_extended_snapshot, write_extended_fade_time, ExtendedReadSnapshot, KnownVersions};
#[cfg(test)]
use extended::{extended_fade_time_byte_from_ms, extended_fade_time_ms_from_byte};
pub use outcomes::{classify_read_abort, AttributeReadOutcomes, AttributeReadSection};
use outcomes::{track, track_gated};
pub use scene_colours::read_scene_colour_readback;
use scene_colours::read_scene_colours;
pub use write::{write_short_attributes, ConfirmedWritableAttributes, WriteAttributesExecution};
use write::{reread_physical_minimum, send_dtr0_config_verified, ReadBack, WriteTally};

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
