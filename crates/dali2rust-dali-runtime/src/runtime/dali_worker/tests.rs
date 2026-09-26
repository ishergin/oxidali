use dali2rust_domain::dali::commands::{DaliCommand, DaliResponse, StandardCommand};
use dali2rust_domain::dali::controller::{DaliApplicationController, DaliProductController};
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::ses::DaliSession;
use dali2rust_domain::dali::types::DaliAddress;

struct MockController {
    last_command: Option<DaliCommand>,
    response: DaliResponse,
    session: DaliSession,
}

impl MockController {
    fn new(response: DaliResponse) -> Self {
        Self {
            last_command: None,
            response,
            session: DaliSession::new(),
        }
    }
}

impl DaliProductController for MockController {
    type Error = dali2rust_domain::dali::frame::FrameError;

    fn send_command(&mut self, cmd: &DaliCommand) -> Result<DaliResponse, Self::Error> {
        self.last_command = Some(*cmd);
        Ok(self.response)
    }

    fn session(&self) -> &DaliSession {
        &self.session
    }
}

impl DaliApplicationController for MockController {
    fn send_raw(
        &mut self,
        _frame: ForwardFrame,
        _expects_backward: bool,
    ) -> Result<DaliResponse, Self::Error> {
        Ok(DaliResponse::NoAnswer)
    }
}

#[test]
fn mock_controller_baseline() {
    let mut ctrl = MockController::new(DaliResponse::Answer(0x55));
    let cmd = DaliCommand::Standard {
        address: DaliAddress::short(1).unwrap(),
        command: StandardCommand::QueryStatus,
    };
    let resp = ctrl.send_command(&cmd).unwrap();
    assert_eq!(resp, DaliResponse::Answer(0x55));
    assert_eq!(ctrl.last_command, Some(cmd));
}

use super::DaliWorkerCounters;

#[test]
fn test_publish_read_attributes_evidence_publishes_runtime_update_and_event() {
    use dali2rust_bus::{BusConfig, BusHost};
    use dali2rust_contracts::msg::{
        DaliReadAttributesCommand, LightSetpoint, MemoryBankReadPreset, PowerState,
    };

    let config = BusConfig {
        commands_ingress: 1,
        events_ingress: 10,
        ..Default::default()
    };
    let (_host, publisher, ()) = BusHost::spawn(config, |_| {});

    let ar = DaliReadAttributesCommand {
        registry_adapter_id: 0,
        short_address: 5,
        attribute_groups_mask: 0,
        memory_banks: MemoryBankReadPreset::None,
    };

    let setpoint = LightSetpoint {
        power: PowerState::On,
        level: 254,
        ..Default::default()
    };

    let execution = crate::runtime::executor::AttributeReadExecution {
        runtime_setpoint: Some(setpoint),
        runtime_observation: Some(dali2rust_contracts::msg::RuntimeObservation::default()),
        random_address: None,
        has_common_102: false,
        has_dt8_color: false,
        dt8_supported: Some(false),
        has_groups: false,
        has_scenes: false,
        has_dt6_led: false,
        has_extended: false,
        dt8_color_mode: dali2rust_contracts::msg::ColorMode::None,
        dt8_xy_capable: false,
        dt8_tc_capable: false,
        dt8_rgb_capable: false,
        c102: Default::default(),
        groups_membership: None,
        scene_levels: None,
        dt8_rgbwaf_capable: false,
        dt8_color: Default::default(),
        dt6: None,
        extended_fade_time_ms: None,
        extended_version_number: None,
        has_scene_colours: false,
        scene_colours: None,
    };

    let counters = DaliWorkerCounters::default();
    let adapter_id = dali2rust_bus::BusId(1);
    let correlation_id = 12345;

    assert!(super::attribute_read::publish_read_attributes_evidence(
        &publisher,
        adapter_id,
        correlation_id,
        super::attribute_read::ReadProvenance {
            value_source: super::RuntimeSource::Api,
            started_mono_ms: 0,
        },
        &ar,
        &execution,
        &counters,
    ));
}

mod measured_colour {
    use dali2rust_contracts::msg::ColorMode;

    use crate::runtime::executor::AttributeReadExecution;

    fn dt8_read(mode: ColorMode) -> AttributeReadExecution {
        AttributeReadExecution {
            runtime_setpoint: None,
            runtime_observation: None,
            random_address: None,
            has_common_102: false,
            has_dt8_color: true,
            dt8_supported: Some(true),
            has_groups: false,
            has_scenes: false,
            has_dt6_led: false,
            has_extended: false,
            dt8_color_mode: mode,
            dt8_xy_capable: false,
            dt8_tc_capable: false,
            dt8_rgb_capable: false,
            c102: Default::default(),
            groups_membership: None,
            scene_levels: None,
            dt8_rgbwaf_capable: false,
            dt8_color: crate::runtime::executor::Dt8ColorFields {
                active_mode: mode,
                ..Default::default()
            },
            dt6: None,
            extended_fade_time_ms: None,
            extended_version_number: None,
            has_scene_colours: false,
            scene_colours: None,
        }
    }

    fn colour(
        execution: &AttributeReadExecution,
    ) -> Option<dali2rust_contracts::msg::ColorValue> {
        super::super::attribute_read::measured_colour(execution)
    }

