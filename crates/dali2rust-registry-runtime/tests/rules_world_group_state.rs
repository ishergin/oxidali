mod support;

use std::time::Duration;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{LightSetpoint, PowerState, RuntimeRegistryUpdateEntry};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_test_support::wait_until;
use support::{publish_cmd, seed_bound_group_member, spawn_registry_stack};

const WAIT: Duration = Duration::from_secs(2);

const LIT: (u8, u8, u8) = (11, 12, 1);
const DARK: (u8, u8, u8) = (12, 11, 2);

fn any_on(stack: &support::RegistryTestStack, group_id: u8) -> bool {
    stack
        .store
        .rules_group_rows()
        .into_iter()
        .find(|row| row.1 == u16::from(group_id))
        .map(|row| row.2)
        .unwrap_or(false)
}

fn commit_power(stack: &support::RegistryTestStack, correlation_id: u64, lamp: (u8, u8, u8), on: bool) {
    let setpoint = LightSetpoint {
        power: if on { PowerState::On } else { PowerState::Off },
        level: if on { 200 } else { 0 },
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
        level: 0,
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
            .find(|row| row.1 == u16::from(group_id))
            .unwrap_or_else(|| panic!("no row for group {group_id}"));
        if group_id == LIT.2 {
            assert!(row.2, "the seeded group is lit");
            assert_eq!(row.3, 1, "one member");
        } else {
            assert!(!row.2, "group {group_id} has no lit member");
            assert_eq!(row.3, 0, "group {group_id} has no members");
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

    let level = stack
        .store
        .physical_device_view(0, LIT.1)
        .expect("seeded device")
        .state
        .level;
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
            .find(|row| row.1 == u16::from(LIT.0))
            .map(|row| row.2)
            .unwrap_or(false)
    };
    wait_until(|| is_on(&stack), WAIT);
    assert!(is_on(&stack), "the lamp the rules engine reads is on");
}
