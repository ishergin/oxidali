use super::*;

fn decode_status_flags(raw: u8) -> StatusFlags {
    dali2rust_domain::dali::status::decode(raw)
}

pub(super) fn read_runtime_setpoint(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
) -> Result<Option<RuntimeSectionRead>, SemanticDaliError> {
    let status = send_standard_query(controller, address, StandardCommand::QueryStatus)?
        .ok_or(SemanticDaliError::OperationFailed("query_status_no_answer"))?;
    let level = send_standard_query(controller, address, StandardCommand::QueryActualLevel)?
        .ok_or(SemanticDaliError::OperationFailed(
            "query_actual_level_no_answer",
        ))?;
    let flags = decode_status_flags(status);
    // IEC 62386-102 §11.5.20
    let level_unknown = level == ACTUAL_LEVEL_MASK;
    let failure_suspected = flags.lamp_failure || flags.gear_failure || level_unknown;
    let setpoint = if level_unknown {
        LightSetpoint {
            power: PowerState::Unknown,
            level: 0,
            color: None,
        }
    } else {
        LightSetpoint {
            power: if level == 0 {
                PowerState::Off
            } else {
                PowerState::On
            },
            level,
            color: None,
        }
    };
    let observation = RuntimeObservation {
        status_flags: Some(flags),
        failure_status: None,
        value_source: None,
        last_seen_ms: None,
        last_dapc_source: dali2rust_contracts::msg::LastDapcSource::Unknown,
        error: None,
    };
    Ok(Some(RuntimeSectionRead {
        setpoint,
        observation,
        failure_suspected,
    }))
}

pub(super) struct RuntimeSectionRead {
    pub setpoint: LightSetpoint,
    pub observation: RuntimeObservation,
    // IEC 62386-207 §11.3.4.2
    pub failure_suspected: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Common102Fields {
    pub version: Option<u8>,
    pub device_type: Option<u8>,
    pub physical_minimum: Option<u8>,
    pub min_level: Option<u8>,
    pub max_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub system_failure_level: Option<u8>,
    pub fade_time_ms: Option<u32>,
    pub fade_rate: Option<u8>,
    pub supported_device_types: Option<DeviceTypeSet>,
    pub light_source_type: Option<u8>,
    pub light_source_types: Option<u32>,
}

const LIGHT_SOURCE_TYPE_MASK: u8 = 0xFF;

// IEC 62386-102 §11.5.19
pub(super) fn read_light_source_type(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<(Option<u8>, Option<u32>), SemanticDaliError> {
    controller.transaction(|controller| {
        let mut query = |cmd| send_standard_query_stable(controller, content_confirm, address, cmd);
        let answered = query(StandardCommand::QueryLightSourceType)?;
        if answered != Some(LIGHT_SOURCE_TYPE_MASK) {
            return Ok((answered, None));
        }
        let first = query(StandardCommand::QueryContentDtr0)?;
        let second = query(StandardCommand::QueryContentDtr1)?;
        let third = query(StandardCommand::QueryContentDtr2)?;
        let packed = match (first, second, third) {
            (Some(a), Some(b), Some(c)) => {
                Some((u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c))
            }
            _ => None,
        };
        Ok((answered, packed))
    })
}

pub(super) fn read_common_102_attributes(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    breaker: &mut MandatorySilenceBreaker,
) -> Result<Common102Fields, SemanticDaliError> {
    let mut query =
        |cmd| send_mandatory_query_stable(breaker, controller, content_confirm, address, cmd);
    let version = query(StandardCommand::QueryVersionNumber)?;
    let device_type = query(StandardCommand::QueryDeviceType)?;
    let physical_minimum = query(StandardCommand::QueryPhysicalMinimum)?;
    let min_level = query(StandardCommand::QueryMinLevel)?;
    let max_level = query(StandardCommand::QueryMaxLevel)?;
    let power_on_level = query(StandardCommand::QueryPowerOnLevel)?;
    let system_failure_level = query(StandardCommand::QuerySystemFailureLevel)?;
    let (fade_time_ms, fade_rate) = if let Some(combined) =
        query(StandardCommand::QueryFadeTimeFadeRate)?
    {
        let ft = (combined >> 4) & 0x0F;
        let fr = combined & 0x0F;
        let fp = FadeParams::new(ft, fr);
        (Some(fp.fade_duration_ms()), Some(fr))
    } else {
        (None, None)
    };
    let (light_source_type, light_source_types) =
        read_light_source_type(controller, address, content_confirm)?;
    Ok(Common102Fields {
        version,
        supported_device_types: declared_device_types_from_single_answer(device_type),
        device_type,
        physical_minimum,
        min_level,
        max_level,
        power_on_level,
        system_failure_level,
        fade_time_ms,
        fade_rate,
        light_source_type,
        light_source_types,
    })
}

pub(super) fn read_groups_membership(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    breaker: &mut MandatorySilenceBreaker,
) -> Result<Option<u16>, SemanticDaliError> {
    let first = read_groups_membership_once(controller, address, breaker)?;
    match first {
        Some(mask) if is_doubled_membership(mask) => {
            log::info!("groups membership 0x{mask:04X} has the doubled-byte signature; re-reading");
            let confirmed = read_groups_membership_once(controller, address, breaker)?;
            Ok(confirmed.or(first))
        }
        other => Ok(other),
    }
}

fn read_groups_membership_once(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    breaker: &mut MandatorySilenceBreaker,
) -> Result<Option<u16>, SemanticDaliError> {
    let lo = send_mandatory_query(breaker, controller, address, StandardCommand::QueryGroups0To7)?;
    let hi = send_mandatory_query(breaker, controller, address, StandardCommand::QueryGroups8To15)?;
    Ok(if let (Some(lo), Some(hi)) = (lo, hi) {
        Some(u16::from(lo) | (u16::from(hi) << 8))
    } else {
        None
    })
}

fn is_doubled_membership(mask: u16) -> bool {
    let lo = (mask & 0x00FF) as u8;
    let hi = (mask >> 8) as u8;
    hi == lo && hi != 0
}

pub fn read_group_membership_mask(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
) -> Result<Option<u16>, SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    read_groups_membership(controller, address, &mut MandatorySilenceBreaker::new())
}

pub(super) fn read_scene_levels(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    breaker: &mut MandatorySilenceBreaker,
) -> Result<Option<Vec<u8>>, SemanticDaliError> {
    let mut levels = Vec::with_capacity(16);
    for scene in 0u8..16 {
        let lv = send_mandatory_query(
            breaker,
            controller,
            address,
            StandardCommand::QuerySceneLevel { scene },
        )?;
        let Some(lv) = lv else {
            return Ok(None);
        };
        levels.push(lv);
    }
    Ok(Some(levels))
}
