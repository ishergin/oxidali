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
}

pub fn probe_bus_health(
    controller: &mut impl DaliApplicationController,
) -> Result<BusHealthProbe, SemanticDaliError> {
    let control = send_standard_response(
        controller,
        DaliAddress::Broadcast,
        StandardCommand::QueryControlGearPresent,
    )?;
    let lamp_failure = send_standard_response(
        controller,
        DaliAddress::Broadcast,
        StandardCommand::QueryLampFailure,
    )?;
    Ok(BusHealthProbe {
        control_answered: control.is_yes(),
        lamp_failure: verdict_of(lamp_failure),
    })
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

    fn probe_over(control: Option<u8>, failure: Option<u8>) -> BusHealthProbe {
        let mock = MockDaliTransport::new();
        mock.expect_forward_frame_with_backward(
            broadcast_frame(StandardCommand::QueryControlGearPresent),
            control,
        );
        mock.expect_forward_frame_with_backward(
            broadcast_frame(StandardCommand::QueryLampFailure),
            failure,
        );
        let transport = Arc::new(Mutex::new(mock));
        let mut controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
        let probe = probe_bus_health(&mut controller).expect("probe");
        let guard = transport.lock().expect("transport lock");
        assert_eq!(
            guard.sent_frames(),
            vec![
                broadcast_frame(StandardCommand::QueryControlGearPresent),
                broadcast_frame(StandardCommand::QueryLampFailure),
            ],
            "two broadcast frames, control first"
        );
        assert_eq!(guard.scripted_exchanges_remaining(), 0);
        probe
    }

    #[test]
    fn the_probe_is_two_broadcast_frames_with_the_control_first() {
        assert_eq!(
            probe_over(Some(0xFF), None),
            BusHealthProbe {
                control_answered: true,
                lamp_failure: BusHealthVerdict::Clear,
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
            }
        );
    }
}
