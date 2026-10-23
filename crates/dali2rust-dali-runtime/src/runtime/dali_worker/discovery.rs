use super::*;

#[derive(Clone, Copy)]
struct Scan<'a> {
    env: &'a WorkerEnv<'a>,
    w: u64,
    reg_aid: u8,
}

pub(super) fn handle_discover_devices(
    controller: &mut impl DaliApplicationController,
    env: &WorkerEnv<'_>,
    correlation_id: u64,
    dc: &dali2rust_contracts::msg::DaliDiscoverDevicesCommand,
) {
    let scan = Scan { env, w: correlation_id, reg_aid: dc.registry_adapter_id };
    publish_operation_worker_signal(
        scan.env.publisher,
        scan.env.adapter_id,
        scan.w,
        OperationWorkerSignal::WorkerStarted,
        None,
        Origin::Internal,
        scan.env.counters,
    );
    let result = run_discovery_mode(controller, &scan, dc.mode);
    publish_discovery_outcome(&scan, result);
    scan.env.counters
        .semantic_discover_devices_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn run_discovery_mode(
    controller: &mut impl DaliApplicationController,
    scan: &Scan<'_>,
    mode: DiscoveryMode,
) -> Result<(), SemanticDaliError> {
    let mut used_shorts = scan.env.read_port.known_physical_short_addresses(scan.reg_aid);
    match mode {
        DiscoveryMode::RefreshKnown => discover_refresh_known(controller, &used_shorts, scan),
        DiscoveryMode::ScanKnownShortAddresses => discover_scan_all(controller, scan),
        DiscoveryMode::CommissionUnaddressed => {
            commission_unaddressed_loop(controller, &mut used_shorts, scan)
        }
    }
}

fn publish_discovery_outcome(scan: &Scan<'_>, result: Result<(), SemanticDaliError>) {
    match result {
        Err(error) => publish_discovery_failed(scan, &error),
        Ok(()) => publish_discovery_succeeded(scan),
    }
}

fn publish_discovery_failed(scan: &Scan<'_>, error: &SemanticDaliError) {
    let fail_ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, scan.w, scan.env.adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryFailedEvent {});
    publish_event_typed(scan.env.publisher, fail_ev);
    scan.env.counters
        .discovery_failed_events_published
        .fetch_add(1, Ordering::Relaxed);
    publish_operation_worker_signal(
        scan.env.publisher,
        scan.env.adapter_id,
        scan.w,
        OperationWorkerSignal::WorkerFailed,
        Some((error.code(), error.message())),
        Origin::Internal,
        scan.env.counters,
    );
}

fn publish_discovery_succeeded(scan: &Scan<'_>) {
    let done =
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, scan.w, scan.env.adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryCompletedEvent { registry_adapter_id: scan.reg_aid });
    publish_event_typed(scan.env.publisher, done);
    publish_policy_apply_if_armed(scan);
    publish_operation_worker_signal(
        scan.env.publisher,
        scan.env.adapter_id,
        scan.w,
        OperationWorkerSignal::WorkerSucceeded,
        None,
        Origin::Internal,
        scan.env.counters,
    );
}

fn publish_policy_apply_if_armed(scan: &Scan<'_>) {
    let Scan { env, reg_aid, .. } = *scan;
    let (publisher, adapter_id, counters) = (env.publisher, env.adapter_id, env.counters);
    let (read_port, correlation) = (env.read_port, env.correlation);
    if !read_port.apply_on_discovery_armed(reg_aid) {
        return;
    }
    let workflow = correlation.next_id();
    let operation_id = format!("policy-apply-{reg_aid}-{workflow}");
    let begin = dali2rust_contracts::msg::OperationBeginCommand::with_defaults(
        &operation_id,
        dali2rust_contracts::msg::OperationType::PolicyApply,
    );
    if !publish_internal_command(publisher, adapter_id, workflow, begin) {
        log::warn!(
            "policy apply on discovery refused at ingress (adapter {reg_aid}, workflow {workflow}); \
             the policy is unwritten until POST /policies/apply"
        );
        return;
    }
    let execute = dali2rust_contracts::msg::PolicyApplyExecuteCommand {
        registry_adapter_id: reg_aid,
        operation_key: dali2rust_contracts::msg::fixed_text_32(&operation_id),
    };
    if !publish_internal_command(publisher, adapter_id, workflow, execute) {
        publish_operation_worker_signal(
            publisher,
            adapter_id,
            workflow,
            OperationWorkerSignal::WorkerFailed,
            Some((ErrorCode::CommandsIngressOverload, "policy_apply_ingress_overload")),
            Origin::Internal,
            counters,
        );
    }
}

