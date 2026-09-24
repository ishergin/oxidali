use crate::contracts::BufferBytes;
use crate::http::handlers::json_command::{DaliCommandRequest, LevelRequest, RawFrameRequest};
use dali2rust_domain::dali::types::DaliAddress;
use serde::Serialize;

pub fn wire_address_from_short(short_addr: u8) -> Option<u8> {
    DaliAddress::short(short_addr)
        .ok()
        .map(|a| a.encode_address_byte())
}

pub struct DaliCommandRequestBuffer {
    data: Vec<u8>,
}

impl DaliCommandRequestBuffer {
    pub fn new(wire_address: u8, command: u8, repeat_count: u8) -> Self {
        Self {
            data: serialize_request(&DaliCommandRequest {
                wire_address,
                command,
                repeat_count,
            }),
        }
    }
}

impl BufferBytes for DaliCommandRequestBuffer {
    fn inner_data(&self) -> &Vec<u8> {
        &self.data
    }
}

pub struct DaliLevelRequestBuffer {
    data: Vec<u8>,
}

impl DaliLevelRequestBuffer {
    pub fn new(wire_address: u8, level: u8) -> Self {
        Self {
            data: serialize_request(&LevelRequest {
                wire_address,
                level,
            }),
        }
    }
}

impl BufferBytes for DaliLevelRequestBuffer {
    fn inner_data(&self) -> &Vec<u8> {
        &self.data
    }
}

pub struct DaliRawFrameRequestBuffer {
    data: Vec<u8>,
}

impl DaliRawFrameRequestBuffer {
    pub fn new(frame: u16, expects_backward: bool) -> Self {
        Self {
            data: serialize_request(&RawFrameRequest {
                frame,
                expects_backward,
            }),
        }
    }
}

impl BufferBytes for DaliRawFrameRequestBuffer {
    fn inner_data(&self) -> &Vec<u8> {
        &self.data
    }
}

fn serialize_request<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaliCommandRequestRead {
    pub wire_address: u8,
    pub command: u8,
    pub repeat_count: u8,
}

impl DaliCommandRequestRead {
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        let req: DaliCommandRequest = serde_json::from_slice(buf).ok()?;
        Some(Self {
            wire_address: req.wire_address,
            command: req.command,
            repeat_count: if req.repeat_count == 0 {
                1
            } else {
                req.repeat_count
            },
        })
    }
}

pub fn root_as_dali_command_request(
    buf: &[u8],
) -> Result<DaliCommandRequestRead, serde_json::Error> {
    serde_json::from_slice::<DaliCommandRequest>(buf).map(|req| DaliCommandRequestRead {
        wire_address: req.wire_address,
        command: req.command,
        repeat_count: if req.repeat_count == 0 {
            1
        } else {
            req.repeat_count
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_request_roundtrip() {
        let buf = DaliCommandRequestBuffer::new(5, 254, 1);
        let read = DaliCommandRequestRead::from_bytes(buf.as_bytes()).expect("parse");
        assert_eq!(read.wire_address, 5);
        assert_eq!(read.command, 254);
        assert_eq!(read.repeat_count, 1);
    }

    #[test]
    fn command_request_default_repeat() {
        let buf = DaliCommandRequestBuffer::new(0, 0, 1);
        let read = DaliCommandRequestRead::from_bytes(buf.as_bytes()).expect("parse");
        assert_eq!(read.repeat_count, 1);
    }

    #[test]
    fn command_request_parse_empty_fails() {
        assert!(DaliCommandRequestRead::from_bytes(&[]).is_none());
    }
}
