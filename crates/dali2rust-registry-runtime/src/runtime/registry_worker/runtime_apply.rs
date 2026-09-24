use std::sync::atomic::Ordering;

use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{
    ErrorCode, LightSetpoint, RegistryRuntimeUpdateCommand, RuntimeObservation,
    RuntimeRegistryUpdateEntry,
};

use crate::runtime::registry::physical_devices::{PhysicalRuntimeCommit, RuntimeCommitOutcome};
use crate::runtime::registry::publish::publish_runtime_state_changed;
use crate::runtime::registry::RegistryStore;

use super::confirm::{
    check_adapter_id, check_primary_adapter, publish_correlation_failed, publish_correlation_ok,
};
use dali2rust_bus::BusId;

use super::RegistryCommandCounters;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeApplyOutcome {
    Applied,
    NotApplied,
    VlUnbound,
    Superseded,
    Unchanged,
}

pub(super) fn handle_runtime_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    reg: &RegistryRuntimeUpdateCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, reg.adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    let entry = &reg.update;
    let Some(setpoint) = entry.setpoint.as_ref() else {
        return fail_runtime_invalid(publisher, corr, "registry_runtime_missing_setpoint");
    };
    let Some(obs) = entry.observation.as_ref() else {
        return fail_runtime_invalid(publisher, corr, "registry_runtime_missing_observation");
    };
    if entry.virtual_lamp_id.is_none() && entry.short_address.is_none() {
        return fail_runtime_invalid(publisher, corr, "registry_runtime_missing_target");
    }

    let applied = match apply_registry_runtime_targets(
        publisher,
        counters,
        corr,
        reg.adapter_id,
        entry,
        setpoint,
        obs,
        store,
    ) {
        Ok(outcome) => outcome,
        Err(()) => return,
    };
    finish_runtime_update_outcome(publisher, corr, counters, applied);
}

fn fail_runtime_invalid(publisher: &BusPublisher, corr: u64, reason: &'static str) {
    publish_correlation_failed(publisher, corr, ErrorCode::InvalidValue, reason);
}

fn finish_runtime_update_outcome(
    publisher: &BusPublisher,
    corr: u64,
    counters: &RegistryCommandCounters,
    outcome: RuntimeApplyOutcome,
) {
    match outcome {
        RuntimeApplyOutcome::Applied => {
            counters
                .runtime_updates_applied
                .fetch_add(1, Ordering::Relaxed);
            publish_correlation_ok(publisher, corr);
        }
        RuntimeApplyOutcome::NotApplied => {
            publish_correlation_failed(
                publisher,
                corr,
                ErrorCode::OperationFailed,
                "registry_runtime_not_applied",
            );
        }
        RuntimeApplyOutcome::VlUnbound => {
            publish_correlation_failed(publisher, corr, ErrorCode::VlUnbound, "vl_unbound");
        }
        RuntimeApplyOutcome::Superseded => {
            counters
                .runtime_updates_superseded
                .fetch_add(1, Ordering::Relaxed);
            publish_correlation_ok(publisher, corr);
        }
        RuntimeApplyOutcome::Unchanged => {
            counters
                .runtime_updates_applied
                .fetch_add(1, Ordering::Relaxed);
            publish_correlation_ok(publisher, corr);
        }
    }
}

fn apply_registry_runtime_targets(
    publisher: &BusPublisher,
    counters: &RegistryCommandCounters,
    corr: u64,
    aid: u8,
    entry: &RuntimeRegistryUpdateEntry,
    setpoint: &LightSetpoint,
    obs: &RuntimeObservation,
    store: &RegistryStore,
) -> Result<RuntimeApplyOutcome, ()> {
    let (vl, sa) = match (entry.virtual_lamp_id, entry.short_address) {
        (Some(vl_id), Some(sa_cmd)) => {
            check_runtime_binding_match(publisher, store, corr, aid, vl_id, sa_cmd)?;
            (Some(vl_id), sa_cmd)
        }
        (Some(vl_id), None) => match store.internal_virtual_lamp_binding_short(aid, vl_id) {
            Some(sa_bound) => (Some(vl_id), sa_bound),
            None => return Ok(RuntimeApplyOutcome::VlUnbound),
        },
        (None, Some(sa_only)) => (None, sa_only),
        (None, None) => unreachable!("missing_target checked above"),
    };
    store.clear_active_scene_unless_from_scene(aid, entry.last_dapc_source);
    Ok(apply_runtime_physical_and_publish(
        publisher,
        counters,
        corr,
        aid,
        vl,
        &PhysicalRuntimeCommit {
            short_address: sa,
            setpoint,
            observation: obs,
            observed_at_mono_ms: entry.observed_at_mono_ms,
            entry_last_dapc_source: entry.last_dapc_source,
            entry_source: entry.source,
        },
        store,
    ))
}

fn check_runtime_binding_match(
    publisher: &BusPublisher,
    store: &RegistryStore,
    corr: u64,
    aid: u8,
    vl_id: u8,
    sa_cmd: u8,
) -> Result<(), ()> {
    if store.internal_virtual_lamp_binding_short(aid, vl_id) == Some(sa_cmd) {
        return Ok(());
    }
    publish_correlation_failed(
        publisher,
        corr,
        ErrorCode::Conflict,
        "registry_runtime_binding_mismatch",
    );
    Err(())
}

