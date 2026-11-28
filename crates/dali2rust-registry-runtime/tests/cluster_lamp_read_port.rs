mod support;

use std::time::Duration;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    ColorMode, DeviceType, LightSetpoint, Origin, PowerState, RegistryRuntimeUpdateCommand,
    RuntimeObservation, RuntimeRegistryUpdateEntry, RuntimeSource,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::ClusterLampReadPort;
use dali2rust_test_support::wait_until;
use support::{publish_cmd, seed_bound_group_member, spawn_registry_stack, RegistryTestStack};

const WAIT: Duration = Duration::from_secs(2);
const GROUP: u8 = 5;
const LIT_LAMP: u8 = 1;
const LIT_SHORT: u8 = 0;
const SILENT_LAMP: u8 = 2;
const SILENT_SHORT: u8 = 1;
const ABSENT_LAMP: u8 = 3;
const ABSENT_SHORT: u8 = 2;
const LIT_LEVEL: u8 = 180;

fn publish_entry(stack: &RegistryTestStack, corr: u64, entry: RuntimeRegistryUpdateEntry) {
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(Origin::Internal),
            RegistryRuntimeUpdateCommand::internal(0, entry),
        ),
    );
}

fn absence(short_address: u8) -> RuntimeRegistryUpdateEntry {
    RuntimeRegistryUpdateEntry {
        virtual_lamp_id: None,
        short_address: Some(short_address),
        setpoint: Some(LightSetpoint::default()),
        observation: Some(RuntimeObservation::device_absent(RuntimeSource::Poller)),
        last_dapc_source: None,
        source: RuntimeSource::Poller,
        observed_at_mono_ms: None,
        hold_hcl: true,
    }
}

fn three_members() -> RegistryTestStack {
    let stack = spawn_registry_stack(1, 64);
    seed_bound_group_member(&stack, LIT_LAMP, LIT_SHORT, GROUP, 10);
    seed_bound_group_member(&stack, SILENT_LAMP, SILENT_SHORT, GROUP, 20);
    seed_bound_group_member(&stack, ABSENT_LAMP, ABSENT_SHORT, GROUP, 30);
    publish_entry(&stack, 40, RuntimeRegistryUpdateEntry::sniffer_level(LIT_LAMP, LIT_LEVEL, 1_000));
    publish_entry(&stack, 41, RuntimeRegistryUpdateEntry::sniffer_level(ABSENT_LAMP, LIT_LEVEL, 1_000));
    publish_entry(&stack, 42, absence(ABSENT_SHORT));
    wait_until(
        || stack.store.cluster_lamp_state(0, ABSENT_LAMP).is_some_and(|state| state.absent),
        WAIT,
    );
    stack
}

#[test]
fn a_lit_lamp_reads_with_its_level_and_its_source() {
    let stack = three_members();
    let state = stack.store.cluster_lamp_state(0, LIT_LAMP).expect("the lamp exists");
    assert_eq!(state.power, PowerState::On);
    assert_eq!(state.level, Some(LIT_LEVEL));
    assert_eq!(state.value_source, Some(RuntimeSource::Sniffer));
    assert!(!state.absent);
    let ids = stack.store.cluster_lamp_ids(0);
    assert_eq!(ids, (1 << LIT_LAMP) | (1 << SILENT_LAMP) | (1 << ABSENT_LAMP));
    assert_eq!(stack.store.cluster_lamp_state(0, 9), None, "a lamp that does not exist reads as nothing");
}

#[test]
fn a_lamp_never_observed_reads_unknown_not_off() {
    let stack = three_members();
    let state = stack.store.cluster_lamp_state(0, SILENT_LAMP).expect("the lamp exists");
    assert_eq!(state.power, PowerState::Unknown);
    assert_eq!(state.level, None, "an unknown level is None, never 0");
    assert_eq!(state.observed_at_ms, None);
}

#[test]
fn the_info_names_the_lamp_and_what_its_gear_can_do() {
    let stack = three_members();
    let info = stack.store.cluster_lamp_info(0, LIT_LAMP).expect("the lamp exists");
    assert_eq!(info.device_type, DeviceType::Dt8Color);
    assert!(info.bound);
    assert!(info.ha_entity);
    assert!(info.capabilities.brightness);
    assert_ne!(info.color_mode, ColorMode::None);
    assert!(stack.store.cluster_lamp_info(0, 9).is_none());
}

#[test]
fn a_group_counts_lit_known_and_members_and_an_absent_member_is_not_known() {
    let stack = three_members();
    let lit = stack.store.cluster_group_lit(0, GROUP);
    assert!(!lit.commanded, "nothing commanded the group");
    assert_eq!((lit.lit, lit.known, lit.members), (1, 1, 3));
}
