use super::*;

pub(super) fn handle_discover_devices(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    dc: &dali2rust_contracts::msg::DaliDiscoverDevicesCommand,
    counters: &DaliWorkerCounters,
    correlation: &dali2rust_bus::CorrelationIdAllocator,
) {
    let w = correlation_id;
    publish_operation_worker_signal(
        publisher,
        adapter_id,
        w,
        OperationWorkerSignal::WorkerStarted,
        None,
        "",
        Origin::Internal,
        counters,
    );
    let reg_aid = dc.registry_adapter_id;
    let result = run_discovery_mode(
        controller,
        runtime_config,
        read_port,
        publisher,
        w,
        adapter_id,
        reg_aid,
        dc.mode,
        counters,
    );
    publish_discovery_outcome(
        publisher, adapter_id, w, reg_aid, result, counters, read_port, correlation,
    );
    counters
        .semantic_discover_devices_handled
        .fetch_add(1, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments, reason = "mirrors the dispatch-arm signature")]
fn run_discovery_mode(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    w: u64,
    adapter_id: BusId,
    reg_aid: u8,
    mode: DiscoveryMode,
    counters: &DaliWorkerCounters,
) -> Result<(), SemanticDaliError> {
    let mut used_shorts = read_port.known_physical_short_addresses(reg_aid);
    match mode {
        DiscoveryMode::RefreshKnown => {
            discover_refresh_known(controller, &used_shorts, publisher, w, adapter_id, reg_aid, counters)
        }
        DiscoveryMode::ScanKnownShortAddresses => {
            discover_scan_all(controller, runtime_config, publisher, w, adapter_id, reg_aid, counters)
        }
        DiscoveryMode::CommissionUnaddressed => commission_unaddressed_loop(
            controller,
            runtime_config,
            &mut used_shorts,
            publisher,
            w,
            adapter_id,
            reg_aid,
            counters,
        ),
    }
}

#[allow(clippy::too_many_arguments, reason = "threaded, not rebuilt per call")]
fn publish_discovery_outcome(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    reg_aid: u8,
    result: Result<(), SemanticDaliError>,
    counters: &DaliWorkerCounters,
    read_port: &dyn RegistryReadPort,
    correlation: &dali2rust_bus::CorrelationIdAllocator,
) {
    match result {
        Err(error) => publish_discovery_failed(publisher, adapter_id, w, &error, counters),
        Ok(()) => publish_discovery_succeeded(
            publisher,
            adapter_id,
            w,
            reg_aid,
            counters,
            read_port,
            correlation,
        ),
    }
}

fn publish_discovery_failed(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    error: &SemanticDaliError,
    counters: &DaliWorkerCounters,
) {
    let fail_ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, w, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryFailedEvent {});
    publish_event_typed(publisher, fail_ev);
    counters
        .discovery_failed_events_published
        .fetch_add(1, Ordering::Relaxed);
    publish_operation_worker_signal(
        publisher,
        adapter_id,
        w,
        OperationWorkerSignal::WorkerFailed,
        Some(error.code()),
        error.message(),
        Origin::Internal,
        counters,
    );
}

#[allow(clippy::too_many_arguments, reason = "threaded, not rebuilt per call")]
fn publish_discovery_succeeded(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    reg_aid: u8,
    counters: &DaliWorkerCounters,
    read_port: &dyn RegistryReadPort,
    correlation: &dali2rust_bus::CorrelationIdAllocator,
) {
    let done =
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, w, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryCompletedEvent { registry_adapter_id: reg_aid });
    publish_event_typed(publisher, done);
    publish_policy_apply_if_armed(publisher, adapter_id, reg_aid, read_port, correlation);
    publish_operation_worker_signal(
        publisher,
        adapter_id,
        w,
        OperationWorkerSignal::WorkerSucceeded,
        None,
        "",
        Origin::Internal,
        counters,
    );
}

fn publish_policy_apply_if_armed(
    publisher: &BusPublisher,
    adapter_id: BusId,
    reg_aid: u8,
    read_port: &dyn RegistryReadPort,
    correlation: &dali2rust_bus::CorrelationIdAllocator,
) {
    if !read_port.apply_on_discovery_armed(reg_aid) {
        return;
    }
    let workflow = correlation.next_id();
    let operation_id = format!("policy-apply-{reg_aid}-{workflow}");
    let execute = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::PolicyApplyExecuteCommand {
            registry_adapter_id: reg_aid,
            operation_key: dali2rust_contracts::msg::fixed_text_32(&operation_id),
        },
    );
    if publisher.try_publish(
        dali2rust_bus::BusChannel::Commands,
        dali2rust_bus::BusFrame::command(execute),
    ) != dali2rust_bus::PublishResult::Queued
    {
        log::warn!(
            "policy apply on discovery refused at ingress (adapter {reg_aid},              workflow {workflow}); the policy is unwritten until POST /policies/apply"
        );
    }
}