fn store_outcome_to_runtime_outcome(outcome: RuntimeCommitOutcome) -> RuntimeApplyOutcome {
    match outcome {
        RuntimeCommitOutcome::Committed => RuntimeApplyOutcome::Applied,
        RuntimeCommitOutcome::Unchanged => RuntimeApplyOutcome::Unchanged,
        RuntimeCommitOutcome::Superseded => RuntimeApplyOutcome::Superseded,
        RuntimeCommitOutcome::NoRecord => RuntimeApplyOutcome::NotApplied,
    }
}

fn apply_runtime_physical_and_publish(
    publisher: &BusPublisher,
    counters: &RegistryCommandCounters,
    corr: u64,
    adapter_id: u8,
    vl: Option<u8>,
    commit: &PhysicalRuntimeCommit<'_>,
    store: &RegistryStore,
) -> RuntimeApplyOutcome {
    let short_address = commit.short_address;
    let outcome = store_outcome_to_runtime_outcome(
        store.apply_physical_runtime_short(adapter_id, commit),
    );
    if outcome != RuntimeApplyOutcome::Applied {
        return outcome;
    }
    counters
        .physical_device_runtime_applied
        .fetch_add(1, Ordering::Relaxed);
    if vl.is_some() {
        counters
            .virtual_lamp_runtime_applied
            .fetch_add(1, Ordering::Relaxed);
    }
    store_outcome_to_runtime_outcome(if publish_runtime_snapshot(
        publisher,
        counters,
        corr,
        adapter_id,
        vl,
        short_address,
        store,
        commit.entry_source,
        commit.setpoint.dimensions(),
    ) {
        RuntimeCommitOutcome::Committed
    } else {
        RuntimeCommitOutcome::NoRecord
    })
}

#[allow(clippy::too_many_arguments, reason = "mirrors the event it publishes")]
fn publish_runtime_snapshot(
    publisher: &BusPublisher,
    counters: &RegistryCommandCounters,
    corr: u64,
    adapter_id: u8,
    vl: Option<u8>,
    short_address: u8,
    store: &RegistryStore,
    commit_source: dali2rust_contracts::msg::RuntimeSource,
    commit_dimensions: dali2rust_contracts::msg::SetpointDimensions,
) -> bool {
    let Some((snap_sp, snap_obs)) =
        store.runtime_state_payload_from_physical(adapter_id, short_address)
    else {
        log::warn!(
            "registry: physical record missing after runtime apply \
             (adapter={adapter_id} short={short_address}); RuntimeStateChangedEvent skipped"
        );
        return true;
    };
    let vl = vl.or_else(|| store.internal_virtual_lamp_bound_to_short(adapter_id, short_address));
    publish_runtime_state_changed(
        publisher,
        &counters.runtime_events_published,
        corr,
        adapter_id,
        vl,
        Some(short_address),
        &snap_sp,
        &snap_obs,
        commit_source,
        commit_dimensions,
    );
    true
}

fn resolve_transition_target(
    publisher: &BusPublisher,
    store: &RegistryStore,
    corr: u64,
    cmd: &dali2rust_contracts::msg::RegistryLevelTransitionCommand,
) -> Result<(Option<u8>, u8), ()> {
    match (cmd.virtual_lamp_id, cmd.short_address) {
        (Some(vl_id), _) => match store.internal_virtual_lamp_binding_short(cmd.adapter_id, vl_id) {
            Some(bound) => Ok((Some(vl_id), bound)),
            None => {
                publish_correlation_failed(
                    publisher,
                    corr,
                    ErrorCode::VlUnbound,
                    "registry_transition_vl_unbound",
                );
                Err(())
            }
        },
        (None, Some(short)) => Ok((None, short)),
        (None, None) => {
            fail_runtime_invalid(publisher, corr, "registry_transition_missing_target");
            Err(())
        }
    }
}

pub(super) fn handle_level_transition(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    cmd: &dali2rust_contracts::msg::RegistryLevelTransitionCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, cmd.adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    let Some(obs) = cmd.observation.as_ref() else {
        return fail_runtime_invalid(publisher, corr, "registry_transition_missing_observation");
    };
    let Ok((vl, short_address)) = resolve_transition_target(publisher, store, corr, cmd) else {
        return;
    };
    let outcome = store_outcome_to_runtime_outcome(store.apply_level_transition_short(
        cmd.adapter_id,
        &crate::runtime::registry::physical_devices::LevelTransitionCommit {
            short_address,
            transition: cmd.transition,
            observation: obs,
            source: cmd.source,
            observed_at_mono_ms: cmd.observed_at_mono_ms,
        },
    ));
    let outcome = announce_transition(publisher, counters, corr, cmd, vl, short_address, store, outcome);
    finish_runtime_update_outcome(publisher, corr, counters, outcome);
}

#[allow(clippy::too_many_arguments, reason = "mirrors the snapshot publisher it wraps")]
fn announce_transition(
    publisher: &BusPublisher,
    counters: &RegistryCommandCounters,
    corr: u64,
    cmd: &dali2rust_contracts::msg::RegistryLevelTransitionCommand,
    vl: Option<u8>,
    short_address: u8,
    store: &RegistryStore,
    outcome: RuntimeApplyOutcome,
) -> RuntimeApplyOutcome {
    if outcome != RuntimeApplyOutcome::Applied {
        return outcome;
    }
    counters
        .physical_device_runtime_applied
        .fetch_add(1, Ordering::Relaxed);
    store_outcome_to_runtime_outcome(if publish_runtime_snapshot(
        publisher,
        counters,
        corr,
        cmd.adapter_id,
        vl,
        short_address,
        store,
        cmd.source,
        dali2rust_contracts::msg::SetpointDimensions { level: true, color: false },
    ) {
        RuntimeCommitOutcome::Committed
    } else {
        RuntimeCommitOutcome::NoRecord
    })
}
