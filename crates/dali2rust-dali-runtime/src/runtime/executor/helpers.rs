use dali2rust_contracts::msg::ErrorCode;
use dali2rust_domain::dali::commands::{DaliCommand, DaliResponse};
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::devices::dt8_color::{
    Dt8Command, opcode_requires_repeat as dt8_opcode_requires_repeat,
};
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::pres::extended::ExtendedCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_domain::lighting::service::FADE_TIME_MS;

use crate::runtime::config::ContentConfirmPolicy;
use crate::runtime::executor::majority::majority_optional;

pub const PROGRAM_VERIFY_REPAIRS: u8 = 2;

pub const DT8_DEVICE_TYPE: u8 = 8;
pub const DT8_SET_TEMPORARY_X_COORDINATE: u8 = 224;
pub const DT8_SET_TEMPORARY_Y_COORDINATE: u8 = 225;
pub const DT8_ACTIVATE: u8 = 226;
pub const DT8_SET_TEMPERATURE_TC: u8 = 231;
pub const DT8_SET_TEMPORARY_RGB_DIMLEVEL: u8 = 235;
pub const DT8_SET_TEMPORARY_WAF_DIMLEVEL: u8 = 236;
pub const DT8_QUERY_COLOUR_STATUS: u8 = 248;
pub const DT8_QUERY_COLOUR_TYPE_FEATURES: u8 = 249;
pub const DT8_QUERY_COLOUR_VALUE: u8 = 250;
pub use dali2rust_domain::dali::pres::opcode::READ_MEMORY_LOCATION_OPCODE;

