use std::sync::Arc;

use crate::BusId;

use dali2rust_contracts::bus::{
    decode_command_envelope, decode_confirmation_envelope, decode_event_envelope,
    encoded_len_command, encoded_len_confirmation, encoded_len_event, MAX_BUS_WIRE_BYTES,
};
use dali2rust_contracts::msg::{
    ChannelKind, CommandEnvelope, ConfirmationEnvelope, EventEnvelope, MessageKind,
};

pub const MAX_BUS_FRAME_BYTES: usize = MAX_BUS_WIRE_BYTES;

#[derive(Clone, Debug)]
pub enum BusFrame {
    Command(Arc<CommandEnvelope>),
    Confirmation(Arc<ConfirmationEnvelope>),
    Event(Arc<EventEnvelope>),
}

impl BusFrame {
    pub fn command(env: CommandEnvelope) -> Self {
        Self::Command(Arc::new(env))
    }

    pub fn confirmation(env: ConfirmationEnvelope) -> Self {
        Self::Confirmation(Arc::new(env))
    }

    pub fn event(env: EventEnvelope) -> Self {
        Self::Event(Arc::new(env))
    }

    pub fn event_for(&self, bus_id: BusId) -> Option<&EventEnvelope> {
        match self {
            Self::Event(ev) if BusId(ev.meta.target_adapter_id) == bus_id => Some(ev.as_ref()),
            _ => None,
        }
    }

    pub fn command_for(&self, bus_id: BusId) -> Option<&CommandEnvelope> {
        match self {
            Self::Command(cmd) if BusId(cmd.meta.target_adapter_id) == bus_id => Some(cmd.as_ref()),
            _ => None,
        }
    }

    pub fn from_slice(data: &[u8]) -> Option<Self> {
        if let Ok(ce) = decode_command_envelope(data) {
            if ce.meta.channel_kind == ChannelKind::Commands
                && ce.meta.message_kind == MessageKind::Command
            {
                return Some(Self::Command(Arc::new(ce)));
            }
        }
        if let Ok(ce) = decode_confirmation_envelope(data) {
            if ce.meta.channel_kind == ChannelKind::Confirmations
                && ce.meta.message_kind == MessageKind::Confirmation
            {
                return Some(Self::Confirmation(Arc::new(ce)));
            }
        }
        if let Ok(ev) = decode_event_envelope(data) {
            if ev.meta.channel_kind == ChannelKind::Events
                && ev.meta.message_kind == MessageKind::Event
            {
                return Some(Self::Event(Arc::new(ev)));
            }
        }
        None
    }

    pub fn postcard_wire_len(&self) -> Option<usize> {
        match self {
            Self::Command(c) => encoded_len_command(c.as_ref()).ok(),
            Self::Confirmation(c) => encoded_len_confirmation(c.as_ref()).ok(),
            Self::Event(e) => encoded_len_event(e.as_ref()).ok(),
        }
    }
}

#[cfg(test)]
mod from_slice_tests {
    use super::BusFrame;
    use dali2rust_contracts::bus::{encode_event_envelope};

    #[test]
    fn from_slice_dali_attributes_written_decodes_as_event_not_command() {
        let ev = dali2rust_contracts::bus::event_envelope(0, 901, 1, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesWrittenEvent { short_address: 17, fade_time_ms: Some(500), fade_rate: None, power_on_level: None, system_failure_level: None, extended_fade_time_ms: None, registry_adapter_id: 0 , tc_coolest_mirek: None, tc_warmest_mirek: None, min_level: None, max_level: None, dimming_curve: None });
        let bytes = encode_event_envelope(&ev).expect("encode");
        let frame = BusFrame::from_slice(&bytes).expect("from_slice");
        let BusFrame::Event(arc) = frame else {
            panic!("expected event frame, got {frame:?}");
        };
        assert!(matches!(
            &arc.payload,
            dali2rust_contracts::msg::BusEventPayload::DaliAttributesWrittenEvent(b)
                if b.short_address == 17 && b.fade_time_ms == Some(500)
        ));
    }
}
