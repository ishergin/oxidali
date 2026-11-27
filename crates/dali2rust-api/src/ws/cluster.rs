use crate::http::cluster_status_state::{write_cluster_status, ClusterStatusHttpState};
use crate::ws::project::WS_PROJECTED_EVENTS;
use crate::ws::protocol::{event_frame_with_payload, Channel};

pub const CLUSTER_STATUS_FRAME_CEILING_BYTES: usize = 16_384;

const CLUSTER_STATUS_FIRST_RESERVE_BYTES: usize = 1_024;

pub const WS_PORT_EVENTS: &[&str] = &["ClusterStateChangedEvent"];

pub const WS_SUBSCRIBED_EVENTS: &[&str] =
    &joined::<{ WS_PROJECTED_EVENTS.len() + WS_PORT_EVENTS.len() }>(WS_PROJECTED_EVENTS, WS_PORT_EVENTS);

const fn joined<const N: usize>(head: &[&'static str], tail: &[&'static str]) -> [&'static str; N] {
    let mut out = [""; N];
    let mut at = 0;
    while at < head.len() {
        out[at] = head[at];
        at += 1;
    }
    let mut next = 0;
    while next < tail.len() {
        out[at + next] = tail[next];
        next += 1;
    }
    out
}

pub fn cluster_status_frame(state: &dyn ClusterStatusHttpState, ts_ms: u64) -> String {
    let mut payload = Vec::with_capacity(CLUSTER_STATUS_FIRST_RESERVE_BYTES);
    let written = write_cluster_status(state, &mut payload).ok().and_then(|()| String::from_utf8(payload).ok());
    event_frame_with_payload("ClusterStatus", Channel::Cluster, ts_ms, written.as_deref().unwrap_or("null"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::cluster_status_state::{
        ClusterIgnoredDto, ClusterNodeDto, ClusterOwnDto, ClusterWireOwnerDto, MAX_CLUSTER_STATUS_IGNORED,
        MAX_CLUSTER_STATUS_NODES,
    };

    const ESCAPED_EVERY_BYTE: char = '\u{1}';
    const INSTALLATION_BYTES: usize = 32;
    const CLUSTER_NAME_BYTES: usize = 64;
    const WIDEST_BOARD: &str = "dali-ffffff";
    const WIDEST_ADDRESS: &str = "255.255.255.255";

    fn hostile(bytes: usize) -> String {
        core::iter::repeat_n(ESCAPED_EVERY_BYTE, bytes).collect()
    }

    struct WorstCluster;

    impl ClusterStatusHttpState for WorstCluster {
        fn own(&self) -> ClusterOwnDto {
            ClusterOwnDto {
                state: "bind_failed",
                node_number: u8::MAX,
                wire: u8::MAX,
                board: Some(WIDEST_BOARD.into()),
                boot_epoch: u32::MAX,
                wire_owner: Some(ClusterWireOwnerDto { node: u8::MAX, board: WIDEST_BOARD.into(), this_node: false }),
                split_pair: true,
                wire_chain: true,
                number_held_by: Some(WIDEST_BOARD.into()),
            }
        }

        fn node(&self, node: u8) -> Option<ClusterNodeDto> {
            Some(ClusterNodeDto {
                node,
                board: WIDEST_BOARD.into(),
                address: WIDEST_ADDRESS.into(),
                installation: hostile(INSTALLATION_BYTES),
                wire: u8::MAX,
                wire_protection: false,
                home_assistant_face: Some(false),
                heard: false,
                age_ms: u32::MAX,
                sequence_gaps: u32::MAX,
                split: false,
            })
        }

        fn ignored(&self, _index: usize) -> Option<ClusterIgnoredDto> {
            Some(ClusterIgnoredDto {
                board: WIDEST_BOARD.into(),
                address: WIDEST_ADDRESS.into(),
                node: u8::MAX,
                reason: "other_cluster",
                cluster_name: Some(hostile(CLUSTER_NAME_BYTES)),
                age_ms: u32::MAX,
            })
        }
    }

    #[test]
    fn the_cluster_status_frame_worst_case_is_measured_and_frozen() {
        let frame = cluster_status_frame(&WorstCluster, u64::MAX);
        eprintln!(
            "worst-case ClusterStatus: {} B of {} B ceiling",
            frame.len(),
            CLUSTER_STATUS_FRAME_CEILING_BYTES
        );
        assert!(frame.len() <= CLUSTER_STATUS_FRAME_CEILING_BYTES);
        let parsed: serde_json::Value = serde_json::from_str(&frame).expect("one JSON frame");
        let payload = &parsed["payload"];
        assert_eq!(payload["nodes"].as_array().map(Vec::len), Some(usize::from(MAX_CLUSTER_STATUS_NODES)));
        assert_eq!(payload["ignored"].as_array().map(Vec::len), Some(MAX_CLUSTER_STATUS_IGNORED));
        assert_eq!(parsed["channel"], "cluster");
    }

    #[test]
    fn the_hub_subscribes_to_every_projected_event_and_the_port_built_ones() {
        assert_eq!(WS_SUBSCRIBED_EVENTS.len(), WS_PROJECTED_EVENTS.len() + WS_PORT_EVENTS.len());
        assert!(WS_PROJECTED_EVENTS.iter().chain(WS_PORT_EVENTS).all(|kind| WS_SUBSCRIBED_EVENTS.contains(kind)));
        assert!(WS_PORT_EVENTS.iter().all(|kind| !WS_PROJECTED_EVENTS.contains(kind)));
    }
}
