mod support;

use std::time::Duration;

use dali2rust_contracts::msg::{
    ConfigWriteResource, GroupMatrixDesiredRow, GroupMatrixDesiredRowList,
};
use dali2rust_domain::registry::GroupReadPort;
use dali2rust_test_support::wait_until;

use support::{
    publish_cmd, publish_config_write_commit, publish_group_matrix_write, recv_confirm_for,
    spawn_registry_stack, RegistryTestStack,
};

fn rows(entries: &[(u8, u16)]) -> GroupMatrixDesiredRowList {
    let mut out = GroupMatrixDesiredRowList::new();
    for (virtual_lamp_id, desired_groups_mask) in entries {
        out.push(GroupMatrixDesiredRow {
            virtual_lamp_id: *virtual_lamp_id,
            desired_groups_mask: *desired_groups_mask,
        })
        .expect("row capacity");
    }
    out
}

fn chunk(stack: &RegistryTestStack, corr: u64, entries: &[(u8, u16)]) {
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            corr,
            dali2rust_bus::BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::GroupMatrixDesiredPatchCommand {
                adapter_id: 0,
                rows: rows(entries),
            },
        ),
    );
}

fn mask_of(stack: &RegistryTestStack, virtual_lamp_id: usize) -> Option<Vec<bool>> {
    stack
        .store
        .group_membership_matrix_view(0)
        .map(|view| view.rows[virtual_lamp_id].desired.to_vec())
}

fn desired_bit(stack: &RegistryTestStack, virtual_lamp_id: usize, group_id: usize) -> bool {
    mask_of(stack, virtual_lamp_id)
        .map(|desired| desired[group_id])
        .unwrap_or(false)
}

#[test]
fn a_bracketed_series_commits_every_chunk_at_once() {
    let stack = spawn_registry_stack(1, 64);
    let revision_before = stack.store.group_matrix_revision();

    chunk(&stack, 10, &[(1, 1 << 2)]);
    chunk(&stack, 10, &[(2, 1 << 3)]);
    publish_config_write_commit(
        &stack.publisher,
        10,
        ConfigWriteResource::GroupMatrix,
        0,
        None,
        2,
    );

    wait_until(
        || desired_bit(&stack, 1, 2) && desired_bit(&stack, 2, 3),
        Duration::from_millis(500),
    );
    assert_eq!(
        stack.store.group_matrix_revision(),
        revision_before.wrapping_add(1),
        "a two-chunk series must bump the revision once"
    );
}

#[test]
fn an_unbracketed_series_leaves_the_matrix_untouched() {
    let stack = spawn_registry_stack(1, 64);
    let revision_before = stack.store.group_matrix_revision();

    chunk(&stack, 20, &[(1, 1 << 2)]);
    chunk(&stack, 20, &[(2, 1 << 3)]);

    recv_confirm_for(&stack.conf_rx, 20);
    recv_confirm_for(&stack.conf_rx, 20);

    assert!(
        !desired_bit(&stack, 1, 2) && !desired_bit(&stack, 2, 3),
        "chunks must stage, never apply on their own"
    );
    assert_eq!(stack.store.group_matrix_revision(), revision_before);
}

#[test]
fn a_chunk_count_mismatch_refuses_the_whole_series() {
    let stack = spawn_registry_stack(1, 64);
    let revision_before = stack.store.group_matrix_revision();

    chunk(&stack, 30, &[(1, 1 << 2)]);
    publish_config_write_commit(
        &stack.publisher,
        30,
        ConfigWriteResource::GroupMatrix,
        0,
        None,
        3,
    );

    recv_confirm_for(&stack.conf_rx, 30);
    recv_confirm_for(&stack.conf_rx, 30);

    assert!(
        !desired_bit(&stack, 1, 2),
        "a short series must not commit the chunks that did arrive"
    );
    assert_eq!(stack.store.group_matrix_revision(), revision_before);
}

#[test]
fn a_bracket_without_a_series_is_refused() {
    let stack = spawn_registry_stack(1, 64);
    let revision_before = stack.store.group_matrix_revision();

    publish_config_write_commit(
        &stack.publisher,
        40,
        ConfigWriteResource::GroupMatrix,
        0,
        None,
        1,
    );

    recv_confirm_for(&stack.conf_rx, 40);
    assert_eq!(stack.store.group_matrix_revision(), revision_before);
}

#[test]
fn a_refused_series_does_not_contaminate_the_next_one() {
    let stack = spawn_registry_stack(1, 64);

    chunk(&stack, 50, &[(1, 1 << 2)]);
    publish_config_write_commit(
        &stack.publisher,
        50,
        ConfigWriteResource::GroupMatrix,
        0,
        None,
        9,
    );
    recv_confirm_for(&stack.conf_rx, 50);
    recv_confirm_for(&stack.conf_rx, 50);

    publish_group_matrix_write(&stack.publisher, 60, 0, rows(&[(2, 1 << 3)]));

    wait_until(|| desired_bit(&stack, 2, 3), Duration::from_millis(500));
    assert!(
        !desired_bit(&stack, 1, 2),
        "rows from the refused series must not ride along with the next write"
    );
}

#[test]
fn a_stray_bracket_does_not_wipe_the_newer_series_it_refuses() {
    let stack = spawn_registry_stack(1, 64);

    chunk(&stack, 80, &[(1, 1 << 2)]);
    recv_confirm_for(&stack.conf_rx, 80);
    chunk(&stack, 81, &[(2, 1 << 3)]);
    recv_confirm_for(&stack.conf_rx, 81);

    publish_config_write_commit(
        &stack.publisher,
        80,
        ConfigWriteResource::GroupMatrix,
        0,
        None,
        1,
    );
    recv_confirm_for(&stack.conf_rx, 80);

    publish_config_write_commit(
        &stack.publisher,
        81,
        ConfigWriteResource::GroupMatrix,
        0,
        None,
        1,
    );
    wait_until(|| desired_bit(&stack, 2, 3), Duration::from_millis(500));
    assert!(
        !desired_bit(&stack, 1, 2),
        "the dead series' rows must never land"
    );
}

#[test]
fn a_retry_replaces_an_abandoned_series_instead_of_extending_it() {
    let stack = spawn_registry_stack(1, 64);
    let revision_before = stack.store.group_matrix_revision();

    chunk(&stack, 70, &[(1, 1 << 2)]);
    recv_confirm_for(&stack.conf_rx, 70);

    chunk(&stack, 71, &[(2, 1 << 3)]);
    publish_config_write_commit(
        &stack.publisher,
        71,
        ConfigWriteResource::GroupMatrix,
        0,
        None,
        1,
    );

    wait_until(|| desired_bit(&stack, 2, 3), Duration::from_millis(500));
    assert!(
        !desired_bit(&stack, 1, 2),
        "the abandoned series' rows must not be committed by the retry"
    );
    assert_eq!(
        stack.store.group_matrix_revision(),
        revision_before.wrapping_add(1),
        "the retry commits once, and only its own rows"
    );
}
