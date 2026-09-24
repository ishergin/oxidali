use cucumber::then;
use dali2rust_domain::dali::commands::{DaliCommand, StandardCommand};
use dali2rust_domain::dali::types::DaliAddress;

use crate::DaliWorld;

// CONT-001
#[then(
    regex = r"the mock should have sent forward frame for short address (\d+) direct arc level (\d+)"
)]
async fn then_mock_sent_forward_frame(world: &mut DaliWorld, short_addr: u64, level: u64) {
    let addr = DaliAddress::short(short_addr as u8).expect("short address in range");
    let cmd = DaliCommand::Standard {
        address: addr,
        command: StandardCommand::DirectArcPower { level: level as u8 },
    };
    let expected = cmd.to_forward_frame().raw();
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    let last = *frames
        .last()
        .expect("mock should have recorded a forward frame");
    assert_eq!(
        last, expected,
        "expected forward frame raw 0x{:04X}, got 0x{:04X}",
        expected, last
    );
}
