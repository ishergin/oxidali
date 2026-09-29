use dali2rust_contracts::msg::{Dali103FeedbackConfigureCommand, FeedbackPatchField};
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
    fn slot_mut(&mut self, field: FeedbackPatchField) -> &mut Option<u8> {
        match field {
            FeedbackPatchField::Timing => &mut self.timing,
            FeedbackPatchField::ActiveBrightness => &mut self.active_brightness,
            FeedbackPatchField::ActiveColour => &mut self.active_colour,
            FeedbackPatchField::InactiveBrightness => &mut self.inactive_brightness,
            FeedbackPatchField::InactiveColour => &mut self.inactive_colour,
        }
    }

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

const fn commands_of(field: FeedbackPatchField) -> (Feedback332Command, Feedback332Command) {
    match field {
        FeedbackPatchField::Timing => (Feedback332Command::SetTiming, Feedback332Command::QueryTiming),
        FeedbackPatchField::ActiveBrightness => (
            Feedback332Command::SetActiveBrightness,
            Feedback332Command::QueryActiveBrightness,
        ),
        FeedbackPatchField::ActiveColour => {
            (Feedback332Command::SetActiveColour, Feedback332Command::QueryActiveColour)
        }
        FeedbackPatchField::InactiveBrightness => (
            Feedback332Command::SetInactiveBrightness,
            Feedback332Command::QueryInactiveBrightness,
        ),
        FeedbackPatchField::InactiveColour => {
            (Feedback332Command::SetInactiveColour, Feedback332Command::QueryInactiveColour)
        }
    }
}

pub fn configure_feedback(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103FeedbackConfigureCommand,
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
    for field in FeedbackPatchField::ALL {
        if !cmd.patches(field) {
            continue;
        }
        let (set, query) = commands_of(field);
        write_one(controller, cmd, map, cmd.value(field), set, query)?;
        *proved.slot_mut(field) = Some(cmd.value(field));
        controller.step_boundary();
    }
    Ok(())
}

fn write_one(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103FeedbackConfigureCommand,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::executor::test_helpers::shared::setup_controller;
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;

    const VALUE: u8 = 9;

    fn expected_set_command(field: FeedbackPatchField) -> Feedback332Command {
        match field {
            FeedbackPatchField::Timing => Feedback332Command::SetTiming,
            FeedbackPatchField::ActiveBrightness => Feedback332Command::SetActiveBrightness,
            FeedbackPatchField::ActiveColour => Feedback332Command::SetActiveColour,
            FeedbackPatchField::InactiveBrightness => Feedback332Command::SetInactiveBrightness,
            FeedbackPatchField::InactiveColour => Feedback332Command::SetInactiveColour,
        }
    }

    fn command_patching(field: FeedbackPatchField) -> Dali103FeedbackConfigureCommand {
        let mut cmd = Dali103FeedbackConfigureCommand {
            registry_adapter_id: 0,
            short_address: 2,
            instance_number: 1,
            patch_mask: 0,
            timing: 0,
            active_brightness: 0,
            active_colour: 0,
            inactive_brightness: 0,
            inactive_colour: 0,
            opcode_map: FEEDBACK_MAP_CORRECTED,
        };
        cmd.patch(field, VALUE);
        cmd
    }

    #[test]
    fn every_feedback_field_is_written_with_its_own_command_and_proved_in_its_own_slot() {
        let (address, feature) = (Device103Address::Short(2), InstanceAddress::FeatureNumber(1));
        for field in FeedbackPatchField::ALL {
            let mock = MockDaliTransport::new();
            mock.set_persistent_response(VALUE);
            let (transport, mut controller) = setup_controller(mock);
            let mut proved = ConfiguredFeedback::default();
            configure_feedback(&mut controller, &command_patching(field), &mut proved)
                .unwrap_or_else(|error| panic!("{field:?} did not land: {error:?}"));

            let set = expected_set_command(field)
                .frame(address, feature, FeedbackOpcodeMap::DiiaCorrected)
                .as_bytes();
            let frames = transport.lock().expect("mock lock").sent_frames24();
            assert_eq!(frames.iter().filter(|frame| **frame == set).count(), 2, "{field:?}");
            for other in FeedbackPatchField::ALL {
                let want = (other == field).then_some(VALUE);
                assert_eq!(*proved.slot_mut(other), want, "{field:?} proved {other:?}");
            }
        }
    }
}
