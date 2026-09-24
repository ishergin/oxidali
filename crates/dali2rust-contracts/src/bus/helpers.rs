use crate::msg::{RuntimeObservation, RuntimeSource};

pub fn runtime_observation_timestamped(
    source: RuntimeSource,
    last_seen_ms: u64,
) -> RuntimeObservation {
    RuntimeObservation::timestamped(source, last_seen_ms)
}

pub fn runtime_observation_api_timestamped(last_seen_ms: u64) -> RuntimeObservation {
    RuntimeObservation::api_timestamped(last_seen_ms)
}

pub fn runtime_observation_sniffer_timestamped(last_seen_ms: u64) -> RuntimeObservation {
    RuntimeObservation::sniffer_timestamped(last_seen_ms)
}