    #[test]
    fn a_measured_tc_is_published_in_kelvin() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.dt8_color.value_2 = Some(250);
        let colour = colour(&execution).expect("a measured Tc is an observation");
        assert_eq!(colour.mode, ColorMode::Cct);
        assert_eq!(colour.color_temperature_kelvin, 4000);
    }

    #[test]
    fn an_unanswered_tc_publishes_nothing() {
        let execution = dt8_read(ColorMode::Cct);
        assert!(colour(&execution).is_none());
    }

    #[test]
    fn a_zero_mirek_is_not_a_temperature() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.dt8_color.value_2 = Some(0);
        assert!(colour(&execution).is_none());
    }

    #[test]
    fn xy_needs_both_halves() {
        let mut execution = dt8_read(ColorMode::Xy);
        execution.dt8_color.value_0 = Some(6553);
        assert!(colour(&execution).is_none(), "x alone is half a coordinate");
        execution.dt8_color.value_1 = Some(13107);
        let colour = colour(&execution).expect("both halves measured");
        assert_eq!((colour.mode, colour.x, colour.y), (ColorMode::Xy, 6553, 13107));
    }

    #[test]
    fn measured_rgb_dim_levels_are_published() {
        for mode in [ColorMode::Rgb, ColorMode::Rgbwaf] {
            let mut execution = dt8_read(mode);
            execution.dt8_color.rgb = Some((254, 10, 20));
            let colour = colour(&execution).expect("a measured triple is an observation");
            assert_eq!(
                (colour.mode, colour.r, colour.g, colour.b),
                (ColorMode::Rgb, 255, 55, 79),
                "three channels is the whole colour of a three-channel gear"
            );
        }
    }

    #[test]
    fn a_six_channel_colour_is_published_whole_or_not_at_all() {
        let mut execution = dt8_read(ColorMode::Rgbwaf);
        execution.dt8_rgbwaf_capable = true;
        execution.dt8_color.rgb = Some((254, 10, 20));
        assert!(
            colour(&execution).is_none(),
            "three measured channels beside three unread ones is not a colour"
        );

        execution.dt8_color.waf = Some((1, 2, 3));
        let colour = colour(&execution).expect("all six measured");
        assert_eq!(
            (colour.mode, colour.r, colour.w, colour.a, colour.f),
            (ColorMode::Rgbwaf, 255, 1, 18, 26),
            "all six channels decode, not just the first three"
        );
    }

    #[test]
    fn an_unanswered_rgb_triple_publishes_nothing() {
        for mode in [ColorMode::Rgb, ColorMode::Rgbwaf] {
            assert!(colour(&dt8_read(mode)).is_none(), "{mode:?}");
        }
    }

    #[test]
    fn a_gear_without_colour_reports_brightness() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.dt8_supported = Some(false);
        execution.has_dt8_color = false;
        let colour = colour(&execution).expect("brightness is an observation");
        assert_eq!(colour.mode, ColorMode::Brightness);
    }

    #[test]
    fn a_read_that_never_probed_publishes_nothing() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.dt8_supported = None;
        execution.has_dt8_color = false;
        execution.dt8_color.value_2 = Some(250);
        assert!(colour(&execution).is_none());
    }

    #[test]
    fn a_colour_capable_gear_this_read_skipped_publishes_nothing() {
        let mut execution = dt8_read(ColorMode::Cct);
        execution.has_dt8_color = false;
        execution.dt8_color.value_2 = Some(250);
        assert!(colour(&execution).is_none());
    }

    #[test]
    fn a_gear_that_named_no_active_mode_publishes_nothing() {
        for mode in [ColorMode::None, ColorMode::Unknown] {
            let mut execution = dt8_read(mode);
            execution.dt8_color.value_2 = Some(250);
            assert!(colour(&execution).is_none(), "{mode:?}");
        }
    }
}

#[test]
fn every_contract_attribute_group_survives_the_mask_round_trip() {
    use dali2rust_contracts::msg::DaliAttributeGroup;
    for group in DaliAttributeGroup::ALL {
        assert_eq!(
            super::attribute_read::decode_attribute_groups_mask(group.mask_bit()),
            vec![group],
            "{group:?} lost by the mask decode"
        );
    }
    let all_mask = DaliAttributeGroup::ALL
        .iter()
        .fold(0u8, |mask, group| mask | group.mask_bit());
    assert_eq!(
        super::attribute_read::decode_attribute_groups_mask(all_mask).len(),
        DaliAttributeGroup::ALL.len()
    );
}

#[test]
fn sequence_incomplete_never_pollutes_the_transport_abort_counter() {
    use std::sync::atomic::Ordering;
    let counters = super::DaliWorkerCounters::default();
    super::attribute_read::count_read_abort(
        crate::runtime::executor::SemanticDaliError::OperationFailed(
            crate::runtime::executor::discovery::DEVICE_TYPE_ENUM_INCOMPLETE,
        ),
        &counters,
    );
    assert_eq!(
        counters.read_attributes_sequence_incomplete.load(Ordering::Relaxed),
        1
    );
    assert_eq!(counters.read_attributes_transport_aborts.load(Ordering::Relaxed), 0);
}
