mod support;

use std::time::Duration;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    ColorMode, ColorValue, DaliTargetScope, DecodeStatus, DeviceType, LightSetpoint,
    ObservedFrameWidth, ObservedKind, PowerState,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{HaPublishReadPort, VirtualLampReadPort};
use dali2rust_test_support::wait_until;
use support::{publish_cmd, publish_event, seed_physical_via_discovery, spawn_registry_stack};

const WAIT: Duration = Duration::from_secs(2);
const GROUP: u8 = 3;

fn observed_group_frame(group_id: u8, setpoint: Option<LightSetpoint>, dapc: bool) -> dali2rust_contracts::msg::DaliObservedFrameEvent {
    dali2rust_contracts::msg::DaliObservedFrameEvent {
        registry_adapter_id: 0,
        observed_kind: ObservedKind::TargetStateObserved,
        scope: DaliTargetScope::Group,
        short_address: None,
        group_id: Some(group_id),
        scene_id: None,
        setpoint,
        dapc_observed: dapc,
        level_transition: None,
        raw_frame: [0x87, 0x80, 0x00],
        raw_width: ObservedFrameWidth::Forward16,
        decode_status: DecodeStatus::Decoded,
        observed_at_ms: 1_000,
        observed_at_mono_ms: 1_000,
    }
}

fn on_setpoint(level: u8) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::On,
        level,
        color: None,
    }
}

fn off_setpoint() -> LightSetpoint {
    LightSetpoint {
        power: PowerState::Off,
        level: 0,
        color: None,
    }
}

fn seed_lit_member(stack: &support::RegistryTestStack) {
    seed_physical_via_discovery(&stack.publisher, 1, 0, DeviceType::Dt6Led, &stack.store);
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            2,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::VirtualLampBindCommand {
                adapter_id: 0,
                virtual_lamp_id: 1,
                physical_short_address: 0,
            },
        ),
    );
    wait_until(
        || stack.store.virtual_lamp_view(0, 1).binding_short == Some(0),
        WAIT,
    );
    publish_event(
        &stack.publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            3,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliAttributesReadEvent {
                registry_adapter_id: 0,
                short_address: 0,
                last_chunk: true,
                chunk: dali2rust_contracts::msg::DaliAttributeReadChunk::Groups {
                    membership: Some(1u16 << GROUP),
                },
            },
        ),
    );
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            4,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(
                0,
                dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::sniffer_level(1, 180, 1_000),
            ),
        ),
    );
    wait_until(|| group_state(stack).any_on, WAIT);
}

fn group_state(stack: &support::RegistryTestStack) -> dali2rust_domain::registry::HaGroupStateView {
    stack
        .store
        .ha_group_views(0)
        .into_iter()
        .find(|g| g.group_id == GROUP)
        .map(|g| g.state)
        .unwrap_or_default()
}

#[test]
fn a_foreign_group_frame_arms_the_tile_and_a_group_off_clears_it() {
    let stack = spawn_registry_stack(1, 32);
    seed_lit_member(&stack);
    assert!(!group_state(&stack).commanded, "nothing commanded yet");

    publish_event(
        &stack.publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            5,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            observed_group_frame(GROUP, Some(on_setpoint(128)), true),
        ),
    );
    wait_until(|| group_state(&stack).commanded, WAIT);
    let state = group_state(&stack);
    assert!(state.commanded && state.any_on, "tile ON: commanded and lit");

    publish_event(
        &stack.publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            6,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            observed_group_frame(GROUP, Some(off_setpoint()), false),
        ),
    );
    wait_until(|| !group_state(&stack).commanded, WAIT);
    assert!(!group_state(&stack).commanded, "a group OFF disarms the tile");
}

#[test]
fn scene_recalls_and_colour_only_commands_never_arm_a_tile() {
    let stack = spawn_registry_stack(1, 32);
    seed_lit_member(&stack);

    publish_event(
        &stack.publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            7,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliSceneRecalledEvent {
                registry_adapter_id: 0,
                scope: DaliTargetScope::Broadcast,
                short_address: 0,
                group_id: 0,
                scene_id: 5,
                error: None,
                recalled_at_mono_ms: 2_000,
            },
        ),
    );
    wait_until(|| stack.store.ha_active_scene(0) == Some(5), WAIT);
    assert!(!group_state(&stack).commanded, "a scene recall arms no tile");

    publish_event(
        &stack.publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            8,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliTargetStateAppliedEvent {
                registry_adapter_id: 0,
                scope: DaliTargetScope::Group,
                virtual_lamp_id: None,
                short_address: None,
                group_id: Some(GROUP),
                setpoint: LightSetpoint {
                    power: PowerState::Unknown,
                    level: 0,
                    color: Some(ColorValue {
                        mode: ColorMode::Cct,
                        color_temperature_kelvin: 3000,
                        ..Default::default()
                    }),
                },
                dapc_applied: false,
                source: dali2rust_contracts::msg::RuntimeSource::Api,
                applied_at_mono_ms: 2_500,
            },
        ),
    );
    publish_event(
        &stack.publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            9,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliTargetStateAppliedEvent {
                registry_adapter_id: 0,
                scope: DaliTargetScope::Broadcast,
                virtual_lamp_id: None,
                short_address: None,
                group_id: None,
                setpoint: on_setpoint(1),
                dapc_applied: true,
                source: dali2rust_contracts::msg::RuntimeSource::Api,
                applied_at_mono_ms: 2_600,
            },
        ),
    );
    wait_until(|| group_state(&stack).commanded, WAIT);
    assert!(group_state(&stack).commanded, "broadcast ON arms every group");

    publish_event(
        &stack.publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            10,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliTargetStateAppliedEvent {
                registry_adapter_id: 0,
                scope: DaliTargetScope::Broadcast,
                virtual_lamp_id: None,
                short_address: None,
                group_id: None,
                setpoint: off_setpoint(),
                dapc_applied: false,
                source: dali2rust_contracts::msg::RuntimeSource::Api,
                applied_at_mono_ms: 2_700,
            },
        ),
    );
    wait_until(|| !group_state(&stack).commanded, WAIT);
    assert!(!group_state(&stack).commanded, "broadcast OFF clears every group");
}

#[test]
fn a_group_recall_clears_the_adapter_wide_active_scene() {
    let stack = spawn_registry_stack(1, 32);
    let recall = |corr: u64, scope: DaliTargetScope, group_id: u8, scene_id: u8| {
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliSceneRecalledEvent {
                registry_adapter_id: 0,
                scope,
                short_address: 0,
                group_id,
                scene_id,
                error: None,
                recalled_at_mono_ms: 3_000,
            },
        )
    };

    publish_event(&stack.publisher, recall(20, DaliTargetScope::Broadcast, 0, 5));
    wait_until(|| stack.store.ha_active_scene(0) == Some(5), WAIT);

    publish_event(&stack.publisher, recall(21, DaliTargetScope::Group, 7, 3));
    wait_until(|| stack.store.ha_active_scene(0).is_none(), WAIT);
    assert_eq!(
        stack.store.ha_active_scene(0),
        None,
        "a partial recall invalidates the adapter-wide claim and sets nothing"
    );
}
