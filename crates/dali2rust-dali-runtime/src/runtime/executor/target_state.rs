use dali2rust_contracts::msg::{ColorMode, LightSetpoint, PowerState};
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;

use dali2rust_contracts::msg::ColorValue;

use crate::runtime::executor::helpers::{
    dali_short_address, kelvin_to_mirek, send_dt8_raw, send_dt8_raw_query,
    send_dtr0_backed_extended, send_extended_query, send_special, send_standard,
    send_standard_query, SemanticDaliError, DT8_ACTIVATE, DT8_QUERY_COLOUR_STATUS,
    DT8_SET_TEMPERATURE_TC,
    DT8_SET_TEMPORARY_RGB_DIMLEVEL, DT8_SET_TEMPORARY_WAF_DIMLEVEL,
    DT8_SET_TEMPORARY_X_COORDINATE, DT8_SET_TEMPORARY_Y_COORDINATE, PROGRAM_VERIFY_REPAIRS,
};
use dali2rust_domain::dali::devices::dt8_color::{
    gear_features_automatic_activation, gear_features_store_operand, rgbwaf_control_drives,
    rgbwaf_control_is_target, rgbwaf_control_operand, srgb_channel_to_dim_level, Dt8Command,
    RGBWAF_CONTROL_ALL_CHANNELS, RGBWAF_CONTROL_RGB_CHANNELS,
};
use dali2rust_domain::dali::pres::extended::ExtendedCommand;

use dali2rust_domain::dali::devices::dt8_color::{
    COLOUR_STATUS_RGBWAF_ACTIVE as DT8_STATUS_RGB_ACTIVE,
    COLOUR_STATUS_TC_ACTIVE as DT8_STATUS_TC_ACTIVE,
    COLOUR_STATUS_TC_OUT_OF_RANGE as DT8_STATUS_TC_OUT_OF_RANGE,
    COLOUR_STATUS_XY_ACTIVE as DT8_STATUS_XY_ACTIVE,
    COLOUR_STATUS_XY_OUT_OF_RANGE as DT8_STATUS_XY_OUT_OF_RANGE,
};

// IEC 62386-102 §9.3
const ARC_POWER_LEVEL_OFF: u8 = 0;

pub fn apply_with_sequence_retry<T>(
    sequence_retries: u8,
    mut run: impl FnMut() -> Result<T, SemanticDaliError>,
) -> Result<T, SemanticDaliError> {
    let mut result = run();
    for _ in 0..sequence_retries {
        match result {
            Err(error) if error.is_bus_contended() => result = run(),
            other => return other,
        }
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairAutoActivation {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssertRgbwafControl {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorWritePolicy {
    pub auto_activation: RepairAutoActivation,
    pub rgbwaf_control: AssertRgbwafControl,
}

impl ColorWritePolicy {
    pub const NONE: Self = Self {
        auto_activation: RepairAutoActivation::No,
        rgbwaf_control: AssertRgbwafControl::No,
    };
}

pub fn apply_short_target_state(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    setpoint: &LightSetpoint,
    policy: ColorWritePolicy,
) -> Result<(), SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    if policy.auto_activation == RepairAutoActivation::Yes && setpoint.color.is_some() {
        ensure_automatic_activation(controller, address)?;
    }
    apply_target_state(controller, address, setpoint, policy.rgbwaf_control)
}

// IEC 62386-209 Table 5, Table 8
fn ensure_automatic_activation(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
) -> Result<(), SemanticDaliError> {
    controller.step_boundary();
    for _ in 0..=PROGRAM_VERIFY_REPAIRS {
        let armed = send_dtr0_backed_extended(
            controller,
            address,
            gear_features_store_operand(true),
            ExtendedCommand::Dt8(Dt8Command::StoreGearFeaturesStatus),
        )?;
        if armed {
            let answer = send_extended_query(
                controller,
                address,
                ExtendedCommand::Dt8(Dt8Command::QueryGearFeaturesStatus),
            )?;
            if answer.is_some_and(gear_features_automatic_activation) {
                return Ok(());
            }
        }
    }
    Err(SemanticDaliError::OperationFailed(
        "dt8_auto_activation_unrestored",
    ))
}

pub fn apply_group_target_state(
    controller: &mut impl DaliApplicationController,
    group_id: u8,
    setpoint: &LightSetpoint,
) -> Result<(), SemanticDaliError> {
    let address = super::helpers::group_address(group_id)?;
    apply_target_state(controller, address, setpoint, AssertRgbwafControl::No)
}

pub fn apply_broadcast_target_state(
    controller: &mut impl DaliApplicationController,
    setpoint: &LightSetpoint,
) -> Result<(), SemanticDaliError> {
    apply_target_state(
        controller,
        DaliAddress::Broadcast,
        setpoint,
        AssertRgbwafControl::No,
    )
}

fn apply_target_state(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    setpoint: &LightSetpoint,
    assert_control: AssertRgbwafControl,
) -> Result<(), SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        apply_target_state_unit(controller, address, setpoint, assert_control)
    })
}

