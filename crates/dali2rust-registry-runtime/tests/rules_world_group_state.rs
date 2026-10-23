mod support;

use std::time::Duration;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    fixed_text_64, LightSetpoint, PowerState, RuntimeRegistryUpdateEntry,
    VirtualLampConfigUpdateCommand, VirtualLampUnbindCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_registry_runtime::RulesLampRow;
use dali2rust_test_support::wait_until;
use support::{core_view_of, publish_cmd, seed_bound_group_member, spawn_registry_stack};

const WAIT: Duration = Duration::from_secs(2);

const LIT: (u8, u8, u8) = (11, 12, 1);
const DARK: (u8, u8, u8) = (12, 11, 2);

fn any_on(stack: &support::RegistryTestStack, group_id: u8) -> bool {
    stack
        .store
        .rules_group_rows()
        .into_iter()
        .find(|row| row.id == u16::from(group_id))
        .map(|row| row.any_on)
        .unwrap_or(false)
}

fn commit_power(stack: &support::RegistryTestStack, correlation_id: u64, lamp: (u8, u8, u8), on: bool) {
    let setpoint = LightSetpoint {
        power: if on { PowerState::On } else { PowerState::Off },
        level: if on { Some(200) } else { Some(0) },
        color: None,
    };
    commit_setpoint(stack, correlation_id, lamp, &setpoint);
}

fn commit_switch_on_without_level(
    stack: &support::RegistryTestStack,
    correlation_id: u64,
    lamp: (u8, u8, u8),
) {
    let setpoint = LightSetpoint {
        power: PowerState::On,
        level: None,
        color: None,
    };
    commit_setpoint(stack, correlation_id, lamp, &setpoint);
}

fn commit_setpoint(
    stack: &support::RegistryTestStack,
    correlation_id: u64,
    lamp: (u8, u8, u8),
    setpoint: &LightSetpoint,
) {
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(
                0,
                RuntimeRegistryUpdateEntry::api_virtual_lamp_and_short(lamp.0, lamp.1, setpoint, 5_000 + correlation_id),
            ),
        ),
    );
}

#[test]
fn a_group_reads_the_device_its_member_is_bound_to() {
    let stack = spawn_registry_stack(1, 32);
    seed_bound_group_member(&stack, LIT.0, LIT.1, LIT.2, 10);
    seed_bound_group_member(&stack, DARK.0, DARK.1, DARK.2, 20);

    commit_power(&stack, 30, LIT, true);
    commit_power(&stack, 31, DARK, false);

    wait_until(|| any_on(&stack, LIT.2), WAIT);
    assert!(any_on(&stack, LIT.2), "the lit member's own group");
    assert!(
        !any_on(&stack, DARK.2),
        "group {} has one member, bound to short address {}, and it is dark",
        DARK.2,
        DARK.1
    );
}

#[test]
fn every_group_reports_state_without_a_metadata_record() {
    let stack = spawn_registry_stack(1, 32);
    seed_bound_group_member(&stack, LIT.0, LIT.1, LIT.2, 10);
    commit_power(&stack, 30, LIT, true);
    wait_until(|| any_on(&stack, LIT.2), WAIT);

    let rows = stack.store.rules_group_rows();
    assert_eq!(rows.len(), 16, "one row per group of the one adapter");
    for group_id in 0..16u8 {
        let row = rows
            .iter()
            .find(|row| row.id == u16::from(group_id))
            .unwrap_or_else(|| panic!("no row for group {group_id}"));
        if group_id == LIT.2 {
            assert!(row.any_on, "the seeded group is lit");
            assert!(!row.all_off(), "a lit group is not all off");
            assert_eq!(row.member_count, 1, "one member");
        } else {
            assert!(!row.any_on, "group {group_id} has no lit member");
            assert!(row.all_off(), "group {group_id} has nothing on");
            assert_eq!(row.member_count, 0, "group {group_id} has no members");
        }
    }
}

#[test]
fn a_member_switched_on_without_a_level_lights_its_group() {
    let stack = spawn_registry_stack(1, 32);
    seed_bound_group_member(&stack, LIT.0, LIT.1, LIT.2, 10);

    commit_power(&stack, 30, LIT, true);
    wait_until(|| any_on(&stack, LIT.2), WAIT);
    commit_power(&stack, 31, LIT, false);
    wait_until(|| !any_on(&stack, LIT.2), WAIT);

    commit_switch_on_without_level(&stack, 32, LIT);
    wait_until(|| any_on(&stack, LIT.2), WAIT);
    assert!(
        any_on(&stack, LIT.2),
        "a member told to switch on is lit, whatever level the gear recalled"
    );

    let level = core_view_of(&stack.store, LIT.1).state.level;
    assert_eq!(
        level,
        Some(200),
        "the recall projects the shadowed last-active level"
    );
}

#[test]
fn a_lamp_switched_on_without_a_level_reads_on() {
    let stack = spawn_registry_stack(1, 32);
    seed_bound_group_member(&stack, LIT.0, LIT.1, LIT.2, 10);
    commit_power(&stack, 30, LIT, true);
    wait_until(|| any_on(&stack, LIT.2), WAIT);
    commit_power(&stack, 31, LIT, false);
    wait_until(|| !any_on(&stack, LIT.2), WAIT);

    commit_switch_on_without_level(&stack, 32, LIT);
    let is_on = |stack: &support::RegistryTestStack| {
        stack
            .store
            .rules_lamp_rows()
            .into_iter()
            .find(|row| row.id == u16::from(LIT.0))
            .map(|row| row.is_on)
            .unwrap_or(false)
    };
    wait_until(|| is_on(&stack), WAIT);
    assert!(is_on(&stack), "the lamp the rules engine reads is on");
}