fn discover_refresh_known(
    controller: &mut impl DaliApplicationController,
    shorts: &[u8],
    publisher: &BusPublisher,
    w: u64,
    adapter_id: BusId,
    reg_aid: u8,
    counters: &DaliWorkerCounters,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    for &short_address in shorts {
        if let Some(found) = detect_device(controller, short_address)? {
            publish_discovery_progress(publisher, w, adapter_id, reg_aid, &found, counters);
        }
    }
    Ok(())
}

fn discover_scan_all(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    publisher: &BusPublisher,
    w: u64,
    adapter_id: BusId,
    reg_aid: u8,
    counters: &DaliWorkerCounters,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    let attempts = runtime_config.discovery_step_retries.saturating_add(1);
    for attempt in 0..attempts {
        let mut confirmed_mask: u64 = 0;
        let summary = discover_known_control_gear_with_retry_budget(
            controller,
            runtime_config.discovery_step_retries,
            &mut |found: &DiscoveredDevice| {
                confirmed_mask |= 1u64 << (found.short_address & 0x3F);
                if found.type_enum_degraded {
                    counters
                        .discovery_device_type_degraded
                        .fetch_add(1, Ordering::Relaxed);
                }
                publish_discovery_progress(publisher, w, adapter_id, reg_aid, found, counters);
            },
        )?;
        let had_progress = summary.described > 0;
        let error = summary.error;
        match error {
            None => {
                publish_scan_reconciled(publisher, w, adapter_id, reg_aid, confirmed_mask, counters);
                return Ok(());
            }
            Some(error) if error.is_bus_contended() && !had_progress && attempt + 1 < attempts => {
                continue;
            }
            Some(error) => return Err(error),
        }
    }
    Err(crate::runtime::executor::SemanticDaliError::OperationFailed(
        "bus_contended",
    ))
}

fn publish_scan_reconciled(
    publisher: &BusPublisher,
    w: u64,
    adapter_id: BusId,
    reg_aid: u8,
    confirmed_mask: u64,
    counters: &DaliWorkerCounters,
) {
    let reconciled = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, w, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryScanReconciledEvent { registry_adapter_id: reg_aid, confirmed_mask });
    publish_event_required(publisher, reconciled, counters, PublishKind::Series, "discovery-reconciled");
}

fn commission_unaddressed_loop(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    used_shorts: &mut Vec<u8>,
    publisher: &BusPublisher,
    w: u64,
    adapter_id: BusId,
    reg_aid: u8,
    counters: &DaliWorkerCounters,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    let attempts = runtime_config.discovery_step_retries.saturating_add(1);
    for attempt in 0..attempts {
        match commission_unaddressed_once(
            controller,
            runtime_config,
            used_shorts,
            publisher,
            w,
            adapter_id,
            reg_aid,
            counters,
        ) {
            Ok(()) => return Ok(()),
            Err(error) if error.is_bus_contended() && attempt + 1 < attempts => continue,
            Err(error) => return Err(error),
        }
    }
    Err(crate::runtime::executor::SemanticDaliError::OperationFailed(
        "bus_contended",
    ))
}

fn commission_unaddressed_once(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    used_shorts: &mut Vec<u8>,
    publisher: &BusPublisher,
    w: u64,
    adapter_id: BusId,
    reg_aid: u8,
    counters: &DaliWorkerCounters,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    crate::runtime::executor::begin_unaddressed_commissioning(controller)?;
    let result = commission_unaddressed_drain(
        controller,
        runtime_config,
        used_shorts,
        publisher,
        w,
        adapter_id,
        reg_aid,
        counters,
    );
    let cleanup = crate::runtime::executor::terminate_unaddressed_commissioning(controller);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[allow(clippy::too_many_arguments, reason = "mirrors the dispatch-arm signature")]
fn commission_unaddressed_drain(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    used_shorts: &mut Vec<u8>,
    publisher: &BusPublisher,
    w: u64,
    adapter_id: BusId,
    reg_aid: u8,
    counters: &DaliWorkerCounters,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    loop {
        match commission_next_unaddressed_with_retry_budget(
            controller,
            used_shorts,
            runtime_config.discovery_step_retries,
        ) {
            Ok(Some(found)) => {
                used_shorts.push(found.short_address);
                publish_discovery_progress(publisher, w, adapter_id, reg_aid, &found, counters);
            }
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        }
    }
}

fn publish_discovery_progress(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: BusId,
    registry_adapter_id: u8,
    found: &DiscoveredDevice,
    counters: &DaliWorkerCounters,
) {
    let prog = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliDiscoveryProgressEvent {
            registry_adapter_id,
            short_address: found.short_address,
            random_address: found.random_address,
            device_type: found.device_type,
            color_mode: found.color_mode,
            dt8_xy_capable: found.dt8_xy_capable,
            dt8_tc_capable: found.dt8_tc_capable,
            dt8_rgb_capable: found.dt8_rgb_capable,
            dt8_rgbwaf_capable: found.dt8_rgbwaf_capable,
            supported_device_types: found.supported_device_types,
        },
    );
    if publish_event_required(publisher, prog, counters, PublishKind::Series, "discovery-progress") {
        counters
            .discovery_progress_events_published
            .fetch_add(1, Ordering::Relaxed);
    }
}
