use super::*;

pub(super) fn write_tc_limits(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    confirmed: &mut ConfirmedWritableAttributes,
    (coolest_mirek, warmest_mirek): (Option<u16>, Option<u16>),
) -> Result<(), SemanticDaliError> {
    if coolest_mirek.is_none() && warmest_mirek.is_none() {
        return Ok(());
    }
    let physical = read_physical_tc_pair(controller, address)?;
    let writes = [
        (TC_LIMIT_SELECTOR_COOLEST, DT8_COLOUR_VALUE_TC_COOLEST, coolest_mirek),
        (TC_LIMIT_SELECTOR_WARMEST, DT8_COLOUR_VALUE_TC_WARMEST, warmest_mirek),
    ];
    for (selector, readback_id, mirek) in writes {
        let Some(mirek) = mirek else { continue };
        store_tc_limit_verified(controller, address, selector, readback_id, mirek, physical)?;
    }
    confirmed.tc_coolest_mirek = read_dt8_color_value_u16(
        controller,
        address,
        DT8_COLOUR_VALUE_TC_COOLEST,
        ContentConfirmPolicy::default(),
    )?;
    confirmed.tc_warmest_mirek = read_dt8_color_value_u16(
        controller,
        address,
        DT8_COLOUR_VALUE_TC_WARMEST,
        ContentConfirmPolicy::default(),
    )?;
    Ok(())
}

fn store_tc_limit_verified(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    selector: u8,
    readback_id: u8,
    mirek: u16,
    physical: (Option<u16>, Option<u16>),
) -> Result<(), SemanticDaliError> {
    let expected = tc_limit_expected(mirek, physical);
    for _ in 0..=PROGRAM_VERIFY_REPAIRS {
        let landed = controller.transaction(|controller| {
            if !stage_tc_limit_dtrs(controller, address, mirek, selector)? {
                return Ok(false);
            }
            send_extended_command(
                controller,
                address,
                ExtendedCommand::Dt8(Dt8Command::StoreColourTemperatureTcLimit),
            )?;
            Ok(
                read_dt8_color_value_u16(
                    controller,
                    address,
                    readback_id,
                    ContentConfirmPolicy::default(),
                )? == Some(expected),
            )
        })?;
        if landed {
            return Ok(());
        }
    }
    Err(SemanticDaliError::OperationFailed("dt8_tc_limit_unconfirmed"))
}

fn read_physical_tc_pair(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
) -> Result<(Option<u16>, Option<u16>), SemanticDaliError> {
    let floor = read_dt8_color_value_u16(
        controller,
        address,
        DT8_COLOUR_VALUE_TC_PHYSICAL_COOLEST,
        ContentConfirmPolicy::default(),
    )?;
    let ceil = read_dt8_color_value_u16(
        controller,
        address,
        DT8_COLOUR_VALUE_TC_PHYSICAL_WARMEST,
        ContentConfirmPolicy::default(),
    )?;
    Ok((floor, ceil))
}

fn tc_limit_expected(mirek: u16, physical: (Option<u16>, Option<u16>)) -> u16 {
    match physical {
        (Some(floor), Some(ceil)) if floor <= ceil => mirek.clamp(floor, ceil),
        _ => mirek,
    }
}

fn stage_tc_limit_dtrs(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    mirek: u16,
    selector: u8,
) -> Result<bool, SemanticDaliError> {
    let proofs = [
        (SpecialCommand::Dtr0(mirek as u8), StandardCommand::QueryContentDtr0, mirek as u8),
        (
            SpecialCommand::Dtr1((mirek >> 8) as u8),
            StandardCommand::QueryContentDtr1,
            (mirek >> 8) as u8,
        ),
        (SpecialCommand::Dtr2(selector), StandardCommand::QueryContentDtr2, selector),
    ];
    for (arm, prove, expected) in proofs {
        send_special(controller, arm)?;
        if send_standard_query(controller, address, prove)? != Some(expected) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, Default)]
pub struct Dt8ColorFields {
    pub color_type: Option<u8>,
    pub active_mode: ColorMode,
    pub value_0: Option<u16>,
    pub value_1: Option<u16>,
    pub value_2: Option<u16>,
    pub rgb: Option<(u8, u8, u8)>,
    pub waf: Option<(u8, u8, u8)>,
    pub tc_coolest_mirek: Option<u16>,
    pub tc_warmest_mirek: Option<u16>,
    pub gear_features: Option<u8>,
    pub rgbwaf_control: Option<u8>,
}

