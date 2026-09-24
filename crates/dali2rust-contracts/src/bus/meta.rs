use crate::msg::{
    BusCommandPayload, BusEnvelope, BusEventPayload, ChannelKind, CommandEnvelope, EventEnvelope,
    MessageKind, Origin,
};

const CURRENT_SCHEMA_VERSION: u16 = 1;

macro_rules! envelope_base {
    ($channel:expr, $message:expr, $sender_id:expr, $correlation_id:expr, $target_adapter_id:expr, $origin:expr) => {
        BusEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            channel_kind: $channel,
            message_kind: $message,
            sender_id: $sender_id,
            correlation_id: $correlation_id,
            sequence_no: 0,
            target_adapter_id: $target_adapter_id,
            origin: $origin.unwrap_or(Origin::Internal),
            timestamp_ms: 0,
            bus_id: 0,
            cluster_origin_id: 0,
            adapter_proxy_origin_id: 0,
            payload_table_id: 0,
            error: None,
        }
    };
}

pub fn envelope_commands(
    sender_id: u16,
    correlation_id: u64,
    target_adapter_id: u16,
    origin: Option<Origin>,
) -> BusEnvelope {
    envelope_base!(
        ChannelKind::Commands,
        MessageKind::Command,
        sender_id,
        correlation_id,
        target_adapter_id,
        origin
    )
}

pub fn envelope_events(
    sender_id: u16,
    correlation_id: u64,
    target_adapter_id: u16,
    origin: Option<Origin>,
) -> BusEnvelope {
    envelope_base!(
        ChannelKind::Events,
        MessageKind::Event,
        sender_id,
        correlation_id,
        target_adapter_id,
        origin
    )
}

pub fn envelope_confirmations(
    sender_id: u16,
    correlation_id: u64,
    origin: Option<Origin>,
) -> BusEnvelope {
    envelope_base!(
        ChannelKind::Confirmations,
        MessageKind::Confirmation,
        sender_id,
        correlation_id,
        0,
        origin
    )
}

pub fn command_envelope(
    sender_id: u16,
    correlation_id: u64,
    target_adapter_id: u16,
    origin: Option<Origin>,
    payload: impl Into<BusCommandPayload>,
) -> CommandEnvelope {
    CommandEnvelope {
        meta: envelope_commands(sender_id, correlation_id, target_adapter_id, origin),
        payload: payload.into(),
    }
}

pub fn event_envelope(
    sender_id: u16,
    correlation_id: u64,
    target_adapter_id: u16,
    origin: Option<Origin>,
    payload: impl Into<BusEventPayload>,
) -> EventEnvelope {
    EventEnvelope {
        meta: envelope_events(sender_id, correlation_id, target_adapter_id, origin),
        payload: payload.into(),
    }
}

#[cfg(test)]
mod generic_envelope_tests {
    use super::*;
    use crate::msg::DaliDiscoverDevicesCommand;

    #[test]
    fn command_envelope_matches_manual_assembly() {
        let body = DaliDiscoverDevicesCommand {
            mode: crate::msg::DiscoveryMode::RefreshKnown,
            registry_adapter_id: 3,
        };
        let generic = command_envelope(7, 42, 1, None, body.clone());
        let manual = CommandEnvelope {
            meta: envelope_commands(7, 42, 1, None),
            payload: BusCommandPayload::DaliDiscoverDevicesCommand(body),
        };
        assert_eq!(generic, manual);
    }

    #[test]
    fn event_envelope_matches_manual_assembly() {
        let body = crate::msg::DaliDiscoveryCompletedEvent {
            registry_adapter_id: 3,
        };
        let generic = event_envelope(7, 42, 1, None, body.clone());
        let manual = EventEnvelope {
            meta: envelope_events(7, 42, 1, None),
            payload: BusEventPayload::DaliDiscoveryCompletedEvent(body),
        };
        assert_eq!(generic, manual);
    }
}