fn apply_target_state_unit(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    setpoint: &LightSetpoint,
    assert_control: AssertRgbwafControl,
) -> Result<(), SemanticDaliError> {
    let color_staged = setpoint.states_color();
    if color_staged {
        if let Some(color) = setpoint.color.as_ref() {
            apply_color(controller, address, color, assert_control)?;
        }
    }

    let activated = send_arc_command(controller, address, setpoint, color_staged)?;

    if activated {
        if let Some(driven) = rgbwaf_driven_channels(setpoint, assert_control) {
            verify_rgbwaf_control(controller, address, driven)?;
        }
    }
    Ok(())
}

fn send_arc_command(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    setpoint: &LightSetpoint,
    color_staged: bool,
) -> Result<bool, SemanticDaliError> {
    if setpoint.power == PowerState::Off {
        send_standard(
            controller,
            address,
            StandardCommand::DirectArcPower {
                level: ARC_POWER_LEVEL_OFF,
            },
        )?;
        return Ok(true);
    }

    if setpoint.level > 0 {
        send_standard(
            controller,
            address,
            StandardCommand::DirectArcPower {
                level: setpoint.level,
            },
        )?;
        return Ok(true);
    }

    if setpoint.power == PowerState::On {
        return activate_color_only(controller, address, color_staged);
    }

    if color_staged {
        send_colour_activate(controller, address)?;
        return Ok(true);
    }
    Ok(false)
}

fn activate_color_only(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    color_staged: bool,
) -> Result<bool, SemanticDaliError> {
    if color_staged {
        send_colour_activate(controller, address)?;
    }
    send_standard(controller, address, StandardCommand::GoToLastActiveLevel)?;
    Ok(true)
}

#[inline]
// IEC 62386-209 §9.12.5
fn send_colour_activate(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
) -> Result<(), SemanticDaliError> {
    send_dt8_raw(controller, address, DT8_ACTIVATE)
}

fn apply_color(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    color: &ColorValue,
    assert_control: AssertRgbwafControl,
) -> Result<(), SemanticDaliError> {
    let Some(expected) = stage_color(controller, address, color, assert_control)? else {
        return Ok(());
    };
    if matches!(address, DaliAddress::Short(_))
        && color_status_mismatch(controller, address, expected)?
    {
        stage_color(controller, address, color, assert_control)?;
    }
    Ok(())
}

const fn mode_driven_channels(mode: ColorMode) -> Option<u8> {
    match mode {
        ColorMode::Rgb => Some(RGBWAF_CONTROL_RGB_CHANNELS),
        ColorMode::Rgbwaf => Some(RGBWAF_CONTROL_ALL_CHANNELS),
        _ => None,
    }
}

fn rgbwaf_driven_channels(
    setpoint: &LightSetpoint,
    assert_control: AssertRgbwafControl,
) -> Option<u8> {
    if assert_control != AssertRgbwafControl::Yes {
        return None;
    }
    mode_driven_channels(setpoint.color.as_ref()?.mode)
}

fn ensure_rgbwaf_control(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    driven: u8,
) -> Result<(), SemanticDaliError> {
    if !matches!(address, DaliAddress::Short(_)) {
        return Ok(());
    }
    let answer = send_extended_query(
        controller,
        address,
        ExtendedCommand::Dt8(Dt8Command::QueryRgbwafControl),
    )?;
    if answer.is_none_or(|byte| rgbwaf_control_is_target(byte, driven)) {
        return Ok(());
    }
    for _ in 0..=PROGRAM_VERIFY_REPAIRS {
        if send_dtr0_backed_extended(
            controller,
            address,
            rgbwaf_control_operand(),
            ExtendedCommand::Dt8(Dt8Command::SetTemporaryRgbwafControl),
        )? {
            return Ok(());
        }
    }
    Err(SemanticDaliError::OperationFailed("dt8_rgbwaf_control_unstaged"))
}

// IEC 62386-209 §9.1, §9.12.5
fn verify_rgbwaf_control(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    driven: u8,
) -> Result<(), SemanticDaliError> {
    if !matches!(address, DaliAddress::Short(_)) {
        return Ok(());
    }
    let answer = send_extended_query(
        controller,
        address,
        ExtendedCommand::Dt8(Dt8Command::QueryRgbwafControl),
    )?;
    if answer.is_none_or(|byte| rgbwaf_control_drives(byte, driven)) {
        return Ok(());
    }
    Err(SemanticDaliError::OperationFailed("dt8_rgbwaf_control_refused"))
}

#[derive(Clone, Copy)]
struct ColorVerifyBits {
    type_bit: u8,
    out_of_range_bit: u8,
}

