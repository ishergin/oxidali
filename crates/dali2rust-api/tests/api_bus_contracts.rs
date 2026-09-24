use dali2rust_api::bus_codec::parse_command_envelope;
use dali2rust_api::contracts::BufferBytes;
use dali2rust_api::contracts::dali::{DaliCommandRequestBuffer, DaliCommandRequestRead};
use dali2rust_bus::BusId;
use dali2rust_contracts::bus::{encode_command_envelope};

#[test]
fn api_and_bus_repeat_count_match_cont004() {
    let w = 0x12u8;
    let c = 0x34u8;
    let r = 3u8;
    let api_buf = DaliCommandRequestBuffer::new(w, c, r);
    let api_read = DaliCommandRequestRead::from_bytes(api_buf.as_bytes()).expect("parse api");

    let ce = dali2rust_contracts::bus::command_envelope(0, 1, BusId(1).0, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: w, command: c, repeat_count: r, raw_mode: false, raw_expects_backward: false });
    let bus_bytes = encode_command_envelope(&ce).expect("encode");
    let env = parse_command_envelope(&bus_bytes).expect("parse bus");

    assert_eq!(api_read.repeat_count, env.repeat_count);
    assert_eq!(api_read.wire_address, env.wire_address);
    assert_eq!(api_read.command, env.command);
}
