use std::io;

use crate::http::cluster_info_state::{write_cluster_info, ClusterInfoHttpState};
use crate::http::cluster_lamps_state::{write_cluster_lamps, ClusterLampsHttpState};
use crate::http::cluster_inputs_state::{write_cluster_inputs, ClusterInputsHttpState};
use crate::http::cluster_status_state::{write_cluster_status, ClusterStatusHttpState};
use crate::ws::project::WS_PROJECTED_EVENTS;
use crate::ws::protocol::{event_frame_with_payload, Channel};

pub const CLUSTER_STATUS_FRAME_CEILING_BYTES: usize = 16_384;

pub const CLUSTER_INFO_FRAME_CEILING_BYTES: usize = 2_048;

pub const CLUSTER_LAMPS_FRAME_CEILING_BYTES: usize = 1_216;
pub const CLUSTER_INPUTS_FRAME_CEILING_BYTES: usize = 768;

const CLUSTER_FRAME_FIRST_RESERVE_BYTES: usize = 1_024;

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
    cluster_frame("ClusterStatus", ts_ms, |payload| write_cluster_status(state, payload))
}

pub fn cluster_info_frame(state: &dyn ClusterInfoHttpState, ts_ms: u64) -> String {
    cluster_frame("ClusterInfo", ts_ms, |payload| write_cluster_info(state, payload))
}

pub fn cluster_lamps_frame(state: &dyn ClusterLampsHttpState, ts_ms: u64) -> String {
    cluster_frame("ClusterLamps", ts_ms, |payload| write_cluster_lamps(state, payload))
}

pub fn cluster_inputs_frame(state: &dyn ClusterInputsHttpState, ts_ms: u64) -> String {
    cluster_frame("ClusterInputs", ts_ms, |payload| write_cluster_inputs(state, payload))
}