const NAMED_ONLY: u8 = 13;

const LIT_SEEN_AT_MS: u64 = 5_030;
const DARK_SEEN_AT_MS: u64 = 5_031;
const ONLINE_WINDOW_MS: u64 = 15 * 60 * 1_000;
const DIM_LEVEL: u8 = 200;

fn lamp_row(stack: &support::RegistryTestStack, lamp_id: u8) -> Option<RulesLampRow> {
    stack
        .store
        .rules_lamp_rows()
        .into_iter()
        .find(|row| row.id == u16::from(lamp_id))
}

fn lamp_bound(stack: &support::RegistryTestStack, lamp_id: u8) -> Option<bool> {
    stack
        .store
        .rules_lamp_rows()
        .into_iter()
        .find(|row| row.id == u16::from(lamp_id))
        .map(|row| row.bound)
}

fn api_command<P>(stack: &support::RegistryTestStack, correlation_id: u64, payload: P)
where
    dali2rust_contracts::msg::BusCommandPayload: From<P>,
{
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            payload,
        ),
    );
}

#[test]
fn a_lamp_reads_bound_only_while_its_binding_stands() {
    let stack = spawn_registry_stack(1, 32);
    api_command(
        &stack,
        40,
        VirtualLampConfigUpdateCommand {
            adapter_id: 0,
            virtual_lamp_id: NAMED_ONLY,
            patch_mask: VirtualLampConfigUpdateCommand::PATCH_NAME,
            name: fixed_text_64("hall"),
            ha_entity_enabled: false,
        },
    );
    wait_until(|| lamp_bound(&stack, NAMED_ONLY).is_some(), WAIT);
    assert_eq!(lamp_bound(&stack, NAMED_ONLY), Some(false), "a lamp created by its name alone");

    seed_bound_group_member(&stack, LIT.0, LIT.1, LIT.2, 10);
    assert_eq!(lamp_bound(&stack, LIT.0), Some(true), "a lamp once it is bound");

    api_command(&stack, 41, VirtualLampUnbindCommand { adapter_id: 0, virtual_lamp_id: LIT.0 });
    wait_until(|| lamp_bound(&stack, LIT.0) == Some(false), WAIT);
    assert_eq!(
        lamp_bound(&stack, LIT.0),
        Some(false),
        "the lamp stays in the world after its binding is removed, and reads unbound"
    );
}

#[test]
fn a_lamp_row_carries_the_level_and_a_last_level_that_is_never_zero() {
    let stack = spawn_registry_stack(1, 32);
    seed_bound_group_member(&stack, LIT.0, LIT.1, LIT.2, 10);
    commit_power(&stack, 30, LIT, true);
    wait_until(|| lamp_row(&stack, LIT.0).is_some_and(|row| row.is_on), WAIT);
    let on = lamp_row(&stack, LIT.0).expect("the lamp row");
    assert_eq!((on.level, on.last_level), (Some(DIM_LEVEL), Some(DIM_LEVEL)));
    assert_eq!(on.cct_kelvin, None, "no colour temperature was observed");

    commit_power(&stack, 31, LIT, false);
    wait_until(|| lamp_row(&stack, LIT.0).is_some_and(|row| !row.is_on), WAIT);
    let off = lamp_row(&stack, LIT.0).expect("the lamp row");
    assert_eq!(
        (off.level, off.last_level),
        (Some(0), Some(1)),
        "a level of 0 is a level, and the level to come back to is at least 1"
    );
}

#[test]
fn an_unbound_lamp_row_has_no_level() {
    let stack = spawn_registry_stack(1, 32);
    api_command(
        &stack,
        40,
        VirtualLampConfigUpdateCommand {
            adapter_id: 0,
            virtual_lamp_id: NAMED_ONLY,
            patch_mask: VirtualLampConfigUpdateCommand::PATCH_NAME,
            name: fixed_text_64("hall"),
            ha_entity_enabled: false,
        },
    );
    wait_until(|| lamp_row(&stack, NAMED_ONLY).is_some(), WAIT);
    let row = lamp_row(&stack, NAMED_ONLY).expect("the lamp row");
    assert_eq!((row.is_on, row.level, row.last_level), (false, None, None));
}

#[test]
fn device_rows_are_ordered_and_online_for_fifteen_minutes_after_the_last_sighting() {
    let stack = spawn_registry_stack(1, 32);
    seed_bound_group_member(&stack, LIT.0, LIT.1, LIT.2, 10);
    seed_bound_group_member(&stack, DARK.0, DARK.1, DARK.2, 20);
    commit_power(&stack, 30, LIT, true);
    commit_power(&stack, 31, DARK, false);
    let seen = |now: u64| -> Vec<(u8, bool)> {
        stack
            .store
            .rules_device_rows(now)
            .into_iter()
            .map(|row| (row.short_address, row.online))
            .collect()
    };
    wait_until(|| seen(DARK_SEEN_AT_MS) == [(DARK.1, true), (LIT.1, true)], WAIT);
    assert_eq!(seen(DARK_SEEN_AT_MS), [(DARK.1, true), (LIT.1, true)], "ordered by short address");

    let at_window_end = LIT_SEEN_AT_MS + ONLINE_WINDOW_MS;
    assert_eq!(
        seen(at_window_end),
        [(DARK.1, true), (LIT.1, false)],
        "the window is open for 15 minutes and closes on its last millisecond"
    );
}
