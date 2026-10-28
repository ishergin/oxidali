use dali2rust_contracts::msg::BusHealthVerdict;
use dali2rust_domain::dali::commands::DaliResponse;
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;

use crate::runtime::executor::helpers::{send_standard_response, SemanticDaliError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusHealthProbe {
    pub control_answered: bool,
    pub lamp_failure: BusHealthVerdict,
    pub power_cycle_seen: bool,
}

pub fn probe_bus_health(
    controller: &mut impl DaliApplicationController,
    query_power_failure: bool,
) -> Result<BusHealthProbe, SemanticDaliError> {
    let control = ask_everyone(controller, StandardCommand::QueryControlGearPresent)?;
    let lamp_failure = ask_everyone(controller, StandardCommand::QueryLampFailure)?;
    let power_failure = query_power_failure
        .then(|| ask_everyone(controller, StandardCommand::QueryPowerFailure))
        .transpose()?;
    Ok(BusHealthProbe {
        control_answered: control.is_yes(),
        lamp_failure: verdict_of(lamp_failure),
        power_cycle_seen: power_failure.is_some_and(DaliResponse::is_yes),
    })
}

fn ask_everyone(
    controller: &mut impl DaliApplicationController,
    query: StandardCommand,
) -> Result<DaliResponse, SemanticDaliError> {
    send_standard_response(controller, DaliAddress::Broadcast, query)
}

// IEC 62386-102 §3.13, §3.28
fn verdict_of(response: DaliResponse) -> BusHealthVerdict {
    match response {
        DaliResponse::NoAnswer => BusHealthVerdict::Clear,
        DaliResponse::Answer(_) => BusHealthVerdict::One,
        DaliResponse::Violation => BusHealthVerdict::Several,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_domain::dali::commands::DaliCommand;

    use crate::runtime::clock::StdClock;
    use crate::runtime::controller::DaliController;

    use super::*;

    #[test]
    fn the_three_backward_window_shapes_map_to_the_three_facts() {
        assert_eq!(verdict_of(DaliResponse::NoAnswer), BusHealthVerdict::Clear);
        assert_eq!(verdict_of(DaliResponse::Answer(0xFF)), BusHealthVerdict::One);
        assert_eq!(
            verdict_of(DaliResponse::Violation),
            BusHealthVerdict::Several
        );
    }

    fn broadcast_frame(command: StandardCommand) -> u16 {
        DaliCommand::Standard {
            address: DaliAddress::Broadcast,
            command,
        }
        .to_forward_frame()
        .raw()
    }

    fn run_probe(script: &[(StandardCommand, Option<u8>)], query_power_failure: bool) -> BusHealthProbe {
        let mock = MockDaliTransport::new();
        for (query, answer) in script {
            mock.expect_forward_frame_with_backward(broadcast_frame(*query), *answer);
        }
        let transport = Arc::new(Mutex::new(mock));
        let mut controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
        let probe = probe_bus_health(&mut controller, query_power_failure).expect("probe");
        let guard = transport.lock().expect("transport lock");
        let expected: Vec<u16> = script.iter().map(|(query, _)| broadcast_frame(*query)).collect();
        assert_eq!(guard.sent_frames(), expected, "broadcast frames, control first");
        assert_eq!(guard.scripted_exchanges_remaining(), 0);
        probe
    }

    fn probe_over(control: Option<u8>, failure: Option<u8>) -> BusHealthProbe {
        let script = [
            (StandardCommand::QueryControlGearPresent, control),
            (StandardCommand::QueryLampFailure, failure),
        ];
        run_probe(&script, false)
    }

    fn watched_probe_over(control: Option<u8>, failure: Option<u8>, power_failure: Option<u8>) -> BusHealthProbe {
        let script = [
            (StandardCommand::QueryControlGearPresent, control),
            (StandardCommand::QueryLampFailure, failure),
            (StandardCommand::QueryPowerFailure, power_failure),
        ];
        run_probe(&script, true)
    }

    #[test]
    fn the_probe_is_two_broadcast_frames_with_the_control_first() {
        assert_eq!(
            probe_over(Some(0xFF), None),
            BusHealthProbe {
                control_answered: true,
                lamp_failure: BusHealthVerdict::Clear,
                power_cycle_seen: false,
            }
        );
    }

    #[test]
    fn a_silent_control_voids_a_clear_reading_rather_than_suppressing_it() {
        assert_eq!(
            probe_over(None, None),
            BusHealthProbe {
                control_answered: false,
                lamp_failure: BusHealthVerdict::Clear,
                power_cycle_seen: false,
            }
        );
    }

    #[test]
    fn the_watched_probe_asks_for_power_failures_last() {
        assert_eq!(
            watched_probe_over(Some(0xFF), None, None),
            BusHealthProbe {
                control_answered: true,
                lamp_failure: BusHealthVerdict::Clear,
                power_cycle_seen: false,
            }
        );
    }

    // IEC 62386-102 §11.5.15
    #[test]
    fn any_answer_to_the_power_failure_query_is_a_power_cycle_seen() {
        assert!(watched_probe_over(Some(0xFF), None, Some(0xFF)).power_cycle_seen);
        assert!(
            watched_probe_over(Some(0xFF), None, Some(0x5A)).power_cycle_seen,
            "two gear answering YES at once overlap into a garbled byte, still a YES"
        );
        assert!(
            !watched_probe_over(Some(0xFF), Some(0xFF), None).power_cycle_seen,
            "a lamp failure is not a power cycle"
        );
    }
}