fn cluster_frame(event: &str, ts_ms: u64, write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
    let mut payload = Vec::with_capacity(CLUSTER_FRAME_FIRST_RESERVE_BYTES);
    let written = write(&mut payload).ok().and_then(|()| String::from_utf8(payload).ok());
    event_frame_with_payload(event, Channel::Cluster, ts_ms, written.as_deref().unwrap_or("null"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::cluster_info_state::{ClusterInfoNodeDto, ClusterInfoSource};
    use crate::http::cluster_lamps_state::{ClusterLampDto, ClusterLampsNodeDto};
    use dali2rust_contracts::msg::ClusterRecordRef;
    use dali2rust_domain::registry::ChannelPartLit;
    use crate::http::cluster_inputs_state::{
        ClusterInputDeviceDto, ClusterInputInstanceDto, ClusterInputOwnerDto, ClusterInputsNodeDto,
    };
    use crate::http::cluster_parts_state::ClusterPartDto;
    use crate::http::cluster_records_state::ClusterRecordDto;
    use dali2rust_contracts::msg::ClusterRecordKind;
    use dali2rust_domain::registry::{ClusterPartKeys, ClusterRecordNumbers, CLUSTER_NODE_COUNT};
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
                boot_epoch: Some(u32::MAX),
                wire_owner: Some(ClusterWireOwnerDto { node: u8::MAX, board: WIDEST_BOARD.into(), this_node: false }),
                split_pair: true,
                wire_chain: true,
                unprotected_wire: true,
                number_held_by: Some(WIDEST_BOARD.into()),
                block: "contested",
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

        fn neighbour_names_wire(&self, _node: u8, _wire: u8) -> bool {
            true
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

    struct WorstInfo;

    impl ClusterInfoHttpState for WorstInfo {
        fn node_info(&self, node: u8) -> Option<ClusterInfoNodeDto> {
            Some(ClusterInfoNodeDto {
                node,
                source: ClusterInfoSource::Cached,
                set: Some(u32::MAX),
                age_ms: Some(u32::MAX),
                channels: u8::MAX,
                scenes: u8::MAX,
                channel_parts: u8::MAX,
                scene_parts: u8::MAX,
                skipped: u16::MAX,
            })
        }

        fn record(&self, _node: u8, _kind: ClusterRecordKind, _number: u8) -> Option<ClusterRecordDto> {
            None
        }

        fn record_numbers(&self, _node: u8, _kind: ClusterRecordKind) -> ClusterRecordNumbers {
            ClusterRecordNumbers::default()
        }

        fn part(&self, _node: u8, _kind: ClusterRecordKind, _adapter: u8, _local: u8) -> Option<ClusterPartDto> {
            None
        }

        fn part_keys(&self, _node: u8, _kind: ClusterRecordKind) -> ClusterPartKeys {
            ClusterPartKeys::default()
        }

        fn record_write_block(&self) -> Result<u8, &'static str> {
            Ok(0)
        }
    }

    #[test]
    fn the_cluster_info_frame_worst_case_is_measured_and_frozen() {
        let frame = cluster_info_frame(&WorstInfo, u64::MAX);
        eprintln!("worst-case ClusterInfo: {} B of {} B ceiling", frame.len(), CLUSTER_INFO_FRAME_CEILING_BYTES);
        assert!(frame.len() <= CLUSTER_INFO_FRAME_CEILING_BYTES);
        let parsed: serde_json::Value = serde_json::from_str(&frame).expect("one JSON frame");
        assert_eq!(parsed["payload"]["nodes"].as_array().map(Vec::len), Some(usize::from(CLUSTER_NODE_COUNT)));
        assert_eq!((parsed["type"].as_str(), parsed["channel"].as_str()), (Some("ClusterInfo"), Some("cluster")));
    }

    struct WorstInputs;

    impl ClusterInputsHttpState for WorstInputs {
        fn inputs_node(&self, node: u8) -> Option<ClusterInputsNodeDto> {
            Some(ClusterInputsNodeDto {
                node,
                source: ClusterInfoSource::Stale,
                set: Some(u32::MAX),
                age_ms: Some(u32::MAX),
                device_count: u8::MAX,
            })
        }

        fn input_owner(&self, _node: u8) -> Option<ClusterInputOwnerDto> {
            None
        }

        fn input_device(&self, _node: u8, _index: usize) -> Option<ClusterInputDeviceDto> {
            None
        }

        fn input_instance(&self, _node: u8, _device_index: usize, _index: usize) -> Option<ClusterInputInstanceDto> {
            None
        }
    }

    struct WorstLamps;

    impl ClusterLampsHttpState for WorstLamps {
        fn lamps_node(&self, node: u8) -> Option<ClusterLampsNodeDto> {
            Some(ClusterLampsNodeDto {
                node,
                source: ClusterInfoSource::Cached,
                lamps: u8::MAX,
                state_set: Some(u32::MAX),
                info_set: Some(u32::MAX),
                age_ms: Some(u32::MAX),
                sequence_gaps: Some(u32::MAX),
            })
        }

        fn lamp_ids(&self, _node: u8) -> u64 {
            0
        }

        fn lamp(&self, _node: u8, _lamp: u8) -> Option<ClusterLampDto> {
            None
        }

        fn part_lit(&self, _node: u8, _adapter: u8, _group: u8, _channel: ClusterRecordRef) -> ChannelPartLit {
            ChannelPartLit::UNHEARD
        }
    }

    #[test]
    fn the_cluster_inputs_frame_worst_case_is_measured_and_frozen() {
        let frame = cluster_inputs_frame(&WorstInputs, u64::MAX);
        eprintln!("worst-case ClusterInputs: {} B of {} B ceiling", frame.len(), CLUSTER_INPUTS_FRAME_CEILING_BYTES);
        assert!(frame.len() <= CLUSTER_INPUTS_FRAME_CEILING_BYTES);
        let parsed: serde_json::Value = serde_json::from_str(&frame).expect("one JSON frame");
        assert_eq!(parsed["payload"]["nodes"].as_array().map(Vec::len), Some(usize::from(CLUSTER_NODE_COUNT)));
        assert_eq!((parsed["type"].as_str(), parsed["channel"].as_str()), (Some("ClusterInputs"), Some("cluster")));
    }

    #[test]
    fn the_cluster_lamps_frame_worst_case_is_measured_and_frozen() {
        let frame = cluster_lamps_frame(&WorstLamps, u64::MAX);
        eprintln!("worst-case ClusterLamps: {} B of {} B ceiling", frame.len(), CLUSTER_LAMPS_FRAME_CEILING_BYTES);
        assert!(frame.len() <= CLUSTER_LAMPS_FRAME_CEILING_BYTES);
        let parsed: serde_json::Value = serde_json::from_str(&frame).expect("one JSON frame");
        assert_eq!(parsed["payload"]["nodes"].as_array().map(Vec::len), Some(usize::from(CLUSTER_NODE_COUNT)));
        assert_eq!((parsed["type"].as_str(), parsed["channel"].as_str()), (Some("ClusterLamps"), Some("cluster")));
    }

    #[test]
    fn the_hub_subscribes_to_every_projected_event_and_the_port_built_ones() {
        assert_eq!(WS_SUBSCRIBED_EVENTS.len(), WS_PROJECTED_EVENTS.len() + WS_PORT_EVENTS.len());
        assert!(WS_PROJECTED_EVENTS.iter().chain(WS_PORT_EVENTS).all(|kind| WS_SUBSCRIBED_EVENTS.contains(kind)));
        assert!(WS_PORT_EVENTS.iter().all(|kind| !WS_PROJECTED_EVENTS.contains(kind)));
    }
}
