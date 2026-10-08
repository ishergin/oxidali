use dali2rust_contracts::msg::{
    ColorMode, ColorValue, DaliSceneTargetState, SceneProgramAction,
};
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;

use crate::runtime::executor::helpers::{
    dali_short_address, program_with_verify_repair, send_special, send_standard,
    send_standard_query, SemanticDaliError,
};
use crate::runtime::executor::target_state::{
    apply_dt8_cct, apply_dt8_rgb, apply_dt8_waf, apply_dt8_xy, waf_channels,
};

pub(crate) const SCENE_SLOT_MASK: u8 = 0xFF;

pub fn program_scene_row(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    scene_id: u8,
    action: SceneProgramAction,
    target_state: Option<&DaliSceneTargetState>,
) -> Result<Option<u8>, SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    let expected = expected_slot_byte(action, target_state)?;
    program_with_verify_repair(
        controller,
        |c| drive_scene_program(c, address, scene_id, action, target_state),
        |readback| readback == Some(expected),
    )
}

fn expected_slot_byte(
    action: SceneProgramAction,
    target_state: Option<&DaliSceneTargetState>,
) -> Result<u8, SemanticDaliError> {
    match action {
        SceneProgramAction::Write | SceneProgramAction::Update => target_state
            .map(DaliSceneTargetState::stored_level)
            .ok_or(SemanticDaliError::Conflict("scene_target_state_missing")),
        SceneProgramAction::Clear => Ok(SCENE_SLOT_MASK),
    }
}

fn drive_scene_program(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    scene_id: u8,
    action: SceneProgramAction,
    target_state: Option<&DaliSceneTargetState>,
) -> Result<Option<u8>, SemanticDaliError> {
    match action {
        SceneProgramAction::Write | SceneProgramAction::Update => {
            let target = target_state
                .ok_or(SemanticDaliError::Conflict("scene_target_state_missing"))?;
            program_scene_write(controller, address, scene_id, target)
        }
        SceneProgramAction::Clear => program_scene_clear(controller, address, scene_id),
    }
}

// IEC 62386-209 §9.11.2
fn program_scene_write(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    scene_id: u8,
    target: &DaliSceneTargetState,
) -> Result<Option<u8>, SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        controller.unit(|controller| {
            prepare_scene_colour(controller, address, scene_id, target)?;
            send_special(controller, SpecialCommand::Dtr0(target.stored_level()))?;
            send_standard(
                controller,
                address,
                StandardCommand::SetScene { scene: scene_id },
            )
        })?;
        read_scene_level(controller, address, scene_id)
    })
}

fn program_scene_clear(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    scene_id: u8,
) -> Result<Option<u8>, SemanticDaliError> {
    send_standard(
        controller,
        address,
        StandardCommand::RemoveScene { scene: scene_id },
    )?;
    read_scene_level(controller, address, scene_id)
}

fn prepare_scene_colour(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    scene_id: u8,
    target: &DaliSceneTargetState,
) -> Result<(), SemanticDaliError> {
    if apply_scene_color(controller, address, target)? {
        return Ok(());
    }
    clear_scene_colour(controller, address, scene_id, target.stored_level())
}

// IEC 62386-209 §11.2.2, §11.2.3, Table 4, Table 6
fn clear_scene_colour(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    scene_id: u8,
    level: u8,
) -> Result<(), SemanticDaliError> {
    send_special(controller, SpecialCommand::Dtr0(level))?;
    send_standard(controller, address, StandardCommand::SetScene { scene: scene_id })?;
    send_standard(controller, address, StandardCommand::RemoveScene { scene: scene_id })?;
    Ok(())
}

