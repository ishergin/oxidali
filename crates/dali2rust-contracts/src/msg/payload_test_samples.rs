use crate::msg::{
    fixed_text_32, fixed_text_48, fixed_text_64, fixed_text_96, BusCommandPayload, BusEventPayload, ColorMode,
    ColorValue, CommandEnvelope, CompactErrorPayload, Dt6ReadSnapshot, ErrorCode, EventEnvelope,
    FailureStatus, FixedText32, FixedText48, FixedText64, FixedText96, LightSetpoint, RuntimeObservation,
    RuntimeSource, StatusFlags,
};

pub(crate) fn worst_rule_chunk(
) -> crate::msg::FixedItems<u8, { crate::msg::commands::RULE_SOURCE_CHUNK_BYTES }> {
    let mut bytes = crate::msg::FixedItems::new();
    for _ in 0..crate::msg::commands::RULE_SOURCE_CHUNK_BYTES {
        let _ = bytes.push(u8::MAX);
    }
    bytes
}

pub(crate) fn worst_text64() -> FixedText64 {
    fixed_text_64("0123456789012345678901234567890123456789012345678901234567890123")
}

pub(crate) fn worst_text96() -> FixedText96 {
    fixed_text_96("012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345")
}

pub(crate) fn worst_text32() -> FixedText32 {
    fixed_text_32("01234567890123456789012345678901")
}

pub(crate) fn worst_text48() -> FixedText48 {
    fixed_text_48("012345678901234567890123456789012345678901234567")
}

pub(crate) fn worst_compact_error_payload() -> CompactErrorPayload {
    CompactErrorPayload {
        code: ErrorCode::ExecutionFailed,
        message: worst_text32(),
    }
}

pub(crate) fn worst_color_value() -> ColorValue {
    ColorValue {
        mode: ColorMode::Rgbwaf,
        color_temperature_kelvin: u16::MAX,
        x: u16::MAX,
        y: u16::MAX,
        r: u8::MAX,
        g: u8::MAX,
        b: u8::MAX,
        w: u8::MAX,
        a: u8::MAX,
        f: u8::MAX,
    }
}

pub(crate) fn worst_setpoint() -> LightSetpoint {
    LightSetpoint {
        power: crate::msg::PowerState::On,
        level: u8::MAX,
        color: Some(worst_color_value()),
    }
}

pub(crate) fn worst_observation() -> RuntimeObservation {
    RuntimeObservation {
        status_flags: Some(StatusFlags {
            raw: u8::MAX,
            lamp_failure: true,
            gear_failure: true,
            lamp_on: true,
            limit_error: true,
            fade_running: true,
            reset_state: true,
            missing_short_address: true,
            power_cycle_seen: true,
        }),
        failure_status: Some(FailureStatus {
            raw: u8::MAX,
            lamp_failure: true,
            gear_failure: true,
            communication_failure: true,
            source: RuntimeSource::Poller,
        }),
        value_source: Some(RuntimeSource::Poller),
        last_seen_ms: Some(u64::MAX),
        last_dapc_source: crate::msg::LastDapcSource::Scene,
        error: Some(worst_compact_error_payload()),
    }
}

pub(crate) fn worst_dt6_snapshot() -> Dt6ReadSnapshot {
    Dt6ReadSnapshot {
        gear_type: Some(u8::MAX),
        dimming_curve: Some(u8::MAX),
        possible_operating_mode: Some(u8::MAX),
        features: Some(u8::MAX),
        failure_status: Some(u8::MAX),
        short_circuit: Some(u8::MAX),
        open_circuit: Some(u8::MAX),
        load_decrease: Some(u8::MAX),
        load_increase: Some(u8::MAX),
        current_protector_active: Some(u8::MAX),
        thermal_shutdown: Some(u8::MAX),
        thermal_overload: Some(u8::MAX),
        reference_running: Some(u8::MAX),
        reference_measurement_failed: Some(u8::MAX),
        current_protector_enabled: Some(u8::MAX),
        operating_mode: Some(u8::MAX),
        fast_fade_time: Some(u8::MAX),
        min_fast_fade_time: Some(u8::MAX),
        extended_version_number: Some(u8::MAX),
    }
}

pub(crate) fn worst_group_matrix_rows() -> crate::msg::GroupMatrixDesiredRowList {
    let mut rows = crate::msg::GroupMatrixDesiredRowList::new();
    for _ in 0..crate::msg::MAX_GROUP_MATRIX_ROWS_PER_COMMAND {
        rows.push(crate::msg::GroupMatrixDesiredRow {
            virtual_lamp_id: 63,
            desired_groups_mask: u16::MAX,
        })
        .expect("row capacity");
    }
    rows
}

pub(crate) fn worst_scene_target_state() -> crate::msg::DaliSceneTargetState {
    crate::msg::DaliSceneTargetState {
        power: Some(crate::msg::PowerState::Unknown),
        level: Some(u8::MAX),
        color: Some(worst_color_value()),
    }
}

pub(crate) fn worst_scene_matrix_rows() -> crate::msg::SceneMatrixDesiredRowList {
    let mut rows = crate::msg::SceneMatrixDesiredRowList::new();
    for _ in 0..crate::msg::MAX_SCENE_MATRIX_ROWS_PER_COMMAND {
        rows.push(crate::msg::SceneMatrixDesiredRow {
            virtual_lamp_id: 63,
            included: true,
            target: Some(worst_scene_target_state()),
        })
        .expect("row capacity");
    }
    rows
}

