use super::*;

pub(super) fn write_dimming_curve(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    confirmed: &mut ConfirmedWritableAttributes,
    dimming_curve: Option<u8>,
) -> Result<(), SemanticDaliError> {
    let Some(curve) = dimming_curve else {
        return Ok(());
    };
    controller.step_boundary();
    for _ in 0..=PROGRAM_VERIFY_REPAIRS {
        if !send_dtr0_backed_extended(
            controller,
            address,
            curve,
            ExtendedCommand::Dt6(Dt6Command::SelectDimmingCurve),
        )? {
            continue;
        }
        let read = send_extended_query_stable(
            controller,
            ContentConfirmPolicy::default(),
            address,
            ExtendedCommand::Dt6(Dt6Command::QueryDimmingCurve),
        )?;
        match read {
            Some(v) if v == curve => {
                confirmed.dimming_curve = Some(v);
                return reread_physical_minimum(controller, address, confirmed);
            }
            None => {
                confirmed.dimming_curve = Some(curve);
                return reread_physical_minimum(controller, address, confirmed);
            }
            Some(_) => {}
        }
    }
    Ok(())
}

const DT6_ANSWER_YES: u8 = 0xFF;

pub(super) fn read_dt6_failure_byte(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<Dt6ReadSnapshot>, SemanticDaliError> {
    let Some(raw) = send_extended_query_stable(
        controller,
        content_confirm,
        address,
        ExtendedCommand::Dt6(Dt6Command::QueryFailureStatus),
    )?
    else {
        return Ok(None);
    };
    let bits = decode_failure_status(raw);
    let answer = |set: bool| Some(if set { DT6_ANSWER_YES } else { 0 });
    Ok(Some(Dt6ReadSnapshot {
        failure_status: Some(raw),
        short_circuit: answer(bits.short_circuit),
        open_circuit: answer(bits.open_circuit),
        load_decrease: answer(bits.load_decrease),
        load_increase: answer(bits.load_increase),
        current_protector_active: answer(bits.current_protector_active),
        thermal_shutdown: answer(bits.thermal_shut_down),
        thermal_overload: answer(bits.thermal_overload),
        reference_measurement_failed: answer(bits.reference_measurement_failed),
        ..Dt6ReadSnapshot::default()
    }))
}

pub(super) fn read_dt6_led_snapshot(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Dt6ReadSnapshot, SemanticDaliError> {
    let mut a = Dt6ReadSnapshot::default();
    read_dt6_features_and_status(controller, address, content_confirm, &mut a)?;
    read_dt6_protectors_and_modes(controller, address, content_confirm, &mut a)?;
    Ok(a)
}

fn read_dt6_features_and_status(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    a: &mut Dt6ReadSnapshot,
) -> Result<(), SemanticDaliError> {
    let mut query = |cmd| {
        send_extended_query_stable(controller, content_confirm, address, ExtendedCommand::Dt6(cmd))
    };
    a.gear_type = query(Dt6Command::QueryGearType)?;
    a.dimming_curve = query(Dt6Command::QueryDimmingCurve)?;
    a.possible_operating_mode = query(Dt6Command::QueryPossibleOperatingMode)?;
    a.features = query(Dt6Command::QueryFeatures)?;
    a.failure_status = query(Dt6Command::QueryFailureStatus)?;
    let answered = a.failure_status.is_some();
    a.short_circuit = yes_no(answered, query(Dt6Command::QueryShortCircuit)?);
    a.open_circuit = yes_no(answered, query(Dt6Command::QueryOpenCircuit)?);
    a.load_decrease = yes_no(answered, query(Dt6Command::QueryLoadDecrease)?);
    a.load_increase = yes_no(answered, query(Dt6Command::QueryLoadIncrease)?);
    Ok(())
}

fn yes_no(gear_answers_207: bool, answer: Option<u8>) -> Option<u8> {
    if gear_answers_207 {
        answer.or(Some(0))
    } else {
        answer
    }
}

fn read_dt6_protectors_and_modes(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    a: &mut Dt6ReadSnapshot,
) -> Result<(), SemanticDaliError> {
    let mut query = |cmd| {
        send_extended_query_stable(controller, content_confirm, address, ExtendedCommand::Dt6(cmd))
    };
    let answered = a.failure_status.is_some();
    a.current_protector_active = yes_no(answered, query(Dt6Command::QueryCurrentProtectorActive)?);
    a.thermal_shutdown = yes_no(answered, query(Dt6Command::QueryThermalShutdown)?);
    a.thermal_overload = yes_no(answered, query(Dt6Command::QueryThermalOverload)?);
    a.reference_running = yes_no(answered, query(Dt6Command::QueryReferenceRunning)?);
    a.reference_measurement_failed =
        yes_no(answered, query(Dt6Command::QueryReferenceMeasurementFailed)?);
    a.current_protector_enabled = yes_no(answered, query(Dt6Command::QueryCurrentProtectorEnabled)?);
    a.operating_mode = query(Dt6Command::QueryOperatingMode)?;
    a.fast_fade_time = query(Dt6Command::QueryFastFadeTime)?;
    a.min_fast_fade_time = query(Dt6Command::QueryMinFastFadeTime)?;
    a.extended_version_number = query(Dt6Command::QueryExtendedVersionNumber)?;
    Ok(())
}
