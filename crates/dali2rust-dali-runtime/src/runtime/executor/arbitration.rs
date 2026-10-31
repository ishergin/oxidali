use dali2rust_domain::dali::dev103::address::{Device103Address, InstanceAddress};
use dali2rust_domain::dali::dev103::command::Device103Command;
use dali2rust_domain::dali::dev103::frame::ForwardFrame24;
use dali2rust_domain::dali::pres::DaliResponse;
use dali2rust_domain::dali::DaliApplicationController;

use super::dev103::{send, send_once};
use super::SemanticDaliError;

#[must_use]
// DiiA 351 §7
pub fn read_verdict(response: DaliResponse) -> bool {
    match response {
        DaliResponse::NoAnswer => false,
        _ => response.is_yes() || response.value().is_some(),
    }
}

pub fn probe_application_controller(
    controller: &mut impl DaliApplicationController,
) -> Result<bool, SemanticDaliError> {
    let frame = ForwardFrame24::command(
        Device103Address::Broadcast,
        InstanceAddress::Device,
        Device103Command::QueryApplicationControlEnabled
            .metadata()
            .opcode,
    );
    let response = send_once(controller, frame, true)?;
    Ok(read_verdict(response))
}

// IEC 62386-103 §11.6.16
pub fn enable_peer_controller(
    controller: &mut impl DaliApplicationController,
    peer: u8,
) -> Result<(), SemanticDaliError> {
    let Some(address) = short_address(peer) else {
        return Err(SemanticDaliError::Conflict(PEER_ADDRESS_INVALID));
    };
    controller.transaction(|c| {
        super::dev103::send_twice(c, Device103Command::EnableApplicationController.frame(address))?;
        let answer = send(c, Device103Command::QueryApplicationControlEnabled.frame(address), true)?;
        peer_enabled(answer)
    })
}

// IEC 62386-101 §8.2.5
fn peer_enabled(answer: DaliResponse) -> Result<(), SemanticDaliError> {
    match answer {
        DaliResponse::Answer(_) => Ok(()),
        DaliResponse::Violation => Err(SemanticDaliError::Conflict(PEER_ANSWER_CONTENDED)),
        DaliResponse::NoAnswer => Err(SemanticDaliError::Conflict(PEER_NOT_ENABLED)),
    }
}

const PEER_ADDRESS_INVALID: &str = "peer_address_invalid";
const PEER_NOT_ENABLED: &str = "peer_not_enabled";
const PEER_ANSWER_CONTENDED: &str = "peer_answer_contended";

fn short_address(value: u8) -> Option<Device103Address> {
    (value <= dali2rust_domain::dali::dev103::address::MAX_SHORT_ADDRESS)
        .then_some(Device103Address::Short(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_out_of_range_peer_address_is_refused_rather_than_clamped() {
        assert!(short_address(64).is_none());
        assert!(short_address(63).is_some());
    }

    #[test]
    fn silence_means_the_bus_is_unowned() {
        assert!(!read_verdict(DaliResponse::NoAnswer));
    }

    #[test]
    fn a_plain_answer_means_owned() {
        assert!(read_verdict(DaliResponse::Answer(0xFF)));
        assert!(read_verdict(DaliResponse::Answer(0x00)));
    }

    #[test]
    fn a_violating_frame_means_owned_because_351_says_so() {
        assert!(read_verdict(DaliResponse::Violation));
    }
}