pub(crate) fn worst_runtime_update_entry() -> crate::msg::RuntimeRegistryUpdateEntry {
    crate::msg::RuntimeRegistryUpdateEntry {
        virtual_lamp_id: Some(63),
        short_address: Some(63),
        setpoint: Some(worst_setpoint()),
        observation: Some(worst_observation()),
        last_dapc_source: Some(crate::msg::LastDapcSource::Scene),
        source: RuntimeSource::Poller,
        observed_at_mono_ms: Some(u32::MAX),
    }
}

pub(crate) fn worst_attributes_read_event() -> crate::msg::DaliAttributesReadEvent {
    crate::msg::DaliAttributesReadEvent {
        registry_adapter_id: u8::MAX,
        short_address: 63,
        last_chunk: true,
        chunk: crate::msg::DaliAttributeReadChunk::Dt6Led {
            snapshot: worst_dt6_snapshot(),
        },
    }
}

pub(crate) fn worst_attribute_read_chunks() -> [crate::msg::DaliAttributeReadChunk; 10] {
    use crate::msg::DaliAttributeReadChunk as Chunk;
    [
        Chunk::Identity {
            random_address: u32::MAX,
        },
        Chunk::Common102 {
            version: Some(u8::MAX),
            device_type: Some(u8::MAX),
            physical_minimum: Some(u8::MAX),
            min_level: Some(u8::MAX),
            max_level: Some(u8::MAX),
            power_on_level: Some(u8::MAX),
            system_failure_level: Some(u8::MAX),
            fade_time_ms: Some(u32::MAX),
            fade_rate: Some(u8::MAX),
            supported_device_types: Some(crate::msg::DeviceTypeSet::from_bits(u64::MAX)),
            light_source_type: Some(u8::MAX),
            light_source_types: Some(u32::MAX),
        },
        Chunk::Dt8Color {
            color_mode: ColorMode::Unknown,
            xy_capable: true,
            tc_capable: true,
            rgb_capable: true,
            color_type: Some(u8::MAX),
            color_value_0: Some(u16::MAX),
            color_value_1: Some(u16::MAX),
            color_value_2: Some(u16::MAX),
            tc_coolest_mirek: Some(u16::MAX),
            tc_warmest_mirek: Some(u16::MAX),
            gear_features: Some(u8::MAX),
            rgbwaf_capable: true,
            rgbwaf_control: Some(u8::MAX),
        },
        Chunk::Groups {
            membership: Some(u16::MAX),
        },
        Chunk::Scenes {
            levels: Some([u8::MAX; 16]),
        },
        Chunk::Dt6Led {
            snapshot: worst_dt6_snapshot(),
        },
        Chunk::Extended {
            fade_time_ms: Some(u16::MAX),
            version_number: Some(u8::MAX),
        },
        Chunk::RuntimeStatus {
            setpoint: Some(worst_setpoint()),
            observation: worst_observation(),
            read_started_mono_ms: u32::MAX,
        },
        Chunk::SceneColour {
            scene: 15,
            level: Some(u8::MAX),
            colour_type: Some(u8::MAX),
            values: [Some(u16::MAX); 6],
        },
        Chunk::ExtendedVersions {
            versions: [Some(crate::msg::ExtendedVersionEntry {
                device_type: u8::MAX,
                version_number: Some(u8::MAX),
            }); crate::msg::MAX_EXTENDED_VERSIONS],
        },
    ]
}

fn worst_meta(channel: crate::msg::ChannelKind, message: crate::msg::MessageKind) -> crate::msg::BusEnvelope {
    let mut meta = match channel {
        crate::msg::ChannelKind::Commands => {
            crate::bus::envelope_commands(u16::MAX, u64::MAX, u16::MAX, None)
        }
        _ => crate::bus::envelope_events(u16::MAX, u64::MAX, u16::MAX, None),
    };
    meta.channel_kind = channel;
    meta.message_kind = message;
    meta
}

pub(crate) fn command_probe(payload: &BusCommandPayload) -> usize {
    let ce = CommandEnvelope {
        meta: worst_meta(
            crate::msg::ChannelKind::Commands,
            crate::msg::MessageKind::Command,
        ),
        payload: payload.clone(),
    };
    crate::bus::encode_command_envelope(&ce)
        .expect("postcard encode of command envelope")
        .len()
}

pub(crate) fn event_probe(payload: &BusEventPayload) -> usize {
    let ev = EventEnvelope {
        meta: worst_meta(
            crate::msg::ChannelKind::Events,
            crate::msg::MessageKind::Event,
        ),
        payload: payload.clone(),
    };
    crate::bus::encode_event_envelope(&ev)
        .expect("postcard encode of event envelope")
        .len()
}

pub fn worst_hcl_targets() -> crate::msg::commands::HclTargetList {
    let mut rows = crate::msg::commands::HclTargetList::new();
    for _ in 0..crate::msg::commands::MAX_HCL_TARGETS_PER_COMMAND {
        let _ = rows.push(crate::msg::commands::HclTargetRow {
            adapter_id: u8::MAX,
            scope: crate::msg::kinds::HclTargetScope::Group,
            group_mask: u16::MAX,
        });
    }
    rows
}

pub fn worst_hcl_points() -> crate::msg::commands::HclPointList {
    let mut rows = crate::msg::commands::HclPointList::new();
    for _ in 0..crate::msg::commands::MAX_HCL_POINTS_PER_COMMAND {
        let _ = rows.push(crate::msg::commands::HclSchedulePointRow {
            time_ref: crate::msg::kinds::HclTimeRef::Sunset,
            offset_minutes: i16::MIN,
            level_mode: crate::msg::kinds::HclLevelMode::Absolute,
            level: Some(u8::MAX),
            color_temperature_kelvin: Some(u16::MAX),
        });
    }
    rows
}