fn stage_color(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    color: &ColorValue,
    assert_control: AssertRgbwafControl,
) -> Result<Option<ColorVerifyBits>, SemanticDaliError> {
    let bits = match color.mode {
        ColorMode::Cct => {
            apply_dt8_cct(controller, address, color.color_temperature_kelvin)?;
            ColorVerifyBits {
                type_bit: DT8_STATUS_TC_ACTIVE,
                out_of_range_bit: DT8_STATUS_TC_OUT_OF_RANGE,
            }
        }
        ColorMode::Xy => {
            apply_dt8_xy(controller, address, color.x, color.y)?;
            ColorVerifyBits {
                type_bit: DT8_STATUS_XY_ACTIVE,
                out_of_range_bit: DT8_STATUS_XY_OUT_OF_RANGE,
            }
        }
        ColorMode::Rgb | ColorMode::Rgbwaf => {
            stage_rgbwaf(controller, address, color, assert_control)?;
            ColorVerifyBits {
                type_bit: DT8_STATUS_RGB_ACTIVE,
                out_of_range_bit: 0,
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(bits))
}

fn color_status_mismatch(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    expected: ColorVerifyBits,
) -> Result<bool, SemanticDaliError> {
    Ok(send_dt8_raw_query(controller, address, DT8_QUERY_COLOUR_STATUS)?
        .map(|status| {
            status & expected.type_bit == 0 || status & expected.out_of_range_bit != 0
        })
        .unwrap_or(false))
}

const DT8_STAGE_ARM_RETRIES: u8 = 2;

const DTR_WRITE: [fn(u8) -> SpecialCommand; 3] = [
    SpecialCommand::Dtr0,
    SpecialCommand::Dtr1,
    SpecialCommand::Dtr2,
];
const DTR_QUERY: [StandardCommand; 3] = [
    StandardCommand::QueryContentDtr0,
    StandardCommand::QueryContentDtr1,
    StandardCommand::QueryContentDtr2,
];

fn stage_dtrs_verified(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    values: &[u8],
) -> Result<(), SemanticDaliError> {
    let provable = matches!(address, DaliAddress::Short(_));
    for _ in 0..=DT8_STAGE_ARM_RETRIES {
        for (write, value) in DTR_WRITE.iter().zip(values) {
            send_special(controller, write(*value))?;
        }
        if !provable || staged_dtrs_confirmed(controller, address, values)? {
            return Ok(());
        }
    }
    Err(SemanticDaliError::OperationFailed("dt8_staging_unconfirmed"))
}

fn staged_dtrs_confirmed(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    values: &[u8],
) -> Result<bool, SemanticDaliError> {
    for (query, value) in DTR_QUERY.iter().zip(values) {
        if send_standard_query(controller, address, *query)? != Some(*value) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn apply_dt8_cct(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    kelvin: u16,
) -> Result<(), SemanticDaliError> {
    let mirek = kelvin_to_mirek(kelvin)?;
    stage_dtrs_verified(
        controller,
        address,
        &[(mirek & 0x00FF) as u8, (mirek >> 8) as u8],
    )?;
    send_dt8_raw(controller, address, DT8_SET_TEMPERATURE_TC)
}

pub(crate) fn apply_dt8_xy(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    x: u16,
    y: u16,
) -> Result<(), SemanticDaliError> {
    controller.transaction_exempt(|controller| {
        stage_dtrs_verified(controller, address, &[(x & 0x00FF) as u8, (x >> 8) as u8])?;
        send_dt8_raw(controller, address, DT8_SET_TEMPORARY_X_COORDINATE)?;

        stage_dtrs_verified(controller, address, &[(y & 0x00FF) as u8, (y >> 8) as u8])?;
        send_dt8_raw(controller, address, DT8_SET_TEMPORARY_Y_COORDINATE)
    })
}

fn stage_rgbwaf(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    color: &ColorValue,
    assert_control: AssertRgbwafControl,
) -> Result<(), SemanticDaliError> {
    if assert_control == AssertRgbwafControl::Yes {
        if let Some(driven) = mode_driven_channels(color.mode) {
            ensure_rgbwaf_control(controller, address, driven)?;
        }
    }
    apply_dt8_rgb(controller, address, color.r, color.g, color.b)?;
    let (w, a, f) = waf_channels(color);
    apply_dt8_waf(controller, address, w, a, f)?;
    Ok(())
}

// IEC 62386-209 §9.1
pub(crate) const fn waf_channels(color: &ColorValue) -> (u8, u8, u8) {
    match color.mode {
        ColorMode::Rgbwaf => (color.w, color.a, color.f),
        _ => (0, 0, 0),
    }
}

pub(crate) fn apply_dt8_rgb(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    r: u8,
    g: u8,
    b: u8,
) -> Result<(), SemanticDaliError> {
    let levels = [
        srgb_channel_to_dim_level(r),
        srgb_channel_to_dim_level(g),
        srgb_channel_to_dim_level(b),
    ];
    stage_dtrs_verified(controller, address, &levels)?;
    send_dt8_raw(controller, address, DT8_SET_TEMPORARY_RGB_DIMLEVEL)
}

pub(crate) fn apply_dt8_waf(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    w: u8,
    a: u8,
    f: u8,
) -> Result<(), SemanticDaliError> {
    let levels = [
        srgb_channel_to_dim_level(w),
        srgb_channel_to_dim_level(a),
        srgb_channel_to_dim_level(f),
    ];
    stage_dtrs_verified(controller, address, &levels)?;
    send_dt8_raw(controller, address, DT8_SET_TEMPORARY_WAF_DIMLEVEL)
}

#[cfg(test)]
mod tests;