fn publish_internal_command<P>(publisher: &BusPublisher, adapter_id: BusId, workflow: u64, payload: P) -> bool
where
    dali2rust_contracts::msg::BusCommandPayload: From<P>,
{
    let envelope = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        adapter_id.0,
        Some(Origin::Internal),
        payload,
    );
    publisher.try_publish(
        dali2rust_bus::BusChannel::Commands,
        dali2rust_bus::BusFrame::command(envelope),
    ) == dali2rust_bus::PublishResult::Queued
}

fn discover_refresh_known(
    controller: &mut impl DaliApplicationController,
    shorts: &[u8],
    scan: &Scan<'_>,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    for &short_address in shorts {
        if let Some(found) = detect_device(controller, short_address)? {
            publish_discovery_progress(scan, &found);
        }
    }
    Ok(())
}

fn discover_scan_all(
    controller: &mut impl DaliApplicationController,
    scan: &Scan<'_>,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    let retries = scan.env.runtime_config.discovery_step_retries;
    let attempts = retries.saturating_add(1);
    for attempt in 0..attempts {
        let mut confirmed_mask: u64 = 0;
        let summary = discover_known_control_gear_with_retry_budget(
            controller,
            retries,
            &mut |found: &DiscoveredDevice| {
                confirmed_mask |= 1u64 << (found.short_address & 0x3F);
                if found.type_enum_degraded {
                    scan.env.counters
                        .discovery_device_type_degraded
                        .fetch_add(1, Ordering::Relaxed);
                }
                publish_discovery_progress(scan, found);
            },
        )?;
        let had_progress = summary.described > 0;
        let error = summary.error;
        match error {
            None => {
                publish_scan_reconciled(scan, confirmed_mask);
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

fn publish_scan_reconciled(scan: &Scan<'_>, confirmed_mask: u64) {
    let reconciled = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, scan.w, scan.env.adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryScanReconciledEvent { registry_adapter_id: scan.reg_aid, confirmed_mask });
    publish_event_required(scan.env.publisher, reconciled, scan.env.counters, PublishKind::Series, "discovery-reconciled");
}

fn commission_unaddressed_loop(
    controller: &mut impl DaliApplicationController,
    used_shorts: &mut Vec<u8>,
    scan: &Scan<'_>,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    let attempts = scan.env.runtime_config.discovery_step_retries.saturating_add(1);
    for attempt in 0..attempts {
        match commission_unaddressed_once(controller, used_shorts, scan) {
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
    used_shorts: &mut Vec<u8>,
    scan: &Scan<'_>,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    crate::runtime::executor::begin_unaddressed_commissioning(controller)?;
    let result = commission_unaddressed_drain(controller, used_shorts, scan);
    let cleanup = crate::runtime::executor::terminate_unaddressed_commissioning(controller);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn commission_unaddressed_drain(
    controller: &mut impl DaliApplicationController,
    used_shorts: &mut Vec<u8>,
    scan: &Scan<'_>,
) -> Result<(), crate::runtime::executor::SemanticDaliError> {
    loop {
        match commission_next_unaddressed_with_retry_budget(
            controller,
            used_shorts,
            scan.env.runtime_config.discovery_step_retries,
        ) {
            Ok(Some(found)) => {
                used_shorts.push(found.short_address);
                publish_discovery_progress(scan, &found);
            }
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        }
    }
}

fn publish_discovery_progress(scan: &Scan<'_>, found: &DiscoveredDevice) {
    let prog = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        scan.w,
        scan.env.adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliDiscoveryProgressEvent {
            registry_adapter_id: scan.reg_aid,
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
    if publish_event_required(scan.env.publisher, prog, scan.env.counters, PublishKind::Series, "discovery-progress") {
        scan.env.counters
            .discovery_progress_events_published
            .fetch_add(1, Ordering::Relaxed);
    }
}
