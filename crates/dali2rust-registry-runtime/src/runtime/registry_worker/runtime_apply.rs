use std::sync::atomic::Ordering;

use dali2rust_contracts::msg::{
    ErrorCode, LastDapcSource, LightSetpoint, RegistryRuntimeUpdateCommand, RuntimeObservation,
    RuntimeRegistryUpdateEntry,
};

use crate::runtime::registry::physical_devices::{PhysicalRuntimeCommit, RuntimeCommitOutcome};
use crate::runtime::registry::publish::{publish_runtime_state_changed, RuntimeCommitMeta};

use super::command_ctx::CommandCtx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeApplyOutcome {
    Applied,
    NotApplied,
    VlUnbound,
    Superseded,
    Unchanged,
}

pub(super) fn handle_runtime_update(ctx: &CommandCtx<'_>, reg: &RegistryRuntimeUpdateCommand) {
    if ctx.check_adapter_target(reg.adapter_id).is_err() {
        return;
    }
    let entry = &reg.update;
    let Some(setpoint) = entry.setpoint.as_ref() else {
        return fail_runtime_invalid(ctx, "runtime_missing_setpoint");
    };
    let Some(obs) = entry.observation.as_ref() else {
        return fail_runtime_invalid(ctx, "runtime_missing_observation");
    };
    if entry.virtual_lamp_id.is_none() && entry.short_address.is_none() {
        return fail_runtime_invalid(ctx, "runtime_missing_target");
    }

    let applied = match apply_registry_runtime_targets(ctx, reg.adapter_id, entry, setpoint, obs) {
        Ok(outcome) => outcome,
        Err(()) => return,
    };
    finish_runtime_update_outcome(ctx, applied);
}

fn fail_runtime_invalid(ctx: &CommandCtx<'_>, reason: &'static str) {
    ctx.confirm_failed(ErrorCode::InvalidValue, reason);
}

fn finish_runtime_update_outcome(ctx: &CommandCtx<'_>, outcome: RuntimeApplyOutcome) {
    match outcome {
        RuntimeApplyOutcome::Applied => {
            ctx.counters
                .runtime_updates_applied
                .fetch_add(1, Ordering::Relaxed);
            ctx.confirm_ok();
        }
        RuntimeApplyOutcome::NotApplied => {
            ctx.confirm_failed(ErrorCode::OperationFailed, "runtime_not_applied");
        }
        RuntimeApplyOutcome::VlUnbound => {
            ctx.confirm_failed(ErrorCode::VlUnbound, "vl_unbound");
        }
        RuntimeApplyOutcome::Superseded => {
            ctx.counters
                .runtime_updates_superseded
                .fetch_add(1, Ordering::Relaxed);
            ctx.confirm_ok();
        }
        RuntimeApplyOutcome::Unchanged => {
            ctx.counters
                .runtime_updates_applied
                .fetch_add(1, Ordering::Relaxed);
            ctx.confirm_ok();
        }
    }
}

fn apply_registry_runtime_targets(
    ctx: &CommandCtx<'_>,
    aid: u8,
    entry: &RuntimeRegistryUpdateEntry,
    setpoint: &LightSetpoint,
    obs: &RuntimeObservation,
) -> Result<RuntimeApplyOutcome, ()> {
    let (vl, sa) = match (entry.virtual_lamp_id, entry.short_address) {
        (Some(vl_id), Some(sa_cmd)) => {
            check_runtime_binding_match(ctx, aid, vl_id, sa_cmd)?;
            (Some(vl_id), sa_cmd)
        }
        (Some(vl_id), None) => match ctx.store.internal_virtual_lamp_binding_short(aid, vl_id) {
            Some(sa_bound) => (Some(vl_id), sa_bound),
            None => return Ok(RuntimeApplyOutcome::VlUnbound),
        },
        (None, Some(sa_only)) => (None, sa_only),
        (None, None) => unreachable!("missing_target checked above"),
    };
    ctx.store.clear_active_scene_unless_from_scene(aid, entry.last_dapc_source);
    Ok(apply_runtime_physical_and_publish(
        ctx,
        aid,
        vl,
        &PhysicalRuntimeCommit {
            short_address: sa,
            setpoint,
            observation: obs,
            observed_at_mono_ms: entry.observed_at_mono_ms,
            entry_last_dapc_source: entry.last_dapc_source,
            entry_source: entry.source,
            entry_holds_hcl: entry.hold_hcl,
            clears_power_cycle_seen: issues_a_level_command(entry, setpoint),
        },
    ))
}