pub use dali2rust_domain::dali::devices::dt8_color::{
    COLOUR_VALUE_AMBER as DT8_COLOUR_VALUE_AMBER, COLOUR_VALUE_BLUE as DT8_COLOUR_VALUE_BLUE,
    COLOUR_VALUE_FREECOLOUR as DT8_COLOUR_VALUE_FREECOLOUR,
    COLOUR_VALUE_WHITE as DT8_COLOUR_VALUE_WHITE, COLOUR_VALUE_GREEN as DT8_COLOUR_VALUE_GREEN,
    COLOUR_VALUE_LEVEL_MASK as DT8_COLOUR_VALUE_LEVEL_MASK,
    COLOUR_VALUE_MASK as DT8_COLOUR_VALUE_MASK, COLOUR_VALUE_RED as DT8_COLOUR_VALUE_RED,
    COLOUR_VALUE_TC as DT8_COLOUR_VALUE_TC, COLOUR_VALUE_TC_COOLEST as DT8_COLOUR_VALUE_TC_COOLEST,
    COLOUR_VALUE_TC_PHYSICAL_COOLEST as DT8_COLOUR_VALUE_TC_PHYSICAL_COOLEST,
    COLOUR_VALUE_TC_PHYSICAL_WARMEST as DT8_COLOUR_VALUE_TC_PHYSICAL_WARMEST,
    COLOUR_VALUE_TC_WARMEST as DT8_COLOUR_VALUE_TC_WARMEST, COLOUR_VALUE_X as DT8_COLOUR_VALUE_X,
    COLOUR_VALUE_Y as DT8_COLOUR_VALUE_Y,
    COLOUR_TYPE_BYTE_RGBWAF as DT8_COLOUR_TYPE_RGBWAF, COLOUR_TYPE_BYTE_TC as DT8_COLOUR_TYPE_TC,
    COLOUR_TYPE_BYTE_XY as DT8_COLOUR_TYPE_XY,
    COLOUR_VALUE_REPORT_COLOUR_TYPE as DT8_COLOUR_VALUE_REPORT_COLOUR_TYPE,
    COLOUR_VALUE_REPORT_RED as DT8_COLOUR_VALUE_REPORT_RED,
    COLOUR_VALUE_REPORT_TC as DT8_COLOUR_VALUE_REPORT_TC,
    COLOUR_VALUE_REPORT_X as DT8_COLOUR_VALUE_REPORT_X,
    COLOUR_VALUE_REPORT_Y as DT8_COLOUR_VALUE_REPORT_Y,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticDaliError {
    Conflict(&'static str),
    OperationFailed(&'static str),
}

pub const DEVICE_ABSENT_MESSAGE: &str = "device_absent";

impl SemanticDaliError {
    pub fn code(self) -> ErrorCode {
        match self {
            Self::Conflict(_) => ErrorCode::Conflict,
            _ if self.is_device_absent() => ErrorCode::DeviceAbsent,
            _ if self.is_preempted() => ErrorCode::Preempted,
            Self::OperationFailed(VERIFY_FAILED_MESSAGE) => ErrorCode::VerifyFailed,
            Self::OperationFailed(VERIFY_UNANSWERED_MESSAGE) => ErrorCode::VerifyUnanswered,
            Self::OperationFailed(VERIFY_CONTENDED_MESSAGE) => ErrorCode::VerifyContended,
            Self::OperationFailed(READ_CONTENDED_MESSAGE) => ErrorCode::ReadContended,
            Self::OperationFailed(_) => ErrorCode::OperationFailed,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::Conflict(msg) | Self::OperationFailed(msg) => msg,
        }
    }

    pub fn is_bus_contended(self) -> bool {
        matches!(self, Self::OperationFailed("bus_contended"))
    }

    pub fn is_preempted(self) -> bool {
        matches!(self, Self::OperationFailed(PREEMPTED_MESSAGE))
    }

    pub fn is_device_absent(self) -> bool {
        matches!(self, Self::OperationFailed(DEVICE_ABSENT_MESSAGE))
    }
}

pub const PREEMPTED_MESSAGE: &str = "preempted";

pub const VERIFY_FAILED_MESSAGE: &str = "verify_failed";
pub const VERIFY_UNANSWERED_MESSAGE: &str = "verify_unanswered";
pub const VERIFY_CONTENDED_MESSAGE: &str = "verify_contended";

pub const READ_CONTENDED_MESSAGE: &str = "read_contended";

// IEC 62386-102 §11.7.5
// IEC 62386-103 §11.10.4
pub const RANDOMISE_SETTLE_MS: u64 = 100;

fn map_transport_error(error: &impl core::fmt::Debug) -> SemanticDaliError {
    let rendered = format!("{error:?}");
    if rendered.contains("Preempted") {
        SemanticDaliError::OperationFailed(PREEMPTED_MESSAGE)
    } else if rendered.contains("Collision") || rendered.contains("BusBusy") {
        SemanticDaliError::OperationFailed("bus_contended")
    } else {
        SemanticDaliError::OperationFailed("dali_transport_error")
    }
}

pub fn dali_short_address(short_address: u8) -> Result<DaliAddress, SemanticDaliError> {
    DaliAddress::short(short_address)
        .map_err(|_| SemanticDaliError::Conflict("invalid_short_address"))
}

pub fn kelvin_to_mirek(kelvin: u16) -> Result<u16, SemanticDaliError> {
    dali2rust_domain::registry::kelvin_to_mirek(kelvin)
        .ok_or(SemanticDaliError::Conflict("invalid_color_temperature"))
}

// IEC 62386-102 §9.5.2, Table 4
pub fn fade_time_dtr0_from_ms(fade_time_ms: u32) -> u8 {
    if fade_time_ms == 0 {
        return 0;
    }
    let mut best_step: u8 = 1;
    let mut best_diff: u32 = u32::MAX;
    for step in 1..=15u8 {
        let diff = fade_time_ms.abs_diff(FADE_TIME_MS[step as usize]);
        if diff < best_diff {
            best_diff = diff;
            best_step = step;
        }
    }
    best_step
}

#[inline]
pub fn fade_time_ms_from_dtr0(code: u8) -> u32 {
    FADE_TIME_MS
        .get(code as usize)
        .copied()
        .unwrap_or(FADE_TIME_MS[15])
}

pub fn group_address(group_id: u8) -> Result<DaliAddress, SemanticDaliError> {
    DaliAddress::group(group_id).map_err(|_| SemanticDaliError::Conflict("invalid_group_id"))
}

pub fn send_standard(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: StandardCommand,
) -> Result<DaliResponse, SemanticDaliError> {
    controller
        .send_command(&DaliCommand::Standard { address, command })
        .map_err(|error| map_transport_error(&error))
}

pub fn send_standard_query_observed(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: StandardCommand,
) -> Result<(Option<u8>, bool), SemanticDaliError> {
    let (response, contended) = controller
        .send_command_observed(&DaliCommand::Standard { address, command })
        .map_err(|error| map_transport_error(&error))?;
    Ok((response.value(), contended))
}

pub fn send_standard_response(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: StandardCommand,
) -> Result<DaliResponse, SemanticDaliError> {
    send_standard(controller, address, command)
}

pub fn send_standard_query(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: StandardCommand,
) -> Result<Option<u8>, SemanticDaliError> {
    Ok(send_standard(controller, address, command)?.value())
}

pub fn send_standard_query_stable(
    controller: &mut impl DaliApplicationController,
    policy: ContentConfirmPolicy,
    address: DaliAddress,
    command: StandardCommand,
) -> Result<Option<u8>, SemanticDaliError> {
    confirm_observed_query(controller, policy, |controller| {
        send_standard_query_observed(controller, address, command)
    })
}

pub fn send_standard_query_once(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: StandardCommand,
) -> Result<Option<u8>, SemanticDaliError> {
    let (backward, _) = controller
        .send_raw_once(
            DaliCommand::Standard { address, command }.to_forward_frame(),
            true,
        )
        .map_err(|error| map_transport_error(&error))?;
    Ok(backward.value())
}

pub struct MandatorySilenceBreaker {
    consecutive: u8,
}

pub const MID_READ_SILENCE_BUDGET: u8 = 3;

impl MandatorySilenceBreaker {
    pub fn new() -> Self {
        Self { consecutive: 0 }
    }

    pub fn observe(&mut self, heard: bool) -> Result<(), SemanticDaliError> {
        if heard {
            self.consecutive = 0;
            return Ok(());
        }
        self.consecutive = self.consecutive.saturating_add(1);
        if self.consecutive >= MID_READ_SILENCE_BUDGET {
            return Err(SemanticDaliError::OperationFailed(DEVICE_ABSENT_MESSAGE));
        }
        Ok(())
    }
}

impl Default for MandatorySilenceBreaker {
    fn default() -> Self {
        Self::new()
    }
}

pub fn send_mandatory_query(
    breaker: &mut MandatorySilenceBreaker,
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: StandardCommand,
) -> Result<Option<u8>, SemanticDaliError> {
    let (value, contended) = send_standard_query_observed(controller, address, command)?;
    breaker.observe(value.is_some() || contended)?;
    Ok(value)
}

pub fn send_mandatory_query_stable(
    breaker: &mut MandatorySilenceBreaker,
    controller: &mut impl DaliApplicationController,
    policy: ContentConfirmPolicy,
    address: DaliAddress,
    command: StandardCommand,
) -> Result<Option<u8>, SemanticDaliError> {
    let mut saw_signal = false;
    let value = confirm_observed_query(controller, policy, |controller| {
        let observed = send_standard_query_observed(controller, address, command)?;
        saw_signal |= observed.0.is_some() || observed.1;
        Ok(observed)
    })?;
    breaker.observe(saw_signal || value.is_some())?;
    Ok(value)
}

pub fn send_extended_command(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: ExtendedCommand,
) -> Result<DaliResponse, SemanticDaliError> {
    controller
        .send_command(&DaliCommand::Extended { address, command })
        .map_err(|error| map_transport_error(&error))
}

pub fn send_extended_query_observed(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: ExtendedCommand,
) -> Result<(Option<u8>, bool), SemanticDaliError> {
    let (response, contended) = controller
        .send_command_observed(&DaliCommand::Extended { address, command })
        .map_err(|error| map_transport_error(&error))?;
    Ok((response.value(), contended))
}

pub fn send_extended_query(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: ExtendedCommand,
) -> Result<Option<u8>, SemanticDaliError> {
    Ok(send_extended_command(controller, address, command)?.value())
}

pub fn send_extended_query_stable(
    controller: &mut impl DaliApplicationController,
    policy: ContentConfirmPolicy,
    address: DaliAddress,
    command: ExtendedCommand,
) -> Result<Option<u8>, SemanticDaliError> {
    confirm_observed_query(controller, policy, |controller| {
        send_extended_query_observed(controller, address, command)
    })
}

pub fn send_special(
    controller: &mut impl DaliApplicationController,
    command: SpecialCommand,
) -> Result<(), SemanticDaliError> {
    controller
        .send_command(&DaliCommand::Special(command))
        .map_err(|error| map_transport_error(&error))?;
    Ok(())
}

pub fn send_special_query(
    controller: &mut impl DaliApplicationController,
    command: SpecialCommand,
) -> Result<Option<u8>, SemanticDaliError> {
    Ok(send_special_response(controller, command)?.value())
}

pub fn send_special_response(
    controller: &mut impl DaliApplicationController,
    command: SpecialCommand,
) -> Result<DaliResponse, SemanticDaliError> {
    controller
        .send_command(&DaliCommand::Special(command))
        .map_err(|error| map_transport_error(&error))
}

pub fn send_dtr0_backed_standard(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    dtr0: u8,
    command: StandardCommand,
) -> Result<(), SemanticDaliError> {
    controller.transaction(|controller| {
        send_special(controller, SpecialCommand::Dtr0(dtr0))?;
        let _ = send_standard(controller, address, command)?;
        Ok(())
    })
}

// IEC 62386-209 §11.3.4.2
pub fn send_dt8_raw(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    opcode: u8,
) -> Result<(), SemanticDaliError> {
    if dt8_opcode_requires_repeat(opcode) {
        let command = Dt8Command::from_opcode(opcode)
            .ok_or(SemanticDaliError::Conflict("dt8_configuration_unmodelled"))?;
        send_extended_command(controller, address, ExtendedCommand::Dt8(command))?;
        return Ok(());
    }
    controller.transaction(|controller| {
        send_special(
            controller,
            SpecialCommand::EnableDeviceType(DT8_DEVICE_TYPE),
        )?;
        controller
            .send_raw(
                ForwardFrame::new(address.encode_address_byte() | 0x01, opcode),
                false,
            )
            .map_err(|error| map_transport_error(&error))?;
        Ok(())
    })
}

// IEC 62386-102 §12.3.15
pub fn send_dtr0_backed_extended(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    dtr0: u8,
    command: ExtendedCommand,
) -> Result<bool, SemanticDaliError> {
    controller.transaction(|controller| {
        send_special(controller, SpecialCommand::Dtr0(dtr0))?;
        let echo = send_standard_query(controller, address, StandardCommand::QueryContentDtr0)?;
        if echo != Some(dtr0) {
            return Ok(false);
        }
        send_extended_command(controller, address, command)?;
        Ok(true)
    })
}

pub fn send_dt8_raw_query(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    opcode: u8,
) -> Result<Option<u8>, SemanticDaliError> {
    Ok(send_dt8_raw_query_observed(controller, address, opcode)?.0)
}

pub fn send_dt8_raw_query_observed(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    opcode: u8,
) -> Result<(Option<u8>, bool), SemanticDaliError> {
    let (response, contended) = controller
        .send_raw_enabled_query(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(DT8_DEVICE_TYPE))
                .to_forward_frame(),
            ForwardFrame::new(address.encode_address_byte() | 0x01, opcode),
        )
        .map_err(|error| map_transport_error(&error))?;
    Ok((response.value(), contended))
}

pub fn send_dt8_raw_query_stable(
    controller: &mut impl DaliApplicationController,
    policy: ContentConfirmPolicy,
    address: DaliAddress,
    opcode: u8,
) -> Result<Option<u8>, SemanticDaliError> {
    confirm_observed_query(controller, policy, |controller| {
        send_dt8_raw_query_observed(controller, address, opcode)
    })
}

pub fn send_raw_query_observed(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    opcode: u8,
) -> Result<(Option<u8>, bool), SemanticDaliError> {
    let (frame, contended) = controller
        .send_raw_observed(
            ForwardFrame::new(address.encode_address_byte() | 0x01, opcode),
            true,
        )
        .map_err(|error| map_transport_error(&error))?;
    Ok((frame.value(), contended))
}

pub fn send_raw_query(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    opcode: u8,
) -> Result<Option<u8>, SemanticDaliError> {
    Ok(controller
        .send_raw(
            ForwardFrame::new(address.encode_address_byte() | 0x01, opcode),
            true,
        )
        .map_err(|error| map_transport_error(&error))?
        .value())
}

pub fn send_raw_query_once(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    opcode: u8,
) -> Result<Option<u8>, SemanticDaliError> {
    send_raw_query_once_observed(controller, address, opcode).map(|(value, _)| value)
}

pub fn send_raw_query_once_observed(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    opcode: u8,
) -> Result<(Option<u8>, bool), SemanticDaliError> {
    let (frame, contended) = controller
        .send_raw_once(
            ForwardFrame::new(address.encode_address_byte() | 0x01, opcode),
            true,
        )
        .map_err(|error| map_transport_error(&error))?;
    Ok((frame.value(), contended))
}

pub(crate) fn program_with_verify_repair<V: Copy>(
    mut drive: impl FnMut() -> Result<Option<V>, SemanticDaliError>,
    converged: impl Fn(Option<V>) -> bool,
) -> Result<Option<V>, SemanticDaliError> {
    let mut readback = drive()?;
    for _ in 0..PROGRAM_VERIFY_REPAIRS {
        if converged(readback) {
            break;
        }
        readback = match drive() {
            Ok(second) => second.or(readback),
            Err(_) => readback,
        };
    }
    Ok(readback)
}

pub(crate) fn confirm_observed_query<C, F, T>(
    controller: &mut C,
    policy: ContentConfirmPolicy,
    mut read_once: F,
) -> Result<Option<T>, SemanticDaliError>
where
    C: DaliApplicationController,
    T: Copy + Eq + core::fmt::Debug,
    F: FnMut(&mut C) -> Result<(Option<T>, bool), SemanticDaliError>,
{
    let (first, contended) = read_once(controller)?;
    if !policy.enabled || !contended {
        return Ok(first);
    }

    log::info!("DALI content-confirm: retrying contended scalar read");
    let mut samples = vec![first];
    for _ in 1..usize::from(policy.effective_max_samples()) {
        samples.push(read_once(controller)?.0);
        if let Some(confirmed) = majority_optional(&samples) {
            return Ok(confirmed);
        }
    }

    log::warn!(
        "DALI content-confirm: no consensus after {} samples",
        samples.len()
    );
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use dali2rust_domain::dali::controller::DaliProductController;
    use dali2rust_domain::dali::frame::FrameError;
    use dali2rust_domain::dali::ses::DaliSession;

    struct ObservedQueryController {
        session: DaliSession,
        responses: VecDeque<(DaliResponse, bool)>,
        commands: Vec<DaliCommand>,
    }

    impl ObservedQueryController {
        fn new(responses: &[(DaliResponse, bool)]) -> Self {
            Self {
                session: DaliSession::new(),
                responses: responses.iter().copied().collect(),
                commands: Vec::new(),
            }
        }
    }

    impl DaliProductController for ObservedQueryController {
        type Error = FrameError;

        fn send_command(&mut self, _cmd: &DaliCommand) -> Result<DaliResponse, Self::Error> {
            panic!("helpers tests expect send_command_observed");
        }

        fn send_command_observed(
            &mut self,
            cmd: &DaliCommand,
        ) -> Result<(DaliResponse, bool), Self::Error> {
            self.commands.push(*cmd);
            self.responses
                .pop_front()
                .ok_or(FrameError::TransportError)
        }

        fn session(&self) -> &DaliSession {
            &self.session
        }
    }

    impl DaliApplicationController for ObservedQueryController {
        fn send_raw(
            &mut self,
            _frame: ForwardFrame,
            _expects_backward: bool,
        ) -> Result<DaliResponse, Self::Error> {
            panic!("helpers tests expect standard observed queries only");
        }
    }

    fn short_address_one() -> DaliAddress {
        DaliAddress::short(1).expect("short address 1")
    }

    #[test]
    fn stable_standard_query_returns_majority_value_after_contended_corruption() {
        let mut controller = ObservedQueryController::new(&[
            (DaliResponse::Answer(0xFF), true),
            (DaliResponse::Answer(0x01), false),
            (DaliResponse::Answer(0x01), false),
        ]);

        let value = send_standard_query_stable(
            &mut controller,
            ContentConfirmPolicy::default(),
            short_address_one(),
            StandardCommand::QueryPhysicalMinimum,
        )
        .expect("stable query");

        assert_eq!(value, Some(0x01));
        assert_eq!(controller.commands.len(), 3);
    }

    #[test]
    fn stable_standard_query_skips_resampling_on_quiet_bus() {
        let mut controller = ObservedQueryController::new(&[(DaliResponse::Answer(0x33), false)]);

        let value = send_standard_query_stable(
            &mut controller,
            ContentConfirmPolicy::default(),
            short_address_one(),
            StandardCommand::QueryVersionNumber,
        )
        .expect("quiet query");

        assert_eq!(value, Some(0x33));
        assert_eq!(controller.commands.len(), 1);
    }

    #[test]
    fn stable_standard_query_returns_none_without_consensus() {
        let mut controller = ObservedQueryController::new(&[
            (DaliResponse::Answer(0x11), true),
            (DaliResponse::Answer(0x22), false),
            (DaliResponse::Answer(0x33), false),
            (DaliResponse::Answer(0x44), false),
            (DaliResponse::Answer(0x55), false),
        ]);

        let value = send_standard_query_stable(
            &mut controller,
            ContentConfirmPolicy::default(),
            short_address_one(),
            StandardCommand::QueryPhysicalMinimum,
        )
        .expect("unstable query");

        assert_eq!(value, None);
        assert_eq!(controller.commands.len(), 5);
    }

    #[test]
    fn stable_standard_query_obeys_disabled_policy() {
        let mut controller = ObservedQueryController::new(&[
            (DaliResponse::Answer(0xFF), true),
            (DaliResponse::Answer(0x01), false),
        ]);

        let value = send_standard_query_stable(
            &mut controller,
            ContentConfirmPolicy::new(false, 5),
            short_address_one(),
            StandardCommand::QueryPhysicalMinimum,
        )
        .expect("disabled policy");

        assert_eq!(value, Some(0xFF));
        assert_eq!(controller.commands.len(), 1);
    }

    #[test]
    fn a_preempted_frame_maps_to_its_own_outcome_not_a_transport_error() {
        let mapped = map_transport_error(&FrameError::Preempted);
        assert_eq!(
            mapped,
            SemanticDaliError::OperationFailed(PREEMPTED_MESSAGE)
        );
        assert!(mapped.is_preempted());
        assert!(
            !mapped.is_bus_contended(),
            "a yield is not contention, so the whole-sequence retry must not re-run it"
        );
    }

    #[test]
    fn contention_and_transport_errors_keep_their_classification() {
        for error in [FrameError::Collision, FrameError::BusBusy] {
            assert!(map_transport_error(&error).is_bus_contended());
            assert!(!map_transport_error(&error).is_preempted());
        }
        let other = map_transport_error(&FrameError::BackwardTimeout);
        assert_eq!(
            other,
            SemanticDaliError::OperationFailed("dali_transport_error")
        );
        assert!(!other.is_preempted());
    }

    #[test]
    fn a_dt8_configuration_opcode_is_refused_by_the_raw_helper() {
        const STORE_TY_PRIMARY_N: u8 = 240;

        let mut controller = ObservedQueryController::new(&[]);
        let error = send_dt8_raw(&mut controller, short_address_one(), STORE_TY_PRIMARY_N)
            .expect_err("a configuration command with no typed variant cannot be framed");

        assert_eq!(
            error,
            SemanticDaliError::Conflict("dt8_configuration_unmodelled")
        );
        assert!(
            controller.commands.is_empty(),
            "nothing may reach the wire in a form the gear will ignore"
        );
    }

    #[test]
    fn a_modelled_configuration_opcode_is_rerouted_to_the_typed_pair() {
        const STORE_COLOUR_TEMPERATURE_TC_LIMIT: u8 = 242;

        struct RecordingController {
            session: DaliSession,
            commands: Vec<DaliCommand>,
        }

        impl DaliProductController for RecordingController {
            type Error = FrameError;

            fn send_command(&mut self, cmd: &DaliCommand) -> Result<DaliResponse, Self::Error> {
                self.commands.push(*cmd);
                Ok(DaliResponse::NoAnswer)
            }

            fn session(&self) -> &DaliSession {
                &self.session
            }
        }

        impl DaliApplicationController for RecordingController {
            fn send_raw(
                &mut self,
                _frame: ForwardFrame,
                _expects_backward: bool,
            ) -> Result<DaliResponse, Self::Error> {
                panic!("a configuration opcode must not be framed raw");
            }
        }

        let mut controller = RecordingController {
            session: DaliSession::new(),
            commands: Vec::new(),
        };
        send_dt8_raw(
            &mut controller,
            short_address_one(),
            STORE_COLOUR_TEMPERATURE_TC_LIMIT,
        )
        .expect("a modelled configuration opcode is sent typed");

        assert_eq!(
            controller.commands,
            vec![DaliCommand::Extended {
                address: short_address_one(),
                command: ExtendedCommand::Dt8(Dt8Command::StoreColourTemperatureTcLimit),
            }],
            "the typed command is the whole output; prelude and pair are the controller's unit"
        );
        assert!(
            DaliCommand::Extended {
                address: short_address_one(),
                command: ExtendedCommand::Dt8(Dt8Command::StoreColourTemperatureTcLimit),
            }
            .requires_repeat(),
            "the variant is what carries the send-twice declaration"
        );
    }
}
