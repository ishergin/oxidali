use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfirmedWritableAttributes {
    pub fade_time_ms: Option<u32>,
    pub fade_rate: Option<u8>,
    pub power_on_level: Option<u8>,
    pub system_failure_level: Option<u8>,
    pub extended_fade_time_ms: Option<u16>,
    pub tc_coolest_mirek: Option<u16>,
    pub tc_warmest_mirek: Option<u16>,
    pub min_level: Option<u8>,
    pub max_level: Option<u8>,
    pub dimming_curve: Option<u8>,
    pub physical_minimum_readback: Option<u8>,
}

impl ConfirmedWritableAttributes {
    pub const fn is_empty(&self) -> bool {
        self.fade_time_ms.is_none()
            && self.fade_rate.is_none()
            && self.power_on_level.is_none()
            && self.system_failure_level.is_none()
            && self.extended_fade_time_ms.is_none()
            && self.tc_coolest_mirek.is_none()
            && self.tc_warmest_mirek.is_none()
            && self.min_level.is_none()
            && self.max_level.is_none()
            && self.dimming_curve.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAttributesExecution {
    pub confirmed: ConfirmedWritableAttributes,
    pub error: Option<SemanticDaliError>,
}

// IEC 62386-102 §3.13
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadBack {
    Proved(u8),
    Refused,
    Unanswered,
}

#[derive(Debug, Default)]
pub(super) struct WriteTally {
    pub(super) confirmed: ConfirmedWritableAttributes,
    pub(super) unanswered: bool,
}

impl WriteTally {
    pub(super) fn proved(&mut self, read_back: ReadBack) -> Option<u8> {
        match read_back {
            ReadBack::Proved(value) => Some(value),
            ReadBack::Refused => None,
            ReadBack::Unanswered => {
                self.unanswered = true;
                None
            }
        }
    }

    fn into_execution(self, error: Option<SemanticDaliError>) -> WriteAttributesExecution {
        let unanswered = SemanticDaliError::OperationFailed(VERIFY_UNANSWERED_MESSAGE);
        WriteAttributesExecution {
            confirmed: self.confirmed,
            error: error.or(self.unanswered.then_some(unanswered)),
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "mirrors the public write surface")]
pub fn write_short_attributes(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    fade_time_ms: Option<u32>,
    fade_rate: Option<u8>,
    power_on_level: Option<u8>,
    system_failure_level: Option<u8>,
    extended_fade_time_ms: Option<u16>,
    tc_limits_mirek: (Option<u16>, Option<u16>),
    min_max_levels: (Option<u8>, Option<u8>),
    dimming_curve: Option<u8>,
) -> WriteAttributesExecution {
    let mut tally = WriteTally::default();
    let error = apply_short_attribute_writes(
        controller,
        short_address,
        &mut tally,
        fade_time_ms,
        fade_rate,
        power_on_level,
        system_failure_level,
        extended_fade_time_ms,
        tc_limits_mirek,
        min_max_levels,
        dimming_curve,
    )
    .err();
    tally.into_execution(error)
}

#[allow(clippy::too_many_arguments, reason = "mirrors the public write surface")]
fn apply_short_attribute_writes(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    tally: &mut WriteTally,
    fade_time_ms: Option<u32>,
    fade_rate: Option<u8>,
    power_on_level: Option<u8>,
    system_failure_level: Option<u8>,
    extended_fade_time_ms: Option<u16>,
    tc_limits_mirek: (Option<u16>, Option<u16>),
    min_max_levels: (Option<u8>, Option<u8>),
    dimming_curve: Option<u8>,
) -> Result<(), SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    if let Some(error) = write_fade_params(controller, address, tally, fade_time_ms, fade_rate) {
        return Err(error);
    }
    if let Some(error) =
        write_levels(controller, address, tally, power_on_level, system_failure_level)
    {
        return Err(error);
    }
    write_min_max_levels(controller, address, tally, min_max_levels)?;
    write_extended_fade_time(controller, address, tally, extended_fade_time_ms)?;
    write_tc_limits(controller, address, &mut tally.confirmed, tc_limits_mirek)?;
    write_dimming_curve(controller, address, tally, dimming_curve)
}

pub(super) fn reread_physical_minimum(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    confirmed: &mut ConfirmedWritableAttributes,
) -> Result<(), SemanticDaliError> {
    if let Ok(v) = send_standard_query(controller, address, StandardCommand::QueryPhysicalMinimum)
    {
        confirmed.physical_minimum_readback = v;
    }
    Ok(())
}

pub(super) fn send_dtr0_config_verified<C: DaliApplicationController>(
    controller: &mut C,
    address: DaliAddress,
    dtr0: u8,
    command: StandardCommand,
    mut read_back: impl FnMut(&mut C) -> Result<Option<u8>, SemanticDaliError>,
) -> Result<ReadBack, SemanticDaliError> {
    controller.step_boundary();
    for _ in 0..=PROGRAM_VERIFY_REPAIRS {
        let verified = controller.transaction(|controller| {
            send_dtr0_backed_standard(controller, address, dtr0, command)?;
            read_back(controller)
        })?;
        match verified {
            Some(v) if v == dtr0 => return Ok(ReadBack::Proved(v)),
            None => return Ok(ReadBack::Unanswered),
            Some(_) => {}
        }
    }
    Ok(ReadBack::Refused)
}

fn send_dtr0_config_accepted<C: DaliApplicationController>(
    controller: &mut C,
    address: DaliAddress,
    dtr0: u8,
    command: StandardCommand,
    mut read_back: impl FnMut(&mut C) -> Result<Option<u8>, SemanticDaliError>,
) -> Result<ReadBack, SemanticDaliError> {
    controller.step_boundary();
    let mut previous: Option<u8> = None;
    for _ in 0..=PROGRAM_VERIFY_REPAIRS {
        let answer = controller.transaction(|controller| {
            send_dtr0_backed_standard(controller, address, dtr0, command)?;
            read_back(controller)
        })?;
        match answer {
            Some(v) if v == dtr0 => return Ok(ReadBack::Proved(v)),
            Some(v) if previous == Some(v) => return Ok(ReadBack::Proved(v)),
            Some(v) => previous = Some(v),
            None => return Ok(ReadBack::Unanswered),
        }
    }
    Ok(ReadBack::Refused)
}

// IEC 62386-102 §9.6
fn write_min_max_levels(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    (min_level, max_level): (Option<u8>, Option<u8>),
) -> Result<(), SemanticDaliError> {
    if let Some(min) = min_level {
        tally.confirmed.min_level = tally.proved(write_level_bound(controller, address, min, true)?);
    }
    if let Some(max) = max_level {
        tally.confirmed.max_level = tally.proved(write_level_bound(controller, address, max, false)?);
    }
    if let (Some(min), Some(_)) = (min_level, max_level) {
        if matches!(tally.confirmed.min_level, Some(accepted) if accepted < min) {
            tally.confirmed.min_level =
                tally.proved(write_level_bound(controller, address, min, true)?);
        }
    }
    Ok(())
}

fn write_level_bound(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    dtr0: u8,
    min_bound: bool,
) -> Result<ReadBack, SemanticDaliError> {
    let (set, query) = if min_bound {
        (StandardCommand::SetMinLevel, StandardCommand::QueryMinLevel)
    } else {
        (StandardCommand::SetMaxLevel, StandardCommand::QueryMaxLevel)
    };
    let verify = |c: &mut _| send_standard_query(c, address, query);
    send_dtr0_config_accepted(controller, address, dtr0, set, verify)
}

fn write_fade_params(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    fade_time_ms: Option<u32>,
    fade_rate: Option<u8>,
) -> Option<SemanticDaliError> {
    if let Some(ms) = fade_time_ms {
        let dtr0 = fade_time_dtr0_from_ms(ms);
        let verify = |c: &mut _| {
            Ok(send_standard_query(c, address, StandardCommand::QueryFadeTimeFadeRate)?
                .map(|b| b >> 4))
        };
        match send_dtr0_config_verified(controller, address, dtr0, StandardCommand::SetFadeTime, verify) {
            Err(error) => return Some(error),
            Ok(read_back) => {
                if let Some(code) = tally.proved(read_back) {
                    tally.confirmed.fade_time_ms = Some(fade_time_ms_from_dtr0(code));
                }
            }
        }
    }
    if let Some(rate) = fade_rate {
        let verify = |c: &mut _| {
            Ok(send_standard_query(c, address, StandardCommand::QueryFadeTimeFadeRate)?
                .map(|b| b & 0x0F))
        };
        match send_dtr0_config_verified(controller, address, rate, StandardCommand::SetFadeRate, verify) {
            Err(error) => return Some(error),
            Ok(read_back) => tally.confirmed.fade_rate = tally.proved(read_back),
        }
    }
    None
}

fn write_levels(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    power_on_level: Option<u8>,
    system_failure_level: Option<u8>,
) -> Option<SemanticDaliError> {
    if let Some(level) = power_on_level {
        let verify =
            |c: &mut _| send_standard_query(c, address, StandardCommand::QueryPowerOnLevel);
        match send_dtr0_config_verified(controller, address, level, StandardCommand::SetPowerOnLevel, verify)
        {
            Err(error) => return Some(error),
            Ok(read_back) => tally.confirmed.power_on_level = tally.proved(read_back),
        }
    }
    if let Some(level) = system_failure_level {
        let verify =
            |c: &mut _| send_standard_query(c, address, StandardCommand::QuerySystemFailureLevel);
        match send_dtr0_config_verified(
            controller,
            address,
            level,
            StandardCommand::SetSystemFailureLevel,
            verify,
        ) {
            Err(error) => return Some(error),
            Ok(read_back) => tally.confirmed.system_failure_level = tally.proved(read_back),
        }
    }
    None
}
