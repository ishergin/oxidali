use std::sync::atomic::{AtomicU32, Ordering};

use crate::runtime::registry::publish::{
    publish_group_matrix_changed, publish_physical_device_changed, publish_scene_matrix_changed,
};
use crate::runtime::registry::{ForeignSceneWrite, RegistryStore};
use crate::runtime::registry_worker::RegistryEventsCounters;
use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::BusEventPayload;

fn apply_and_publish_pd(
    publisher: &BusPublisher,
    corr: u64,
    registry_adapter_id: u8,
    short_address: u8,
    counter: &AtomicU32,
    apply: impl FnOnce() -> bool,
) {
    if !apply() {
        return;
    }
    publish_physical_device_changed(publisher, corr, registry_adapter_id, short_address);
    counter.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn apply_event_frame(
    frame: BusFrame,
    publisher: &BusPublisher,
    bus_id: BusId,
    store: &RegistryStore,
    counters: &RegistryEventsCounters,
) {
    let Some(ev) = frame.event_for(bus_id) else {
        return;
    };
    let corr = ev.meta.correlation_id;

    dispatch_registry_event(&ev.payload, publisher, corr, store, counters);
}

dali2rust_contracts::dispatch_bus_events! {
    pub const REGISTRY_EVENTS_HANDLED_EVENTS;
    fn dispatch_registry_event(
        payload: &BusEventPayload,
        publisher: &BusPublisher,
        corr: u64,
        store: &RegistryStore,
        counters: &RegistryEventsCounters,
    );
    payload = payload;
    ignored = { counters.ignored_events.fetch_add(1, Ordering::Relaxed); };
    DaliSceneRecalledEvent(body) => {
        if body.error.is_none() {
            store.note_scene_recalled(body.registry_adapter_id, body.scene_id, body.scope);
            counters.scene_recalls_noted.fetch_add(1, Ordering::Relaxed);
        }
    },
    DaliTargetStateAppliedEvent(body) => {
        note_mass_commanded(
            store,
            body.registry_adapter_id,
            body.scope,
            body.group_id,
            Some(&body.setpoint),
        );
    },
    DaliObservedFrameEvent(body) => {
        match body.observed_kind {
            dali2rust_contracts::msg::ObservedKind::TargetStateObserved => note_mass_commanded(
                store,
                body.registry_adapter_id,
                body.scope,
                body.group_id,
                body.setpoint.as_ref(),
            ),
            dali2rust_contracts::msg::ObservedKind::SceneWriteObserved
            | dali2rust_contracts::msg::ObservedKind::SceneRemovalObserved => {
                commit_foreign_scene_write(publisher, corr, store, body);
            }
            _ => {}
        }
    },
    DaliAttributesWrittenEvent(body) => {
        let aid = body.registry_adapter_id;
        let sa = body.short_address;
        apply_and_publish_pd(
            publisher,
            corr,
            aid,
            sa,
            &counters.dali_attributes_committed,
            || {
                !body.confirms_nothing()
                    && store.apply_attributes_written(
                        aid,
                        sa,
                        body.fade_time_ms,
                        body.fade_rate,
                        body.power_on_level,
                        body.system_failure_level,
                        body.extended_fade_time_ms,
                        (body.tc_coolest_mirek, body.tc_warmest_mirek),
                        (body.min_level, body.max_level),
                        body.dimming_curve,
                    )
            },
        );
    },
    DaliAddressingCompletedEvent(body) => {
        commit_address_change(publisher, corr, store, counters, body);
    },
    DaliDeviceReplacedEvent(body) => {
        commit_device_replacement(publisher, corr, store, counters, body);
    },
    Dali103ApplicationControlObservedEvent(body) => {
        if let Some(applied) = store.apply_application_control(body.scope, body.enable) {
            counters.app_control_applied.fetch_add(1, Ordering::Relaxed);
            crate::runtime::registry::publish::publish_dali_settings_changed(
                publisher,
                corr,
                0,
                &applied,
                dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_WIRE,
            );
        }
    },
    Dali103InstanceConfiguredEvent(body) => {
        let now_ms = dali2rust_bsp::unix_clock::unix_wall_clock_millis();
        let readback = crate::runtime::registry::input_devices::InstanceReadback {
            instance_type: body.instance_type,
            instance_status: body.instance_status,
            instance_status_written: body.instance_status_written,
            resolution: body.resolution,
            event_scheme: body.event_scheme,
            event_filter: body.event_filter,
            event_priority: body.event_priority,
            instance_groups: body.instance_groups,
            timers: body.timers,
            manual_config_active: body.manual_config_active,
            feedback_opcode_map: body.feedback_opcode_map,
            feedback_capability: body.feedback_capability,
            feedback_colour_capability: body.feedback_colour_capability,
            feedback_timing: body.feedback_timing,
            feedback_active_brightness: body.feedback_active_brightness,
            feedback_active_colour: body.feedback_active_colour,
            feedback_inactive_brightness: body.feedback_inactive_brightness,
            feedback_inactive_colour: body.feedback_inactive_colour,
        };
        if store.apply_instance_readback(
            body.registry_adapter_id,
            body.short_address,
            body.instance_number,
            &readback,
            now_ms,
        ) {
            counters
                .input_instance_readbacks_applied
                .fetch_add(1, Ordering::Relaxed);
        }
    },
    Dali103ScanStartedEvent(body) => {
        let cleared = store.clear_input_presence(body.registry_adapter_id);
        if cleared > 0 {
            counters
                .input_presence_cleared
                .fetch_add(cleared, Ordering::Relaxed);
        }
    },
    Dali103ScanProgressEvent(body) => {
        let now_ms = dali2rust_bsp::unix_clock::unix_wall_clock_millis();
        if store.apply_input_scan_progress(body, now_ms) {
            counters
                .input_scan_progress_applied
                .fetch_add(1, Ordering::Relaxed);
        } else {
            counters
                .input_scan_progress_rejected
                .fetch_add(1, Ordering::Relaxed);
        }
    },
    DaliInputEventObservedEvent(body) => {
        if store.apply_input_event(body) {
            counters.input_events_applied.fetch_add(1, Ordering::Relaxed);
        } else {
            counters
                .input_events_unattributed
                .fetch_add(1, Ordering::Relaxed);
        }
    },
    DaliInputDeviceLifecycleEvent(body) => {
        let now_ms = dali2rust_bsp::unix_clock::unix_wall_clock_millis();
        if let Some(short_address) = body.short_address {
            store.note_input_device_power_cycle(body.registry_adapter_id, short_address, now_ms);
        }
        counters
            .input_power_cycles_seen
            .fetch_add(1, Ordering::Relaxed);
    },
    DaliDiscoveryProgressEvent(body) => {
        let applied = store.apply_discovery_progress(body);
        if applied {
            publish_physical_device_changed(
                publisher,
                corr,
                body.registry_adapter_id,
                body.short_address,
            );
            counters
                .discovery_progress_applied
                .fetch_add(1, Ordering::Relaxed);
        } else {
            counters
                .discovery_progress_rejected
                .fetch_add(1, Ordering::Relaxed);
        }
    },
    DaliDiscoveryScanReconciledEvent(body) => {
        let evicted =
            store.reconcile_discovery_scan(body.registry_adapter_id, body.confirmed_mask);
        for short_address in evicted {
            publish_physical_device_changed(
                publisher,
                corr,
                body.registry_adapter_id,
                short_address,
            );
        }
        counters
            .discovery_scan_reconciled_applied
            .fetch_add(1, Ordering::Relaxed);
    },
    DaliAttributesReadEvent(body) => {
        let aid = body.registry_adapter_id;
        let sa = body.short_address;
        apply_and_publish_pd(
            publisher,
            corr,
            aid,
            sa,
            &counters.attribute_read_evidence_applied,
            || store.apply_physical_device_attribute_chunk(aid, sa, &body.chunk),
        );
        if let dali2rust_contracts::msg::DaliAttributeReadChunk::Groups { membership } =
            body.chunk
        {
            let matrix_changed = match membership {
                Some(membership) => {
                    store.apply_group_membership_readback(aid, sa, membership)
                }
                None => store.seed_group_matrix_from_membership_read(aid, sa),
            };
            if matrix_changed {
                publish_group_matrix_changed(publisher, corr, aid);
            }
        }
        if let dali2rust_contracts::msg::DaliAttributeReadChunk::Scenes { .. } = body.chunk {
            let changed_mask = store.seed_scene_matrix_from_levels_read(aid, sa);
            for scene_id in 0..16u8 {
                if changed_mask & (1u16 << scene_id) != 0 {
                    publish_scene_matrix_changed(publisher, corr, aid, scene_id);
                }
            }
        }
    },
    DaliMemoryBankReadEvent(body) => {
        let data = body.data.as_slice();
        apply_and_publish_pd(
            publisher,
            corr,
            body.registry_adapter_id,
            body.short_address,
            &counters.memory_bank_read_committed,
            || {
                store.stage_memory_bank_chunk(
                    corr,
                    body.registry_adapter_id,
                    body.short_address,
                    body.bank,
                    body.start_offset,
                    body.chunk_index,
                    body.last_chunk,
                    data,
                )
            },
        );
    },
    DaliMemoryBankReadAbortedEvent(_) => store.abort_memory_bank_stage(corr),
    DaliGroupMembershipProgrammedEvent(body) => {
        if body.error.is_none() {
            if let (Some(short), Some(membership)) =
                (body.physical_short_address, body.membership)
            {
                if store.apply_group_membership_readback(
                    body.registry_adapter_id,
                    short,
                    membership,
                ) {
                    publish_group_matrix_changed(publisher, corr, body.registry_adapter_id);
                    counters
                        .group_membership_programmed_applied
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    },
    DaliSceneProgrammedEvent(body) => {
        if body.error.is_none() {
            if let Some(short) = body.physical_short_address {
                if store.apply_scene_programmed_readback(
                    body.registry_adapter_id,
                    short,
                    body.scene_id,
                    body.scene_level,
                    body.target_state.as_ref(),
                ) {
                    publish_scene_matrix_changed(
                        publisher,
                        corr,
                        body.registry_adapter_id,
                        body.scene_id,
                    );
                    counters
                        .scene_programmed_applied
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    },
}

fn commanded_power(sp: &dali2rust_contracts::msg::LightSetpoint) -> Option<bool> {
    sp.commanded_power()
}

fn note_mass_commanded(
    store: &RegistryStore,
    adapter_id: u8,
    scope: dali2rust_contracts::msg::DaliTargetScope,
    group_id: Option<u8>,
    setpoint: Option<&dali2rust_contracts::msg::LightSetpoint>,
) {
    let Some(on) = setpoint.and_then(commanded_power) else {
        return;
    };
    match scope {
        dali2rust_contracts::msg::DaliTargetScope::Group => {
            if let Some(group_id) = group_id {
                store.note_group_commanded(adapter_id, group_id, on);
            }
        }
        dali2rust_contracts::msg::DaliTargetScope::Broadcast => {
            store.note_broadcast_commanded(adapter_id, on);
        }
        _ => {}
    }
}

#[inline(never)]
fn commit_address_change(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryEventsCounters,
    body: &dali2rust_contracts::msg::DaliAddressingCompletedEvent,
) {
    if body.error.is_some() {
        return;
    }
    let aid = body.registry_adapter_id;
    if store.apply_address_change(aid, body.old_short_address, body.new_short_address) {
        counters
            .dali_address_changes_committed
            .fetch_add(1, Ordering::Relaxed);
        publish_physical_device_changed(publisher, corr, aid, body.new_short_address);
    }
}

fn commit_foreign_scene_write(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    body: &dali2rust_contracts::msg::DaliObservedFrameEvent,
) {
    let Some(scene_id) = body.scene_id else {
        return;
    };
    let aid = body.registry_adapter_id;
    let write = ForeignSceneWrite {
        scope: body.scope,
        short_address: body.short_address,
        group_id: body.group_id,
        scene_id,
        removal: body.observed_kind == dali2rust_contracts::msg::ObservedKind::SceneRemovalObserved,
    };
    let changed = store.apply_foreign_scene_write(aid, write);
    for short in &changed {
        publish_physical_device_changed(publisher, corr, aid, *short);
    }
    if !changed.is_empty() {
        publish_scene_matrix_changed(publisher, corr, aid, scene_id);
    }
}

#[inline(never)]
fn commit_device_replacement(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryEventsCounters,
    body: &dali2rust_contracts::msg::DaliDeviceReplacedEvent,
) {
    if body.error.is_some() {
        return;
    }
    let aid = body.registry_adapter_id;
    let restored = store.apply_device_replacement(
        aid,
        body.failed_short_address,
        body.replacement_short_address,
        body.restored_metadata_and_overrides,
        body.restored_attributes,
        body.restored_groups,
        body.restored_scenes,
    );
    if restored.is_some() {
        counters
            .dali_device_replacements_committed
            .fetch_add(1, Ordering::Relaxed);
        publish_physical_device_changed(publisher, corr, aid, body.failed_short_address);
    }
}
