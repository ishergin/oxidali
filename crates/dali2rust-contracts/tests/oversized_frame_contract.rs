use dali2rust_contracts::bus::{command_envelope, encode_command_envelope, MAX_BUS_WIRE_BYTES};
use dali2rust_contracts::msg::DaliCommandPayload;

#[test]
fn minimal_command_envelope_fits_bus_wire_budget() {
    let ce = command_envelope(
        0,
        1,
        1,
        None,
        DaliCommandPayload {
            wire_address: 1,
            command: 2,
            repeat_count: 1,
            raw_mode: false,
            raw_expects_backward: false,
        },
    );
    let wire = encode_command_envelope(&ce).expect("encode");
    assert!(wire.len() <= MAX_BUS_WIRE_BYTES);
}
