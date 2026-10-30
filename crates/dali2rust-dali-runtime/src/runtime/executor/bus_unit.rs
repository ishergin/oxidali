use dali2rust_domain::dali::dev103::bus_unit::{TestFrame, TestFrameBits};
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::ses::TransactionPriority;
use dali2rust_domain::dali::DaliApplicationController;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestFrameFailed;

// IEC 62386-103 §11.10.21
pub fn send_test_frame(
    controller: &mut impl DaliApplicationController,
    request: TestFrame,
) -> Result<(), TestFrameFailed> {
    let class = TransactionPriority::from_priority(request.priority()).ok_or(TestFrameFailed)?;
    controller.with_wire_class(Some(class), |c| {
        if request.in_transaction() {
            c.transaction(|c| send_each(c, request))
        } else {
            send_each(c, request)
        }
    })
}

fn send_each(
    controller: &mut impl DaliApplicationController,
    request: TestFrame,
) -> Result<(), TestFrameFailed> {
    (0..=request.repeats()).try_for_each(|_| send_one(controller, request.bits()))
}

fn send_one(
    controller: &mut impl DaliApplicationController,
    bits: TestFrameBits,
) -> Result<(), TestFrameFailed> {
    let sent = match bits {
        TestFrameBits::Forward24(frame) => controller.send_frame24(frame, false).is_ok(),
        TestFrameBits::Forward16([address, opcode]) => {
            controller.send_raw(ForwardFrame::new(address, opcode), false).is_ok()
        }
    };
    if sent {
        Ok(())
    } else {
        Err(TestFrameFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::controller::DaliController;
    use crate::runtime::executor::test_helpers::shared::priorities_of;
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_domain::dali::ses::DaliPriority;
    use dali2rust_test_support::AdvancingClock;
    use std::sync::{Arc, Mutex};

    const CLOCK_STEP_MS: u64 = 20;

    fn send(data: u8, dtr: [u8; 3]) -> Arc<Mutex<MockDaliTransport>> {
        let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
        let mut controller =
            DaliController::new(Arc::clone(&transport), Box::new(AdvancingClock::new(CLOCK_STEP_MS)));
        controller
            .with_wire_class(Some(TransactionPriority::UserAction), |c| {
                send_test_frame(c, TestFrame { data, dtr })
            })
            .expect("a mock takes every plain frame");
        transport
    }

    #[test]
    fn a_24_bit_testframe_goes_at_its_stated_priority_and_repeats_at_it() {
        let transport = send(0b0001_0101, [0x07, 0xFE, 0x10]);
        let guard = transport.lock().unwrap();
        assert_eq!(guard.sent_frames24(), vec![[0x07, 0xFE, 0x10]; 3]);
        assert_eq!(
            priorities_of(&guard.sent_frame24_settle_us()),
            vec![DaliPriority::PeriodicQuery; 3],
            "PPP = 5 outranks the row's class, and T = 0 keeps it on every repeat"
        );
    }

    #[test]
    fn a_testframe_in_a_transaction_repeats_at_priority_one() {
        let transport = send(0b0100_1011, [0x07, 0xFE, 0x10]);
        let guard = transport.lock().unwrap();
        assert_eq!(
            priorities_of(&guard.sent_frame24_settle_us()),
            vec![DaliPriority::Configuration, DaliPriority::Transaction],
            "T = 1 opens at PPP and continues the transaction"
        );
    }

    #[test]
    fn priority_one_is_a_first_frame_too() {
        let transport = send(0b0000_0001, [0x07, 0xFE, 0x10]);
        let guard = transport.lock().unwrap();
        assert_eq!(priorities_of(&guard.sent_frame24_settle_us()), vec![DaliPriority::Transaction]);
    }

    #[test]
    fn a_16_bit_testframe_sends_dtr0_and_dtr1_only() {
        let transport = send(0b0010_1010, [0xA5, 0x5A, 0x99]);
        let guard = transport.lock().unwrap();
        assert!(guard.sent_frames24().is_empty());
        assert_eq!(guard.sent_frames(), vec![0xA55A, 0xA55A]);
        assert_eq!(
            priorities_of(&guard.sent_frame_settle_us()),
            vec![DaliPriority::UserAction; 2]
        );
    }

    #[test]
    fn a_priority_outside_one_to_five_sends_nothing() {
        for data in [0x00, 0x06, 0x07] {
            let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
            let mut controller = DaliController::new(
                Arc::clone(&transport),
                Box::new(AdvancingClock::new(CLOCK_STEP_MS)),
            );
            let sent = send_test_frame(&mut controller, TestFrame { data, dtr: [1, 2, 3] });
            assert_eq!(sent, Err(TestFrameFailed), "{data:#04x}");
            assert!(transport.lock().unwrap().sent_frames24().is_empty());
        }
    }
}
