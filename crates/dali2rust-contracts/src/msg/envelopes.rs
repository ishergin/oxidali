use serde::{Deserialize, Serialize};

use super::commands::BusCommandPayload;
use super::errors::ErrorPayload;
use super::events::BusEventPayload;
use super::kinds::{ChannelKind, DeliveryStatus, MessageKind, Origin};
use super::wire::DaliConfirmationPayload;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BusEnvelope {
    pub schema_version: u16,
    pub channel_kind: ChannelKind,
    pub message_kind: MessageKind,
    pub sender_id: u16,
    pub correlation_id: u64,
    pub sequence_no: u64,
    pub target_adapter_id: u16,
    pub origin: Origin,
    pub timestamp_ms: u64,
    pub bus_id: u32,
    pub cluster_origin_id: u64,
    pub adapter_proxy_origin_id: u64,
    pub payload_table_id: u16,
    pub error: Option<Box<ErrorPayload>>,
}

impl Default for BusEnvelope {
    fn default() -> Self {
        Self {
            schema_version: 1,
            channel_kind: ChannelKind::Commands,
            message_kind: MessageKind::Command,
            sender_id: 0,
            correlation_id: 0,
            sequence_no: 0,
            target_adapter_id: 0,
            origin: Origin::Internal,
            timestamp_ms: 0,
            bus_id: 0,
            cluster_origin_id: 0,
            adapter_proxy_origin_id: 0,
            payload_table_id: 0,
            error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub meta: BusEnvelope,
    pub payload: BusCommandPayload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmationEnvelope {
    pub meta: BusEnvelope,
    pub status: DeliveryStatus,
    pub confirmation: DaliConfirmationPayload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub meta: BusEnvelope,
    pub payload: BusEventPayload,
}
