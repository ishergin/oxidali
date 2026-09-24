use crate::bus::postcard_codec;
use crate::msg::{BusCommandPayload, ChannelKind, CommandEnvelope, MessageKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedDaliCommandEnvelope {
    pub correlation_id: u64,
    pub sender_id: u16,
    pub wire_address: u8,
    pub command: u8,
    pub repeat_count: u8,
    pub target_adapter_id: u16,
    pub raw_mode: bool,
    pub raw_expects_backward: bool,
}

pub fn parse_command_wire(cmd: &CommandEnvelope) -> Option<ParsedDaliCommandEnvelope> {
    let BusCommandPayload::DaliCommandPayload(pl) = &cmd.payload else {
        return None;
    };
    let meta = &cmd.meta;
    Some(ParsedDaliCommandEnvelope {
        correlation_id: meta.correlation_id,
        sender_id: meta.sender_id,
        wire_address: pl.wire_address,
        command: pl.command,
        repeat_count: pl.repeat_count,
        target_adapter_id: meta.target_adapter_id,
        raw_mode: pl.raw_mode,
        raw_expects_backward: pl.raw_expects_backward,
    })
}

pub fn parse_command_envelope(data: &[u8]) -> Option<ParsedDaliCommandEnvelope> {
    let ce: CommandEnvelope = postcard_codec::decode_command_envelope(data).ok()?;
    if ce.meta.channel_kind != ChannelKind::Commands || ce.meta.message_kind != MessageKind::Command
    {
        return None;
    }
    parse_command_wire(&ce)
}
