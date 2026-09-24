use super::*;

#[derive(Clone, Copy)]
pub(super) struct ReadProvenance {
    pub(super) value_source: RuntimeSource,
    pub(super) started_mono_ms: u32,
}

#[allow(clippy::too_many_arguments, reason = "origin threads provenance through one extra param")]
pub(super) fn handle_read_attributes(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    origin: Origin,
    ar: &dali2rust_contracts::msg::DaliReadAttributesCommand,
    counters: &DaliWorkerCounters,
) {
    let w = correlation_id;
    let provenance = ReadProvenance {
        value_source: RuntimeSource::from_origin(origin).unwrap_or(RuntimeSource::Api),
        started_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms(),
    };
    publish_worker_started(publisher, adapter_id, w, origin, counters);
    let groups = decode_attribute_groups_mask(ar.attribute_groups_mask);
    let mut outcomes = crate::runtime::executor::AttributeReadOutcomes::for_request(
        groups.as_slice(),
        ar.memory_banks != dali2rust_contracts::msg::MemoryBankReadPreset::None,
    );
    let terminal = read_attributes_terminal(
        controller,
        runtime_config,
        publisher,
        adapter_id,
        w,
        provenance,
        ar,
        groups.as_slice(),
        &mut outcomes,
        counters,
    );
    publish_attribute_read_outcomes(publisher, adapter_id, w, ar, origin, &outcomes, counters);
    publish_read_attributes_terminal_signal(publisher, adapter_id, w, terminal, origin, counters);
    counters
        .semantic_read_attributes_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn publish_read_attributes_terminal_signal(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    terminal: Result<(), (ErrorCode, &'static str)>,
    origin: Origin,
    counters: &DaliWorkerCounters,
) {
    match terminal {
        Ok(()) => publish_worker_succeeded(publisher, adapter_id, w, origin, counters),
        Err((code, message)) => {
            publish_worker_failed(publisher, adapter_id, w, origin, code, message, counters)
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "mirrors the surrounding handler signatures")]
fn read_attributes_terminal(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    provenance: ReadProvenance,
    ar: &dali2rust_contracts::msg::DaliReadAttributesCommand,
    groups: &[dali2rust_contracts::msg::DaliAttributeGroup],
    outcomes: &mut crate::runtime::executor::AttributeReadOutcomes,
    counters: &DaliWorkerCounters,
) -> Result<(), (ErrorCode, &'static str)> {
    let result = match read_attributes(
        controller,
        ar.short_address,
        groups,
        runtime_config.content_confirm,
        outcomes,
    ) {
        Ok(result) => result,
        Err(error) => {
            count_read_abort(error, counters);
            return Err((error.code(), error.message()));
        }
    };
    if !publish_read_attributes_evidence(publisher, adapter_id, w, provenance, ar, &result, counters) {
        return Err((
            ErrorCode::CommandsIngressOverload,
            "runtime_update_publish_failed",
        ));
    }
    read_attributes_memory_bank_stage(controller, publisher, adapter_id, w, ar, outcomes, counters)
}

fn read_attributes_memory_bank_stage(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    ar: &dali2rust_contracts::msg::DaliReadAttributesCommand,
    outcomes: &mut crate::runtime::executor::AttributeReadOutcomes,
    counters: &DaliWorkerCounters,
) -> Result<(), (ErrorCode, &'static str)> {
    match publish_memory_bank_preset(
        controller,
        publisher,
        adapter_id,
        w,
        ar.registry_adapter_id,
        ar.short_address,
        ar.memory_banks,
        counters,
    ) {
        Ok(()) => {
            if ar.memory_banks != dali2rust_contracts::msg::MemoryBankReadPreset::None {
                outcomes.record(
                    crate::runtime::executor::AttributeReadSection::MemoryBanks,
                    dali2rust_contracts::msg::AttributeGroupReadOutcome::Success,
                );
            }
            Ok(())
        }
        Err(error) => {
            outcomes.record(
                crate::runtime::executor::AttributeReadSection::MemoryBanks,
                crate::runtime::executor::classify_read_abort(error),
            );
            count_read_abort(error, counters);
            Err((error.code(), error.message()))
        }
    }
}

pub(super) fn count_read_abort(
    error: crate::runtime::executor::SemanticDaliError,
    counters: &DaliWorkerCounters,
) {
    use dali2rust_contracts::msg::AttributeGroupReadOutcome;
    let counter = match crate::runtime::executor::classify_read_abort(error) {
        AttributeGroupReadOutcome::ContendedAbort => &counters.read_attributes_contended_aborts,
        AttributeGroupReadOutcome::Preempted => &counters.read_attributes_preempted,
        AttributeGroupReadOutcome::DeviceAbsent => &counters.read_attributes_device_absent,
        AttributeGroupReadOutcome::SequenceIncomplete => {
            &counters.read_attributes_sequence_incomplete
        }
        _ => &counters.read_attributes_transport_aborts,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments, reason = "origin threads provenance through one extra param")]
fn publish_attribute_read_outcomes(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    ar: &dali2rust_contracts::msg::DaliReadAttributesCommand,
    origin: Origin,
    outcomes: &crate::runtime::executor::AttributeReadOutcomes,
    counters: &DaliWorkerCounters,
) {
    let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, w, adapter_id.0, Some(origin), dali2rust_contracts::msg::DaliAttributeReadOutcomesEvent { registry_adapter_id: ar.registry_adapter_id, short_address: ar.short_address, identity: outcomes.identity, runtime_status: outcomes.runtime_status, common_102: outcomes.common_102, dt8_color: outcomes.dt8_color, dt6_led: outcomes.dt6_led, groups: outcomes.groups, scenes: outcomes.scenes, extended: outcomes.extended, memory_banks: outcomes.memory_banks, scene_colours: outcomes.scene_colours });
    publish_event_required(publisher, ev, counters, PublishKind::Series, "attribute-read-outcomes");
}

#[allow(clippy::too_many_arguments, reason = "provenance threads source + stamp through one extra param")]
pub(super) fn publish_read_attributes_evidence(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    provenance: ReadProvenance,
    ar: &dali2rust_contracts::msg::DaliReadAttributesCommand,
    result: &crate::runtime::executor::AttributeReadExecution,
    counters: &DaliWorkerCounters,
) -> bool {
    let chunks = attribute_read_chunks(result, provenance);
    let total = chunks.len();
    for (i, chunk) in chunks.into_iter().enumerate() {
        let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, w, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesReadEvent { registry_adapter_id: ar.registry_adapter_id, short_address: ar.short_address, last_chunk: i + 1 == total, chunk });
        if !publish_event_required(publisher, ev, counters, PublishKind::Series, "attributes-read-chunk") {
            counters.evidence_publish_failed.fetch_add(1, Ordering::Relaxed);
            return false;
        }
    }
    true
}

fn attribute_read_chunks(
    result: &crate::runtime::executor::AttributeReadExecution,
    provenance: ReadProvenance,
) -> Vec<dali2rust_contracts::msg::DaliAttributeReadChunk> {
    use dali2rust_contracts::msg::DaliAttributeReadChunk as Chunk;
    let mut chunks = Vec::with_capacity(8);
    if let Some(sp) = result.runtime_setpoint.as_ref() {
        chunks.push(runtime_status_chunk(result, sp, provenance));
    }
    if let Some(random_address) = result.random_address {
        chunks.push(Chunk::Identity { random_address });
    }
    if result.has_common_102 {
        chunks.push(common102_chunk(result));
    }
    if result.has_dt8_color {
        chunks.push(dt8_color_chunk(result));
    }
    if result.has_groups {
        chunks.push(Chunk::Groups { membership: result.groups_membership });
    }
    if result.has_scenes {
        chunks.push(Chunk::Scenes { levels: scene_levels_fixed(result.scene_levels.as_deref()) });
    }
    if let Some(dt6) = result.dt6.as_ref() {
        chunks.push(Chunk::Dt6Led { snapshot: dt6.clone() });
    }
    if result.has_extended {
        chunks.push(Chunk::Extended {
            fade_time_ms: result.extended_fade_time_ms,
            version_number: result.extended_version_number,
        });
    }
    if let Some(scene_colours) = result.scene_colours.as_ref() {
        chunks.extend(scene_colours.iter().cloned());
    }
    chunks
}

pub(super) fn measured_colour(
    result: &crate::runtime::executor::AttributeReadExecution,
) -> Option<dali2rust_contracts::msg::ColorValue> {
    use dali2rust_contracts::msg::ColorMode;
    match result.dt8_supported {
        None => None,
        Some(false) => Some(colour_of(ColorMode::Brightness)),
        Some(true) if result.has_dt8_color => colour_from_dt8_section(result),
        Some(true) => None,
    }
}

fn colour_of(mode: dali2rust_contracts::msg::ColorMode) -> dali2rust_contracts::msg::ColorValue {
    dali2rust_contracts::msg::ColorValue {
        mode,
        ..Default::default()
    }
}

fn rgbwaf_colour(
    result: &crate::runtime::executor::AttributeReadExecution,
) -> Option<dali2rust_contracts::msg::ColorValue> {
    use dali2rust_contracts::msg::{ColorMode, ColorValue};
    let (r, g, b) = result.dt8_color.rgb.map(decode_rgb_channels)?;
    match (result.dt8_rgbwaf_capable, result.dt8_color.waf) {
        (false, _) => Some(ColorValue {
            r,
            g,
            b,
            ..colour_of(ColorMode::Rgb)
        }),
        (true, Some(waf)) => {
            let (w, a, f) = decode_rgb_channels(waf);
            Some(ColorValue {
                r,
                g,
                b,
                w,
                a,
                f,
                ..colour_of(ColorMode::Rgbwaf)
            })
        }
        (true, None) => None,
    }
}

fn decode_rgb_channels(levels: (u8, u8, u8)) -> (u8, u8, u8) {
    use dali2rust_domain::dali::devices::dt8_color::dim_level_to_srgb_channel;
    let (x, y, z) = levels;
    (
        dim_level_to_srgb_channel(x),
        dim_level_to_srgb_channel(y),
        dim_level_to_srgb_channel(z),
    )
}

fn colour_from_dt8_section(
    result: &crate::runtime::executor::AttributeReadExecution,
) -> Option<dali2rust_contracts::msg::ColorValue> {
    use dali2rust_contracts::msg::{ColorMode, ColorValue};
    let mode = result.dt8_color.active_mode;
    match mode {
        ColorMode::Cct => Some(ColorValue {
            color_temperature_kelvin: result
                .dt8_color.value_2
                .and_then(dali2rust_domain::registry::mirek_to_kelvin)?,
            ..colour_of(mode)
        }),
        ColorMode::Xy => Some(ColorValue {
            x: result.dt8_color.value_0?,
            y: result.dt8_color.value_1?,
            ..colour_of(mode)
        }),
        ColorMode::Rgb | ColorMode::Rgbwaf => rgbwaf_colour(result),
        ColorMode::Brightness => Some(colour_of(mode)),
        ColorMode::None | ColorMode::Unknown => None,
    }
}

fn runtime_status_chunk(
    result: &crate::runtime::executor::AttributeReadExecution,
    setpoint: &dali2rust_contracts::msg::LightSetpoint,
    provenance: ReadProvenance,
) -> dali2rust_contracts::msg::DaliAttributeReadChunk {
    let mut observation = result.runtime_observation.clone().unwrap_or_default();
    observation.value_source = Some(provenance.value_source);
    observation
        .last_seen_ms
        .get_or_insert(unix_wall_clock_millis());
    let setpoint = dali2rust_contracts::msg::LightSetpoint {
        color: measured_colour(result),
        ..setpoint.clone()
    };
    dali2rust_contracts::msg::DaliAttributeReadChunk::RuntimeStatus {
        setpoint: Some(setpoint),
        observation,
        read_started_mono_ms: provenance.started_mono_ms,
    }
}

fn common102_chunk(
    result: &crate::runtime::executor::AttributeReadExecution,
) -> dali2rust_contracts::msg::DaliAttributeReadChunk {
    dali2rust_contracts::msg::DaliAttributeReadChunk::Common102 {
        version: result.c102.version,
        device_type: result.c102.device_type,
        physical_minimum: result.c102.physical_minimum,
        min_level: result.c102.min_level,
        max_level: result.c102.max_level,
        power_on_level: result.c102.power_on_level,
        system_failure_level: result.c102.system_failure_level,
        fade_time_ms: result.c102.fade_time_ms,
        fade_rate: result.c102.fade_rate,
        supported_device_types: result.c102.supported_device_types,
        light_source_type: result.c102.light_source_type,
        light_source_types: result.c102.light_source_types,
    }
}

fn dt8_color_chunk(
    result: &crate::runtime::executor::AttributeReadExecution,
) -> dali2rust_contracts::msg::DaliAttributeReadChunk {
    dali2rust_contracts::msg::DaliAttributeReadChunk::Dt8Color {
        color_mode: result.dt8_color_mode,
        xy_capable: result.dt8_xy_capable,
        tc_capable: result.dt8_tc_capable,
        rgb_capable: result.dt8_rgb_capable,
        color_type: result.dt8_color.color_type,
        color_value_0: result.dt8_color.value_0,
        color_value_1: result.dt8_color.value_1,
        color_value_2: result.dt8_color.value_2,
        tc_coolest_mirek: result.dt8_color.tc_coolest_mirek,
        tc_warmest_mirek: result.dt8_color.tc_warmest_mirek,
        gear_features: result.dt8_color.gear_features,
        rgbwaf_capable: result.dt8_rgbwaf_capable,
        rgbwaf_control: result.dt8_color.rgbwaf_control,
    }
}

pub(super) fn decode_attribute_groups_mask(mask: u8) -> Vec<dali2rust_contracts::msg::DaliAttributeGroup> {
    dali2rust_contracts::msg::DaliAttributeGroup::ALL
        .into_iter()
        .filter(|group| mask & group.mask_bit() != 0)
        .collect()
}