fn apply_scene_color(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    target: &DaliSceneTargetState,
) -> Result<bool, SemanticDaliError> {
    let Some(color) = target.color else {
        return Ok(false);
    };
    match color.mode {
        ColorMode::Cct if color.color_temperature_kelvin == 0 => {
            Err(SemanticDaliError::Conflict("invalid_color_temperature"))
        }
        ColorMode::Cct => apply_dt8_cct(controller, address, color.color_temperature_kelvin),
        ColorMode::Xy if color.x == 0 && color.y == 0 => {
            Err(SemanticDaliError::Conflict("invalid_color_xy"))
        }
        ColorMode::Xy => apply_dt8_xy(controller, address, color.x, color.y),
        ColorMode::Rgb | ColorMode::Rgbwaf => stage_scene_rgbwaf(controller, address, &color),
        _ => return Ok(false),
    }?;
    Ok(true)
}

// IEC 62386-209 §9.12.5
fn stage_scene_rgbwaf(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    color: &ColorValue,
) -> Result<(), SemanticDaliError> {
    apply_dt8_rgb(controller, address, color.r, color.g, color.b)?;
    let (w, a, f) = waf_channels(color);
    apply_dt8_waf(controller, address, w, a, f)
}

fn read_scene_level(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    scene_id: u8,
) -> Result<Option<u8>, SemanticDaliError> {
    Ok(send_standard_query(
        controller,
        address,
        StandardCommand::QuerySceneLevel { scene: scene_id },
    )
    .unwrap_or(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::PowerState;
    use crate::runtime::executor::helpers::{
        DT8_SET_TEMPERATURE_TC, DT8_SET_TEMPORARY_RGB_DIMLEVEL, DT8_SET_TEMPORARY_WAF_DIMLEVEL,
    };
    use crate::runtime::executor::test_helpers::shared::{
        assert_script_consumed, setup_controller, short_address, short_raw_query_frame,
        wire_counters,
    };
    use std::sync::atomic::Ordering::Relaxed;
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_domain::dali::commands::DaliCommand;

    const SHORT: u8 = 9;
    const SCENE: u8 = 3;

    fn standard_frame(command: StandardCommand) -> u16 {
        DaliCommand::Standard {
            address: short_address(SHORT),
            command,
        }
        .to_forward_frame()
        .raw()
    }

    fn special_frame(command: SpecialCommand) -> u16 {
        DaliCommand::Special(command).to_forward_frame().raw()
    }

    fn level_target(level: u8) -> DaliSceneTargetState {
        DaliSceneTargetState {
            power: Some(PowerState::On),
            level: Some(level),
            color: None,
        }
    }

    fn expect_colour_clear(mock: &MockDaliTransport, level: u8) {
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(level)));
        for command in [
            StandardCommand::SetScene { scene: SCENE },
            StandardCommand::RemoveScene { scene: SCENE },
        ] {
            mock.expect_forward_frame(standard_frame(command));
            mock.expect_forward_frame(standard_frame(command));
        }
    }

    #[test]
    fn a_row_without_colour_consumes_and_removes_the_scene_colour_before_storing_its_level() {
        let mock = MockDaliTransport::new();
        expect_colour_clear(&mock, 100);
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(100)));
        let set_scene = standard_frame(StandardCommand::SetScene { scene: SCENE });
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QuerySceneLevel { scene: SCENE }),
            Some(100),
        );

        let (transport, mut controller) = setup_controller(mock);
        let level = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Write,
            Some(&level_target(100)),
        )
        .expect("scene write");
        assert_eq!(level, Some(100));
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_collided_scene_level_read_back_does_not_program_the_scene_again() {
        let mock = MockDaliTransport::new();
        expect_colour_clear(&mock, 100);
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(100)));
        let set_scene = standard_frame(StandardCommand::SetScene { scene: SCENE });
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame(set_scene);
        let read_back = standard_frame(StandardCommand::QuerySceneLevel { scene: SCENE });
        mock.expect_forward_frame_collision(read_back);
        mock.expect_forward_frame_with_backward(read_back, Some(100));

        let (transport, mut controller) = setup_controller(mock);
        let level = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Write,
            Some(&level_target(100)),
        )
        .expect("scene write");
        assert_eq!(level, Some(100), "the read-back is asked again on its own");
        assert_script_consumed(&transport);
    }

    #[test]
    fn scene_write_with_cct_sends_temporary_color_prelude() {
        let mock = MockDaliTransport::new();
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(0x72)));
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr1(0x01)));
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QueryContentDtr0),
            Some(0x72),
        );
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QueryContentDtr1),
            Some(0x01),
        );
        mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
        mock.expect_forward_frame(short_raw_query_frame(
            short_address(SHORT),
            DT8_SET_TEMPERATURE_TC,
        ));
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(179)));
        let set_scene = standard_frame(StandardCommand::SetScene { scene: SCENE });
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QuerySceneLevel { scene: SCENE }),
            Some(179),
        );

        let (transport, mut controller) = setup_controller(mock);
        let mut target = level_target(179);
        target.color = Some(ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 2700,
            ..Default::default()
        });
        let level = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Update,
            Some(&target),
        )
        .expect("scene update");
        assert_eq!(level, Some(179));
        assert_script_consumed(&transport);
    }

    fn expect_staged_triple(mock: &MockDaliTransport, values: [u8; 3], opcode: u8) {
        for (write, value) in [
            SpecialCommand::Dtr0 as fn(u8) -> SpecialCommand,
            SpecialCommand::Dtr1,
            SpecialCommand::Dtr2,
        ]
        .iter()
        .zip(values)
        {
            mock.expect_forward_frame(special_frame(write(value)));
        }
        for (query, value) in [
            StandardCommand::QueryContentDtr0,
            StandardCommand::QueryContentDtr1,
            StandardCommand::QueryContentDtr2,
        ]
        .iter()
        .zip(values)
        {
            mock.expect_forward_frame_with_backward(standard_frame(*query), Some(value));
        }
        mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
        mock.expect_forward_frame(short_raw_query_frame(short_address(SHORT), opcode));
    }

    fn expect_scene_rgbwaf_write(mock: &MockDaliTransport, rgb: [u8; 3], waf: [u8; 3], level: u8) {
        expect_staged_triple(mock, rgb, DT8_SET_TEMPORARY_RGB_DIMLEVEL);
        expect_staged_triple(mock, waf, DT8_SET_TEMPORARY_WAF_DIMLEVEL);
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(level)));
        let set_scene = standard_frame(StandardCommand::SetScene { scene: SCENE });
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QuerySceneLevel { scene: SCENE }),
            Some(level),
        );
    }

    fn program_colour_row(mock: MockDaliTransport, color: ColorValue, level: u8) {
        let (transport, mut controller) = setup_controller(mock);
        let mut target = level_target(level);
        target.color = Some(color);
        let readback = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Write,
            Some(&target),
        )
        .expect("scene write");
        assert_eq!(readback, Some(level));
        assert_script_consumed(&transport);
    }

    #[test]
    fn a_three_channel_scene_row_stores_zero_white_amber_and_freecolour() {
        let mock = MockDaliTransport::new();
        expect_scene_rgbwaf_write(&mock, [1, 2, 3], [0, 0, 0], 179);
        program_colour_row(
            mock,
            ColorValue {
                mode: ColorMode::Rgb,
                r: 10,
                g: 20,
                b: 30,
                w: 44,
                a: 55,
                f: 66,
                ..Default::default()
            },
            179,
        );
    }

    #[test]
    fn a_six_channel_scene_row_stores_all_six_channels() {
        let mock = MockDaliTransport::new();
        expect_scene_rgbwaf_write(&mock, [1, 2, 3], [5, 8, 11], 179);
        program_colour_row(
            mock,
            ColorValue {
                mode: ColorMode::Rgbwaf,
                r: 10,
                g: 20,
                b: 30,
                w: 40,
                a: 50,
                f: 60,
                ..Default::default()
            },
            179,
        );
    }

    #[test]
    fn scene_clear_sends_remove_scene_and_reads_mask_back() {
        let mock = MockDaliTransport::new();
        let remove = standard_frame(StandardCommand::RemoveScene { scene: SCENE });
        mock.expect_forward_frame(remove);
        mock.expect_forward_frame(remove);
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QuerySceneLevel { scene: SCENE }),
            Some(0xFF),
        );

        let (transport, mut controller) = setup_controller(mock);
        let level = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Clear,
            None,
        )
        .expect("scene clear");
        assert_eq!(level, Some(0xFF));
        assert_script_consumed(&transport);
    }

    fn expect_write_drive(mock: &MockDaliTransport, level: u8, readback: Option<u8>) {
        expect_colour_clear(mock, level);
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(level)));
        let set_scene = standard_frame(StandardCommand::SetScene { scene: SCENE });
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QuerySceneLevel { scene: SCENE }),
            readback,
        );
    }

    #[test]
    fn a_collision_on_the_remove_reruns_the_unit_from_its_first_dtr0() {
        let mock = MockDaliTransport::new();
        mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(70)));
        let set_scene = standard_frame(StandardCommand::SetScene { scene: SCENE });
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame(set_scene);
        mock.expect_forward_frame_collision(standard_frame(StandardCommand::RemoveScene {
            scene: SCENE,
        }));
        expect_write_drive(&mock, 70, Some(70));

        let (transport, mut controller) = setup_controller(mock);
        let level = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Write,
            Some(&level_target(70)),
        )
        .expect("scene write after a collision");
        assert_eq!(level, Some(70));
        assert_script_consumed(&transport);
    }

    #[test]
    fn unanswered_readback_is_programmed_but_unconfirmed() {
        let mock = MockDaliTransport::new();
        expect_write_drive(&mock, 50, None);
        expect_write_drive(&mock, 50, None);
        expect_write_drive(&mock, 50, None);

        let (transport, mut controller) = setup_controller(mock);
        let level = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Write,
            Some(&level_target(50)),
        )
        .expect("scene write with silent readback");
        assert_eq!(level, None);
        assert_script_consumed(&transport);
    }

    #[test]
    fn lost_readback_repairs_once_and_confirms() {
        let mock = MockDaliTransport::new();
        expect_write_drive(&mock, 60, None);
        expect_write_drive(&mock, 60, Some(60));

        let (transport, mut controller) = setup_controller(mock);
        let counters = wire_counters(&mut controller);
        let level = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Write,
            Some(&level_target(60)),
        )
        .expect("scene write repaired");
        assert_eq!(level, Some(60));
        assert_eq!(
            counters.program_repairs.load(Relaxed),
            0,
            "a lost read-back is driven again but is not a wrong answer"
        );
        assert_script_consumed(&transport);
    }

    #[test]
    fn stale_clear_readback_redrives_the_config_pair() {
        let mock = MockDaliTransport::new();
        let remove = standard_frame(StandardCommand::RemoveScene { scene: SCENE });
        let query = standard_frame(StandardCommand::QuerySceneLevel { scene: SCENE });
        mock.expect_forward_frame(remove);
        mock.expect_forward_frame(remove);
        mock.expect_forward_frame_with_backward(query, Some(200));
        mock.expect_forward_frame(remove);
        mock.expect_forward_frame(remove);
        mock.expect_forward_frame_with_backward(query, Some(SCENE_SLOT_MASK));

        let (transport, mut controller) = setup_controller(mock);
        let level =
            program_scene_row(&mut controller, SHORT, SCENE, SceneProgramAction::Clear, None)
                .expect("scene clear repaired");
        assert_eq!(level, Some(SCENE_SLOT_MASK));
        assert_script_consumed(&transport);
    }

    #[test]
    fn silent_repair_readback_keeps_last_answered_evidence() {
        let mock = MockDaliTransport::new();
        expect_write_drive(&mock, 60, Some(30));
        expect_write_drive(&mock, 60, None);
        expect_write_drive(&mock, 60, None);

        let (transport, mut controller) = setup_controller(mock);
        let counters = wire_counters(&mut controller);
        let level = program_scene_row(
            &mut controller,
            SHORT,
            SCENE,
            SceneProgramAction::Write,
            Some(&level_target(60)),
        )
        .expect("scene write with mismatched evidence");
        assert_eq!(level, Some(30));
        assert_eq!(
            counters.program_repairs.load(Relaxed),
            1,
            "the first repair followed a wrong level, the second only silence"
        );
        assert_script_consumed(&transport);
    }
}