// IEC 62386-102 §9.16.9
fn issues_a_level_command(entry: &RuntimeRegistryUpdateEntry, setpoint: &LightSetpoint) -> bool {
    !entry.source.reads_the_gear()
        && (setpoint.commanded_power().is_some()
            || entry.last_dapc_source == Some(LastDapcSource::Scene))
}

fn check_runtime_binding_match(
    ctx: &CommandCtx<'_>,
    aid: u8,
    vl_id: u8,
    sa_cmd: u8,
) -> Result<(), ()> {
    if ctx.store.internal_virtual_lamp_binding_short(aid, vl_id) == Some(sa_cmd) {
        return Ok(());
    }
    ctx.confirm_failed(ErrorCode::Conflict, "runtime_binding_mismatch");
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
    ctx: &CommandCtx<'_>,
    adapter_id: u8,
    vl: Option<u8>,
    commit: &PhysicalRuntimeCommit<'_>,
) -> RuntimeApplyOutcome {
    let short_address = commit.short_address;
    let outcome = store_outcome_to_runtime_outcome(
        ctx.store.apply_physical_runtime_short(adapter_id, commit),
    );
    if outcome != RuntimeApplyOutcome::Applied {
        return outcome;
    }
    ctx.counters
        .physical_device_runtime_applied
        .fetch_add(1, Ordering::Relaxed);
    if vl.is_some() {
        ctx.counters
            .virtual_lamp_runtime_applied
            .fetch_add(1, Ordering::Relaxed);
    }
    publish_runtime_snapshot(
        ctx,
        adapter_id,
        vl,
        short_address,
        RuntimeCommitMeta {
            source: commit.entry_source,
            dimensions: commit.setpoint.dimensions(),
            holds_hcl: commit.entry_holds_hcl,
        },
    );
    RuntimeApplyOutcome::Applied
}

fn publish_runtime_snapshot(
    ctx: &CommandCtx<'_>,
    adapter_id: u8,
    vl: Option<u8>,
    short_address: u8,
    commit: RuntimeCommitMeta,
) {
    let Some((snap_sp, snap_obs)) =
        ctx.store.runtime_state_payload_from_physical(adapter_id, short_address)
    else {
        log::warn!(
            "registry: physical record missing after runtime apply \
             (adapter={adapter_id} short={short_address}); RuntimeStateChangedEvent skipped"
        );
        return;
    };
    let vl = vl.or_else(|| ctx.store.internal_virtual_lamp_bound_to_short(adapter_id, short_address));
    publish_runtime_state_changed(
        ctx.publisher,
        &ctx.counters.runtime_events_published,
        ctx.corr,
        adapter_id,
        vl,
        Some(short_address),
        &snap_sp,
        &snap_obs,
        commit,
    );
}

fn resolve_transition_target(
    ctx: &CommandCtx<'_>,
    cmd: &dali2rust_contracts::msg::RegistryLevelTransitionCommand,
) -> Result<(Option<u8>, u8), ()> {
    match (cmd.virtual_lamp_id, cmd.short_address) {
        (Some(vl_id), _) => match ctx.store.internal_virtual_lamp_binding_short(cmd.adapter_id, vl_id) {
            Some(bound) => Ok((Some(vl_id), bound)),
            None => {
                ctx.confirm_failed(ErrorCode::VlUnbound, "transition_vl_unbound");
                Err(())
            }
        },
        (None, Some(short)) => Ok((None, short)),
        (None, None) => {
            fail_runtime_invalid(ctx, "transition_missing_target");
            Err(())
        }
    }
}

