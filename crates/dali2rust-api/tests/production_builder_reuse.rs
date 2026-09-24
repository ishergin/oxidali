use dali2rust_api::contracts::dali::{
    DaliCommandRequestBuffer, DaliCommandRequestRead, DaliLevelRequestBuffer,
    DaliRawFrameRequestBuffer,
};
use dali2rust_api::contracts::BufferBytes;
use dali2rust_api::http::handlers::json_command::{
    DaliCommandRequest, LevelRequest, RawFrameRequest,
};

#[test]
fn production_buffer_round_trips_without_hand_built_bytes() {
    let buf = DaliCommandRequestBuffer::new(2, 254, 1);
    let read = DaliCommandRequestRead::from_bytes(buf.as_bytes()).expect("parse request");
    assert_eq!(read.wire_address, 2);
    assert_eq!(read.command, 254);
    assert_eq!(read.repeat_count, 1);
}

#[test]
fn command_builder_output_parses_as_the_handler_dto() {
    let buf = DaliCommandRequestBuffer::new(7, 160, 2);
    let req: DaliCommandRequest =
        serde_json::from_slice(buf.as_bytes()).expect("handler must accept its own shape");
    assert_eq!(
        (req.wire_address, req.command, req.repeat_count),
        (7, 160, 2)
    );
}

#[test]
fn level_builder_output_parses_as_the_handler_dto() {
    let buf = DaliLevelRequestBuffer::new(4, 128);
    let req: LevelRequest =
        serde_json::from_slice(buf.as_bytes()).expect("handler must accept its own shape");
    assert_eq!((req.wire_address, req.level), (4, 128));
}

#[test]
fn raw_builder_output_parses_as_the_handler_dto() {
    let buf = DaliRawFrameRequestBuffer::new(0xFE00, true);
    let req: RawFrameRequest =
        serde_json::from_slice(buf.as_bytes()).expect("handler must accept its own shape");
    assert_eq!((req.frame, req.expects_backward), (0xFE00, true));
}

#[test]
fn repeat_count_defaults_when_the_field_is_absent() {
    let req: DaliCommandRequest =
        serde_json::from_slice(br#"{"wire_address":2,"command":254}"#).expect("parse");
    assert_eq!(req.repeat_count, 1, "serde default must supply repeat_count");
}
