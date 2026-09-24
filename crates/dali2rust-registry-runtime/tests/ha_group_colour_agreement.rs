mod support;

use std::time::Duration;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    ColorMode, ColorValue, LightSetpoint, PowerState, RuntimeRegistryUpdateEntry,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::HaPublishReadPort;
use dali2rust_test_support::wait_until;
use support::{publish_cmd, seed_bound_group_member, spawn_registry_stack};

const WAIT: Duration = Duration::from_secs(2);
const GROUP: u8 = 3;

fn group_state(stack: &support::RegistryTestStack) -> dali2rust_domain::registry::HaGroupStateView {
    stack
        .store
        .ha_group_views(0)
        .into_iter()
        .find(|g| g.group_id == GROUP)
        .map(|g| g.state)
        .unwrap_or_default()
}

fn seed_member(stack: &support::RegistryTestStack, virtual_lamp_id: u8, short: u8, corr: u64) {
    seed_bound_group_member(stack, virtual_lamp_id, short, GROUP, corr);
}

fn commit_colour(
    stack: &support::RegistryTestStack,
    corr: u64,
    virtual_lamp_id: u8,
    short: u8,
    color: ColorValue,
) {
    let setpoint = LightSetpoint {
        power: PowerState::On,
        level: 200,
        color: Some(color),
    };
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(
                0,
                RuntimeRegistryUpdateEntry::api_virtual_lamp_and_short(
                    virtual_lamp_id,
                    short,
                    &setpoint,
                    5_000 + corr,
                ),
            ),
        ),
    );
}

fn cct(kelvin: u16) -> ColorValue {
    ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: kelvin,
        ..Default::default()
    }
}

fn rgb(r: u8, g: u8, b: u8) -> ColorValue {
    ColorValue {
        mode: ColorMode::Rgb,
        r,
        g,
        b,
        ..Default::default()
    }
}

fn xy(x: u16, y: u16) -> ColorValue {
    ColorValue {
        mode: ColorMode::Xy,
        x,
        y,
        ..Default::default()
    }
}

#[test]
fn a_mixed_rgb_and_cct_group_reports_the_picked_rgb() {
    let stack = spawn_registry_stack(1, 32);
    seed_member(&stack, 1, 0, 10);
    seed_member(&stack, 2, 1, 20);
    commit_colour(&stack, 30, 1, 0, rgb(255, 120, 40));
    commit_colour(&stack, 31, 2, 1, cct(3000));

    wait_until(|| group_state(&stack).rgb.is_some(), WAIT);
    let state = group_state(&stack);
    assert_eq!(state.color_mode, Some(ColorMode::Rgb));
    assert_eq!(state.rgb, Some((255, 120, 40)));
    assert_eq!(
        state.color_temperature_kelvin, None,
        "the white the CCT panel stayed at is not the group's colour"
    );
}

#[test]
fn an_agreeing_xy_group_reports_its_pair() {
    let stack = spawn_registry_stack(1, 32);
    seed_member(&stack, 1, 0, 10);
    seed_member(&stack, 2, 1, 20);
    commit_colour(&stack, 30, 1, 0, xy(29_491, 26_869));
    commit_colour(&stack, 31, 2, 1, xy(29_491, 26_869));

    wait_until(|| group_state(&stack).xy.is_some(), WAIT);
    let state = group_state(&stack);
    assert_eq!(state.color_mode, Some(ColorMode::Xy));
    let (x, y) = state.xy.expect("agreed pair");
    assert!(
        (x - 0.45).abs() < 0.001 && (y - 0.41).abs() < 0.001,
        "product-space chromaticity, decoded once at commit: got ({x}, {y})"
    );
    assert_eq!(state.rgb, None);
    assert_eq!(state.color_temperature_kelvin, None);
}

#[test]
fn a_disagreeing_xy_pair_reports_no_colour() {
    let stack = spawn_registry_stack(1, 32);
    seed_member(&stack, 1, 0, 10);
    seed_member(&stack, 2, 1, 20);
    commit_colour(&stack, 30, 1, 0, xy(29_491, 26_869));
    commit_colour(&stack, 31, 2, 1, xy(45_000, 20_000));

    wait_until(
        || {
            let s = group_state(&stack);
            s.any_on && s.brightness.is_some()
        },
        WAIT,
    );
    wait_until(
        || group_state(&stack).xy.is_none(),
        WAIT,
    );
    let state = group_state(&stack);
    assert_eq!(state.color_mode, None, "a split vote names no colour");
    assert_eq!(state.xy, None);
}