pub(super) fn handle_level_transition(
    ctx: &CommandCtx<'_>,
    cmd: &dali2rust_contracts::msg::RegistryLevelTransitionCommand,
) {
    if ctx.check_adapter_target(cmd.adapter_id).is_err() {
        return;
    }
    let Some(obs) = cmd.observation.as_ref() else {
        return fail_runtime_invalid(ctx, "transition_missing_observation");
    };
    let Ok((vl, short_address)) = resolve_transition_target(ctx, cmd) else {
        return;
    };
    let outcome = store_outcome_to_runtime_outcome(ctx.store.apply_level_transition_short(
        cmd.adapter_id,
        &crate::runtime::registry::physical_devices::LevelTransitionCommit {
            short_address,
            transition: cmd.transition,
            observation: obs,
            source: cmd.source,
            observed_at_mono_ms: cmd.observed_at_mono_ms,
            holds_hcl: cmd.hold_hcl,
        },
    ));
    let outcome = announce_transition(ctx, cmd, vl, short_address, outcome);
    finish_runtime_update_outcome(ctx, outcome);
}

fn announce_transition(
    ctx: &CommandCtx<'_>,
    cmd: &dali2rust_contracts::msg::RegistryLevelTransitionCommand,
    vl: Option<u8>,
    short_address: u8,
    outcome: RuntimeApplyOutcome,
) -> RuntimeApplyOutcome {
    if outcome != RuntimeApplyOutcome::Applied {
        return outcome;
    }
    ctx.counters
        .physical_device_runtime_applied
        .fetch_add(1, Ordering::Relaxed);
    publish_runtime_snapshot(
        ctx,
        cmd.adapter_id,
        vl,
        short_address,
        RuntimeCommitMeta {
            source: cmd.source,
            dimensions: dali2rust_contracts::msg::SetpointDimensions { level: true, color: false },
            holds_hcl: cmd.hold_hcl,
        },
    );
    RuntimeApplyOutcome::Applied
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::{ColorMode, ColorValue, PowerState, RuntimeSource};

    use super::*;

    const DRIVEN_LEVEL: u8 = 100;
    const WARM_WHITE_KELVIN: u16 = 3000;

    fn entry(source: RuntimeSource, last_dapc_source: Option<LastDapcSource>) -> RuntimeRegistryUpdateEntry {
        RuntimeRegistryUpdateEntry {
            source,
            last_dapc_source,
            ..RuntimeRegistryUpdateEntry::default()
        }
    }

    fn colour_only() -> LightSetpoint {
        LightSetpoint {
            power: PowerState::Unknown,
            level: None,
            color: Some(ColorValue {
                mode: ColorMode::Cct,
                color_temperature_kelvin: WARM_WHITE_KELVIN,
                ..ColorValue::default()
            }),
        }
    }

    #[test]
    fn a_drive_that_commands_power_clears_the_bit_and_a_read_never_does() {
        let level = LightSetpoint::from_level(DRIVEN_LEVEL, None);
        assert!(issues_a_level_command(&entry(RuntimeSource::Api, None), &level));
        assert!(issues_a_level_command(&entry(RuntimeSource::Sniffer, None), &level));
        assert!(!issues_a_level_command(&entry(RuntimeSource::Readback, None), &level));
        assert!(!issues_a_level_command(&entry(RuntimeSource::Poller, None), &level));
    }

    #[test]
    fn a_scene_recall_clears_the_bit_even_when_its_entry_states_no_level() {
        let scene = Some(LastDapcSource::Scene);
        assert!(issues_a_level_command(&entry(RuntimeSource::Api, scene), &colour_only()));
        assert!(
            !issues_a_level_command(&entry(RuntimeSource::Hcl, None), &colour_only()),
            "a colour change goes out as ACTIVATE, which leaves the bit alone"
        );
    }
}
