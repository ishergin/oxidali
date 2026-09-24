use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct RedundancyTransitionDto {
    pub now_active: bool,
    pub reason: &'static str,
    pub detected_at_ms: u32,
    pub completed_at_ms: u32,
    pub took_ms: u32,
    pub last_peer_answer_ms: u32,
    pub missed_probes: u8,
}

#[must_use]
pub fn transition_reason_name(code: u8) -> &'static str {
    match code {
        0 => "peer_silent",
        1 => "peer_answered",
        2 => "manual",
        3 => "handover",
        _ => "boot",
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RedundancyProbeDto {
    pub published: u32,
    pub ingress_rejected: u32,
    pub owned: u32,
    pub unowned: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RedundancyReplicationDto {
    pub passes: u32,
    pub peer_unreachable: u32,
    pub pulled: u32,
    pub rejected: u32,
    pub reload_publish_failed: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct RedundancyStateDto {
    pub enabled: bool,
    pub role: &'static str,
    pub active: bool,
    pub answering: bool,
    pub lease_remaining_ms: u32,
    pub probes: RedundancyProbeDto,
    pub takeovers: u32,
    pub stand_downs: u32,
    pub role_publish_failed: u32,
    pub ignored_events: u32,
    pub replication: RedundancyReplicationDto,
    pub transitions: Vec<RedundancyTransitionDto>,
}

pub trait RedundancyHttpState: Send + Sync {
    fn redundancy_state_dto(&self) -> RedundancyStateDto;
}