fn read_dt8_gear_features(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<u8>, SemanticDaliError> {
    send_extended_query_stable(
        controller,
        content_confirm,
        address,
        ExtendedCommand::Dt8(Dt8Command::QueryGearFeaturesStatus),
    )
}

fn active_color_type_from_status(status: u8) -> Option<u8> {
    use dali2rust_domain::dali::devices::dt8_color::{
        COLOUR_STATUS_PRIMARY_N_ACTIVE as STATUS_PRIMARY_N_ACTIVE,
        COLOUR_STATUS_RGBWAF_ACTIVE as STATUS_RGBWAF_ACTIVE,
        COLOUR_STATUS_TC_ACTIVE as STATUS_TC_ACTIVE, COLOUR_STATUS_XY_ACTIVE as STATUS_XY_ACTIVE,
    };
    use dali2rust_domain::dali::devices::dt8_color::ColorType;
    if status & STATUS_XY_ACTIVE != 0 {
        Some(ColorType::XyCoordinate as u8)
    } else if status & STATUS_TC_ACTIVE != 0 {
        Some(ColorType::ColorTemperature as u8)
    } else if status & STATUS_PRIMARY_N_ACTIVE != 0 {
        Some(ColorType::PrimaryN as u8)
    } else if status & STATUS_RGBWAF_ACTIVE != 0 {
        Some(ColorType::Rgbwaf as u8)
    } else {
        None
    }
}

const DT8_VALUE_ARM_RETRIES: u8 = 2;

pub(super) fn dt8_value_sample(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    value_id: u8,
) -> Result<(Option<u16>, bool), SemanticDaliError> {
    for attempt in 0..=DT8_VALUE_ARM_RETRIES {
        let sample = controller.transaction(|controller| {
            send_special(controller, SpecialCommand::Dtr0(value_id))?;
            let echo = send_standard_query(controller, address, StandardCommand::QueryContentDtr0)?;
            if echo != Some(value_id) {
                return Ok(None);
            }
            send_special(
                controller,
                SpecialCommand::EnableDeviceType(
                    crate::runtime::executor::helpers::DT8_DEVICE_TYPE,
                ),
            )?;
            let (high, contended) = send_raw_query_once_observed(
                controller,
                address,
                crate::runtime::executor::helpers::DT8_QUERY_COLOUR_VALUE,
            )?;
            let Some(high) = high else {
                return Ok(Some((None, contended)));
            };
            if !colour_value_is_wide(value_id) {
                return Ok(Some((Some(u16::from(high)), contended)));
            }
            let low = send_standard_query(controller, address, StandardCommand::QueryContentDtr0)?;
            Ok(Some(match low {
                Some(low) => (Some((u16::from(high) << 8) | u16::from(low)), contended),
                None => (None, contended),
            }))
        });
        match sample {
            Ok(Some(result)) => return Ok(result),
            Ok(None) => continue,
            Err(error) if attempt == DT8_VALUE_ARM_RETRIES => return Err(error),
            Err(_) => continue,
        }
    }
    Ok((None, false))
}

pub(super) fn dt8_narrow_value_sample(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    value_id: u8,
) -> Result<(Option<u8>, bool), SemanticDaliError> {
    debug_assert!(
        !colour_value_is_wide(value_id),
        "identifier {value_id} is sixteen-bit; sample it with dt8_value_sample"
    );
    let (value, contended) = dt8_value_sample(controller, address, value_id)?;
    Ok((value.and_then(|v| u8::try_from(v).ok()), contended))
}

// IEC 62386-209 §9.9
pub(super) fn read_dt8_color_value_u8(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    value_id: u8,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<u8>, SemanticDaliError> {
    debug_assert!(
        !colour_value_is_wide(value_id),
        "identifier {value_id} is sixteen-bit; read it with read_dt8_color_value_u16"
    );
    let Some(value) = confirm_observed_query(controller, content_confirm, |c| {
        dt8_value_sample(c, address, value_id)
    })?
    else {
        return Ok(None);
    };
    let Ok(value) = u8::try_from(value) else {
        return Ok(None);
    };
    if value == DT8_COLOUR_VALUE_LEVEL_MASK {
        return Ok(None);
    }
    Ok(Some(value))
}

pub(super) fn read_dt8_color_value_u16(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    value_id: u8,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<u16>, SemanticDaliError> {
    debug_assert!(
        colour_value_is_wide(value_id),
        "identifier {value_id} is one byte; read it with read_dt8_color_value_u8"
    );
    let Some(raw) = confirm_observed_query(controller, content_confirm, |c| {
        dt8_value_sample(c, address, value_id)
    })?
    else {
        return Ok(None);
    };
    if raw == DT8_COLOUR_VALUE_MASK {
        return Ok(None);
    }
    Ok(Some(raw))
}

fn read_dt8_rgbwaf_control(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    has_rgbwaf_channels: bool,
) -> Result<Option<u8>, SemanticDaliError> {
    if !has_rgbwaf_channels {
        return Ok(None);
    }
    send_extended_query_stable(
        controller,
        content_confirm,
        address,
        ExtendedCommand::Dt8(Dt8Command::QueryRgbwafControl),
    )
}

pub(super) fn read_dt8_color_values(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    has_rgbwaf_channels: bool,
    has_more_than_rgb_channels: bool,
) -> Result<Dt8ColorFields, SemanticDaliError> {
    let status = send_extended_query_stable(
        controller,
        content_confirm,
        address,
        ExtendedCommand::Dt8(Dt8Command::QueryColourStatus),
    )?;
    let color_type = status.and_then(active_color_type_from_status);
    let active_mode = status.map_or(ColorMode::Unknown, color_mode_from_status);
    let value_0 =
        read_dt8_color_value_u16(controller, address, DT8_COLOUR_VALUE_X, content_confirm)?;
    let value_1 =
        read_dt8_color_value_u16(controller, address, DT8_COLOUR_VALUE_Y, content_confirm)?;
    let value_2 =
        read_dt8_color_value_u16(controller, address, DT8_COLOUR_VALUE_TC, content_confirm)?;
    let (rgb, waf) = read_dt8_channels(
        controller,
        address,
        content_confirm,
        active_mode == ColorMode::Rgb,
        has_more_than_rgb_channels,
    )?;
    let (tc_coolest_mirek, tc_warmest_mirek) =
        read_dt8_tc_limits(controller, address, content_confirm)?;
    let gear_features = read_dt8_gear_features(controller, address, content_confirm)?;
    let rgbwaf_control =
        read_dt8_rgbwaf_control(controller, address, content_confirm, has_rgbwaf_channels)?;
    Ok(Dt8ColorFields {
        color_type,
        active_mode,
        value_0,
        value_1,
        value_2,
        rgb,
        waf,
        tc_coolest_mirek,
        tc_warmest_mirek,
        gear_features,
        rgbwaf_control,
    })
}

pub(super) fn read_dt8_dim_level_trio(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    value_ids: [u8; 3],
) -> Result<ChannelLevels, SemanticDaliError> {
    let mut channels = [0u8; 3];
    for (slot, value_id) in channels.iter_mut().zip(value_ids) {
        let Some(level) = read_dt8_color_value_u8(controller, address, value_id, content_confirm)?
        else {
            return Ok(None);
        };
        *slot = level;
    }
    Ok(Some((channels[0], channels[1], channels[2])))
}

type ChannelLevels = Option<(u8, u8, u8)>;

fn read_dt8_channels(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    rgb_active: bool,
    has_more_than_rgb_channels: bool,
) -> Result<(ChannelLevels, ChannelLevels), SemanticDaliError> {
    if !rgb_active {
        return Ok((None, None));
    }
    let rgb = read_dt8_dim_level_trio(
        controller,
        address,
        content_confirm,
        [
            DT8_COLOUR_VALUE_RED,
            DT8_COLOUR_VALUE_GREEN,
            DT8_COLOUR_VALUE_BLUE,
        ],
    )?;
    let waf = if rgb.is_some() && has_more_than_rgb_channels {
        read_dt8_dim_level_trio(
            controller,
            address,
            content_confirm,
            [
                DT8_COLOUR_VALUE_WHITE,
                DT8_COLOUR_VALUE_AMBER,
                DT8_COLOUR_VALUE_FREECOLOUR,
            ],
        )?
    } else {
        None
    };
    Ok((rgb, waf))
}

pub(super) fn read_dt8_tc_limits(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<(Option<u16>, Option<u16>), SemanticDaliError> {
    let mut coolest =
        read_dt8_color_value_u16(controller, address, DT8_COLOUR_VALUE_TC_COOLEST, content_confirm)?;
    if coolest.is_none() {
        coolest = read_dt8_color_value_u16(
            controller,
            address,
            DT8_COLOUR_VALUE_TC_PHYSICAL_COOLEST,
            content_confirm,
        )?;
    }
    let mut warmest =
        read_dt8_color_value_u16(controller, address, DT8_COLOUR_VALUE_TC_WARMEST, content_confirm)?;
    if warmest.is_none() {
        warmest = read_dt8_color_value_u16(
            controller,
            address,
            DT8_COLOUR_VALUE_TC_PHYSICAL_WARMEST,
            content_confirm,
        )?;
    }
    match (coolest, warmest) {
        (Some(c), Some(w)) if c <= w => Ok((Some(c), Some(w))),
        _ => Ok((None, None)),
    }
}
