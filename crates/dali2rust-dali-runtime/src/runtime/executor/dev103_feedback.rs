use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::dev103::{
    feedback_capability, Device103Address, Feedback332Command, FeedbackOpcodeMap, InstanceAddress,
};

use super::dev103::{send, send_twice, stage_dtr0, verify_readback};
use crate::runtime::executor::SemanticDaliError;

pub const FEEDBACK_MAP_ABSENT: u8 = 0;
pub const FEEDBACK_MAP_CORRECTED: u8 = 1;
pub const FEEDBACK_MAP_ED1: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedbackProbe {
    pub map_code: u8,
    pub capability: Option<u8>,
    pub colour_capability: Option<u8>,
}

const fn map_of(code: u8) -> Option<FeedbackOpcodeMap> {
    match code {
        FEEDBACK_MAP_CORRECTED => Some(FeedbackOpcodeMap::DiiaCorrected),
        FEEDBACK_MAP_ED1 => Some(FeedbackOpcodeMap::Ed1),
        _ => None,
    }
}

// DiiA SW098bp §11.6.1
pub fn probe_feedback(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    instance_number: u8,
) -> Result<FeedbackProbe, SemanticDaliError> {
    let address = Device103Address::Short(short_address);
    let feature = InstanceAddress::FeatureNumber(instance_number);
    for (map, code) in [
        (FeedbackOpcodeMap::DiiaCorrected, FEEDBACK_MAP_CORRECTED),
        (FeedbackOpcodeMap::Ed1, FEEDBACK_MAP_ED1),
    ] {
        let answer = send(
            controller,
            Feedback332Command::QueryCapability.frame(address, feature, map),
            true,
        )?;
        if !answer.is_yes() {
            continue;
        }
        let capability = answer.value();
        let colour_capability = match (map, capability) {
            (FeedbackOpcodeMap::DiiaCorrected, Some(cap))
                if cap & feedback_capability::COLOUR != 0 =>
            {
                send(
                    controller,
                    Feedback332Command::QueryColourCapability.frame(address, feature, map),
                    true,
                )?
                .value()
            }
            _ => None,
        };
        return Ok(FeedbackProbe {
            map_code: code,
            capability,
            colour_capability,
        });
    }
    Ok(FeedbackProbe {
        map_code: FEEDBACK_MAP_ABSENT,
        capability: None,
        colour_capability: None,
    })
}

pub const FB_PATCH_TIMING: u8 = 1 << 0;
pub const FB_PATCH_ACTIVE_BRIGHTNESS: u8 = 1 << 1;
pub const FB_PATCH_ACTIVE_COLOUR: u8 = 1 << 2;
pub const FB_PATCH_INACTIVE_BRIGHTNESS: u8 = 1 << 3;
pub const FB_PATCH_INACTIVE_COLOUR: u8 = 1 << 4;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConfiguredFeedback {
    pub map_code: u8,
    pub timing: Option<u8>,
    pub active_brightness: Option<u8>,
    pub active_colour: Option<u8>,
    pub inactive_brightness: Option<u8>,
    pub inactive_colour: Option<u8>,
}

impl ConfiguredFeedback {
    pub fn proved_any(&self) -> bool {
        [
            self.timing,
            self.active_brightness,
            self.active_colour,
            self.inactive_brightness,
            self.inactive_colour,
        ]
        .iter()
        .any(Option::is_some)
    }
}

const FB_FIELDS: [(u8, Feedback332Command, Feedback332Command); 5] = [
    (FB_PATCH_TIMING, Feedback332Command::SetTiming, Feedback332Command::QueryTiming),
    (
        FB_PATCH_ACTIVE_BRIGHTNESS,
        Feedback332Command::SetActiveBrightness,
        Feedback332Command::QueryActiveBrightness,
    ),
    (
        FB_PATCH_ACTIVE_COLOUR,
        Feedback332Command::SetActiveColour,
        Feedback332Command::QueryActiveColour,
    ),
    (
        FB_PATCH_INACTIVE_BRIGHTNESS,
        Feedback332Command::SetInactiveBrightness,
        Feedback332Command::QueryInactiveBrightness,
    ),
    (
        FB_PATCH_INACTIVE_COLOUR,
        Feedback332Command::SetInactiveColour,
        Feedback332Command::QueryInactiveColour,
    ),
];

pub fn configure_feedback(
    controller: &mut impl DaliApplicationController,
    cmd: &dali2rust_contracts::msg::Dali103FeedbackConfigureCommand,
    proved: &mut ConfiguredFeedback,
) -> Result<(), SemanticDaliError> {
    let map_code = match map_of(cmd.opcode_map) {
        Some(_) => cmd.opcode_map,
        None => {
            let probe = probe_feedback(controller, cmd.short_address, cmd.instance_number)?;
            probe.map_code
        }
    };
    let Some(map) = map_of(map_code) else {
        return Err(SemanticDaliError::OperationFailed("feedback_not_supported"));
    };
    proved.map_code = map_code;
    let values = [
        cmd.timing,
        cmd.active_brightness,
        cmd.active_colour,
        cmd.inactive_brightness,
        cmd.inactive_colour,
    ];
    let slots: [&mut Option<u8>; 5] = [
        &mut proved.timing,
        &mut proved.active_brightness,
        &mut proved.active_colour,
        &mut proved.inactive_brightness,
        &mut proved.inactive_colour,
    ];
    for (((mask, set, query), value), slot) in FB_FIELDS.iter().zip(values).zip(slots) {
        if cmd.patch_mask & mask != 0 {
            write_one(controller, cmd, map, value, *set, *query)?;
            *slot = Some(value);
            controller.step_boundary();
        }
    }
    Ok(())
}

fn write_one(
    controller: &mut impl DaliApplicationController,
    cmd: &dali2rust_contracts::msg::Dali103FeedbackConfigureCommand,
    map: FeedbackOpcodeMap,
    value: u8,
    set: Feedback332Command,
    query: Feedback332Command,
) -> Result<(), SemanticDaliError> {
    let address = Device103Address::Short(cmd.short_address);
    let feature = InstanceAddress::FeatureNumber(cmd.instance_number);
    controller.transaction(|c| {
        stage_dtr0(c, address, value)?;
        send_twice(c, set.frame(address, feature, map))
    })?;
    let seen = send(controller, query.frame(address, feature, map), true)?;
    verify_readback(seen, |actual| actual == value).map(|_| ())
}

pub fn drive_feedback(
    controller: &mut impl DaliApplicationController,
    cmd: &dali2rust_contracts::msg::Dali103FeedbackDriveCommand,
) -> Result<(), SemanticDaliError> {
    let address = match cmd.short_address {
        Some(short) => Device103Address::Short(short),
        None => Device103Address::Broadcast,
    };
    let feature = match (cmd.feature_number, cmd.feature_group) {
        (Some(number), _) => InstanceAddress::FeatureNumber(number),
        (None, Some(group)) => InstanceAddress::FeatureGroup(group),
        (None, None) => InstanceAddress::FeatureBroadcast,
    };
    let command = match cmd.action {
        0 => Feedback332Command::Activate,
        1 => Feedback332Command::Stop,
        2 => Feedback332Command::Select(cmd.selected_group),
        _ => return Err(SemanticDaliError::OperationFailed("invalid_feedback_action")),
    };
    send(
        controller,
        command.frame(address, feature, FeedbackOpcodeMap::DiiaCorrected),
        false,
    )?;
    Ok(())
}
