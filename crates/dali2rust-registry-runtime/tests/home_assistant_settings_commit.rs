mod support;

use std::sync::atomic::Ordering;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    fixed_text_32, fixed_text_48, fixed_text_64, BusCommandPayload, BusEventPayload, CommandEnvelope,
    DeliveryStatus, HomeAssistantCredentialsUpdateCommand, HomeAssistantSettingsCommitCommand,
    HomeAssistantSettingsUpdateCommand, HomeAssistantTopicsUpdateCommand,
};
use dali2rust_domain::registry::HomeAssistantSettingsReadPort;
use dali2rust_test_support::bus::recv_event_matching;

use support::{
    assert_exec_failed_msg, publish_cmd, recv_confirm_for, spawn_registry_stack, RegistryTestStack,
};

const EVENT_DEADLINE: std::time::Duration = std::time::Duration::from_millis(500);

fn command(corr: u64, payload: impl Into<BusCommandPayload>) -> CommandEnvelope {
    dali2rust_contracts::bus::command_envelope(0, corr, BusId::default().0, None, payload)
}

fn enable_broker(series: u64) -> HomeAssistantSettingsUpdateCommand {
    HomeAssistantSettingsUpdateCommand {
        series,
        patch_mask: HomeAssistantSettingsUpdateCommand::PATCH_ENABLED
            | HomeAssistantSettingsUpdateCommand::PATCH_BROKER_HOST,
        enabled: true,
        broker_host: fixed_text_64("broker.test"),
        broker_port: 0,
        publish_qos: 0,
        retain_state: false,
        retain_discovery: false,
        expose_input_devices: false,
    }
}

fn username(series: u64) -> HomeAssistantCredentialsUpdateCommand {
    HomeAssistantCredentialsUpdateCommand {
        series,
        patch_mask: HomeAssistantCredentialsUpdateCommand::PATCH_BROKER_USERNAME,
        broker_username: fixed_text_32("hiltest"),
        broker_password: fixed_text_48(""),
    }
}

fn discovery_prefix(series: u64) -> HomeAssistantTopicsUpdateCommand {
    HomeAssistantTopicsUpdateCommand {
        series,
        patch_mask: HomeAssistantTopicsUpdateCommand::PATCH_DISCOVERY_PREFIX,
        discovery_prefix: fixed_text_32("staged"),
        state_topic_prefix: fixed_text_32(""),
    }
}

fn stage_ok(stack: &RegistryTestStack, corr: u64, payload: impl Into<BusCommandPayload>) {
    publish_cmd(&stack.publisher, command(corr, payload));
    assert_eq!(recv_confirm_for(&stack.conf_rx, corr).status, DeliveryStatus::Ok);
}

fn applied(stack: &RegistryTestStack) -> u32 {
    stack.counters.command.home_assistant_settings_applied.load(Ordering::Relaxed)
}

#[test]
fn staged_frames_change_nothing_until_one_commit_lands_them_together() {
    let stack = spawn_registry_stack(1, 16);
    let series = 10;
    stage_ok(&stack, 11, enable_broker(series));
    stage_ok(&stack, 12, username(series));
    stage_ok(&stack, 13, discovery_prefix(series));

    let staged = stack.store.home_assistant_settings_view();
    assert!(!staged.enabled, "a staged frame must not reach the live record");
    assert_eq!(staged.discovery_prefix, "homeassistant");
    assert_eq!(applied(&stack), 0, "a staged frame is not a commit");

    stage_ok(&stack, series, HomeAssistantSettingsCommitCommand { series, frames: 3 });
    let view = stack.store.home_assistant_settings_view();
    assert!(view.enabled);
    assert_eq!(view.broker_host, "broker.test");
    assert_eq!(view.broker_username, "hiltest");
    assert_eq!(view.discovery_prefix, "staged");
    assert_eq!(applied(&stack), 1, "one PATCH is one commit for the bridge to act on");

    let changed = recv_event_matching(&stack.ev_rx, EVENT_DEADLINE, |p| {
        matches!(p, BusEventPayload::HomeAssistantSettingsChangedEvent(_))
    });
    assert_eq!(
        changed.meta.correlation_id, series,
        "the first change announced is the commit's, so no staged frame announced one"
    );
}

#[test]
fn a_commit_that_counts_other_frames_than_were_staged_lands_none_of_them() {
    let stack = spawn_registry_stack(1, 16);
    let series = 20;
    stage_ok(&stack, 21, enable_broker(series));
    stage_ok(&stack, 22, username(series));

    publish_cmd(
        &stack.publisher,
        command(series, HomeAssistantSettingsCommitCommand { series, frames: 3 }),
    );
    assert_exec_failed_msg(&recv_confirm_for(&stack.conf_rx, series), "home_assistant_frame_mismatch");
    assert!(!stack.store.home_assistant_settings_view().enabled);

    publish_cmd(
        &stack.publisher,
        command(23, HomeAssistantSettingsCommitCommand { series, frames: 2 }),
    );
    assert_exec_failed_msg(&recv_confirm_for(&stack.conf_rx, 23), "home_assistant_no_stage");
    assert!(!stack.store.home_assistant_settings_view().enabled);
    assert_eq!(applied(&stack), 0);
}

#[test]
fn a_new_series_discards_the_frames_of_one_never_committed() {
    let stack = spawn_registry_stack(1, 16);
    stage_ok(&stack, 31, enable_broker(30));
    stage_ok(&stack, 41, discovery_prefix(40));
    stage_ok(&stack, 40, HomeAssistantSettingsCommitCommand { series: 40, frames: 1 });

    let view = stack.store.home_assistant_settings_view();
    assert_eq!(view.discovery_prefix, "staged");
    assert!(!view.enabled, "the abandoned series must not ride on the next commit");

    publish_cmd(
        &stack.publisher,
        command(30, HomeAssistantSettingsCommitCommand { series: 30, frames: 1 }),
    );
    assert_exec_failed_msg(&recv_confirm_for(&stack.conf_rx, 30), "home_assistant_no_stage");
    assert_eq!(applied(&stack), 1);
}

#[test]
fn a_commit_of_another_series_lands_nothing() {
    let stack = spawn_registry_stack(1, 16);
    stage_ok(&stack, 51, enable_broker(50));

    publish_cmd(
        &stack.publisher,
        command(60, HomeAssistantSettingsCommitCommand { series: 60, frames: 1 }),
    );
    assert_exec_failed_msg(&recv_confirm_for(&stack.conf_rx, 60), "home_assistant_no_stage");
    assert!(!stack.store.home_assistant_settings_view().enabled);

    stage_ok(&stack, 50, HomeAssistantSettingsCommitCommand { series: 50, frames: 1 });
    assert!(stack.store.home_assistant_settings_view().enabled);
    assert_eq!(applied(&stack), 1);
}
