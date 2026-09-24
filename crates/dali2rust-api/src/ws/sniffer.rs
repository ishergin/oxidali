use dali2rust_domain::dali::pres::{
    describe_backward8, describe_forward16, describe_forward24, DescribeContext, FrameDescription,
    FrameTarget, ENABLE_DEVICE_TYPE_ADDRESS,
};
use dali2rust_platform::dali::{FrameDirection, ObservedRawFrameKind, SnifferRecord};
use serde_json::{json, Value};

use crate::ws::batch::batch_frame;
use crate::ws::protocol::Channel;

#[derive(Debug, Default)]
pub struct SnifferDecoderState {
    tx_device_type: Option<u8>,
    rx_device_type: Option<u8>,
}

impl SnifferDecoderState {
    fn take_context(&mut self, record: &SnifferRecord) -> DescribeContext {
        if record.kind != ObservedRawFrameKind::Forward16 {
            return DescribeContext::default();
        }
        let slot = match record.direction {
            FrameDirection::Tx => &mut self.tx_device_type,
            FrameDirection::ForeignRx | FrameDirection::Reply => &mut self.rx_device_type,
        };
        if record.bytes[0] == ENABLE_DEVICE_TYPE_ADDRESS {
            *slot = Some(record.bytes[1]);
            return DescribeContext::default();
        }
        DescribeContext {
            enabled_device_type: slot.take(),
        }
    }

    pub fn describe(&mut self, record: &SnifferRecord) -> FrameDescription {
        let ctx = self.take_context(record);
        match record.kind {
            ObservedRawFrameKind::Forward16 => {
                describe_forward16(record.bytes[0], record.bytes[1], ctx)
            }
            ObservedRawFrameKind::Forward24 => describe_forward24(record.bytes),
            ObservedRawFrameKind::Backward8 => describe_backward8(record.bytes[0]),
        }
    }
}

const fn direction_name(direction: FrameDirection) -> &'static str {
    match direction {
        FrameDirection::Tx => "tx",
        FrameDirection::ForeignRx => "rx",
        FrameDirection::Reply => "reply",
    }
}

const fn width_name(kind: ObservedRawFrameKind) -> &'static str {
    match kind {
        ObservedRawFrameKind::Forward16 => "forward16",
        ObservedRawFrameKind::Forward24 => "forward24",
        ObservedRawFrameKind::Backward8 => "backward8",
    }
}

fn hex(record: &SnifferRecord) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let used: usize = match record.kind {
        ObservedRawFrameKind::Backward8 => 1,
        ObservedRawFrameKind::Forward16 => 2,
        ObservedRawFrameKind::Forward24 => 3,
    };
    let mut out = String::with_capacity(used * 3 - 1);
    for (index, byte) in record.bytes[..used].iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0F)] as char);
    }
    out
}

fn target_value(target: Option<FrameTarget>) -> Value {
    match target {
        Some(FrameTarget::Short(a)) => json!({ "kind": "short", "id": a }),
        Some(FrameTarget::Group(g)) => json!({ "kind": "group", "id": g }),
        Some(FrameTarget::Broadcast) => json!({ "kind": "broadcast" }),
        Some(FrameTarget::BroadcastUnaddressed) => json!({ "kind": "broadcast_unaddressed" }),
        None => Value::Null,
    }
}

fn record_value(record: &SnifferRecord, description: &FrameDescription) -> Value {
    json!({
        "ts_ms": record.at_ms,
        "dir": direction_name(record.direction),
        "width": width_name(record.kind),
        "adapter_id": record.adapter_id,
        "attempt": record.attempt,
        "hex": hex(record),
        "target": target_value(description.target),
        "name": description.name,
        "detail": description.detail,
        "is_query": description.is_query,
    })
}

pub fn sniffer_batch_frame(
    state: &mut SnifferDecoderState,
    records: &[SnifferRecord],
    dropped_since: u32,
    ts_ms: u64,
) -> (String, u32) {
    let frames: Vec<Value> = records
        .iter()
        .map(|record| {
            let description = state.describe(record);
            record_value(record, &description)
        })
        .collect();
    batch_frame(
        "SnifferBatch",
        Channel::Sniffer,
        "frames",
        frames,
        dropped_since,
        ts_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::batch::MAX_BATCH_PAYLOAD_BYTES;

    #[test]
    fn a_batch_is_shed_to_the_byte_budget_and_says_what_it_shed() {
        let mut state = SnifferDecoderState::default();
        let records: Vec<SnifferRecord> = (0..64)
            .map(|i| forward16([0x00, i as u8], FrameDirection::Tx))
            .collect();
        let (frame, shed) = sniffer_batch_frame(&mut state, &records, 0, 9_000);
        let v: Value = serde_json::from_str(&frame).unwrap();
        let kept = v["payload"]["frames"].as_array().unwrap().len();
        assert!(kept < records.len(), "nothing was shed from {} records", records.len());
        assert_eq!(
            v["payload"]["dropped_since"].as_u64().unwrap(),
            (records.len() - kept) as u64,
            "what the cap sheds must be counted, not hidden"
        );
        assert_eq!(
            shed as usize,
            records.len() - kept,
            "the caller counts the shed into the diagnostics counter ((a) LOW tail, 2026-08-21 audit), so the wire and the return must agree"
        );
        let payload_len: usize = v["payload"]["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.to_string().len() + 1)
            .sum();
        assert!(
            payload_len <= MAX_BATCH_PAYLOAD_BYTES,
            "payload {payload_len} B is over the {MAX_BATCH_PAYLOAD_BYTES} B budget"
        );
    }

    #[test]
    fn shedding_keeps_the_newest_records() {
        let mut state = SnifferDecoderState::default();
        let records: Vec<SnifferRecord> = (0..64)
            .map(|i| forward16([0x00, i as u8], FrameDirection::Tx))
            .collect();
        let last = *records.last().unwrap();
        let newest = record_value(&last, &SnifferDecoderState::default().describe(&last));
        let (frame, _shed) = sniffer_batch_frame(&mut state, &records, 0, 9_000);
        let v: Value = serde_json::from_str(&frame).unwrap();
        let frames = v["payload"]["frames"].as_array().unwrap();
        assert_eq!(
            frames.last().unwrap()["hex"],
            newest["hex"],
            "the last record in must be the last record out"
        );
    }

    #[test]
    fn a_small_batch_is_not_shed() {
        let mut state = SnifferDecoderState::default();
        let (frame, shed) = sniffer_batch_frame(
            &mut state,
            &[forward16([0x00, 0x78], FrameDirection::Tx)],
            3,
            9_000,
        );
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(shed, 0);
        assert_eq!(v["payload"]["frames"].as_array().unwrap().len(), 1);
        assert_eq!(v["payload"]["dropped_since"], 3);
    }


    fn forward16(bytes: [u8; 2], direction: FrameDirection) -> SnifferRecord {
        SnifferRecord {
            at_ms: 1_000,
            direction,
            kind: ObservedRawFrameKind::Forward16,
            bytes: [bytes[0], bytes[1], 0],
            adapter_id: 0,
            attempt: 0,
        }
    }

    fn first_frame(json_text: &str) -> Value {
        let v: Value = serde_json::from_str(json_text).unwrap();
        v["payload"]["frames"][0].clone()
    }

    #[test]
    fn a_batch_names_its_channel_and_carries_the_drop_count() {
        let mut state = SnifferDecoderState::default();
        let (frame, _shed) = sniffer_batch_frame(
            &mut state,
            &[forward16([0x00, 0x78], FrameDirection::Tx)],
            3,
            9_000,
        );
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["type"], "SnifferBatch");
        assert_eq!(v["channel"], "sniffer");
        assert_eq!(v["ts_ms"], 9_000);
        assert_eq!(v["payload"]["dropped_since"], 3);
        assert_eq!(v["payload"]["frames"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_dapc_record_carries_hex_direction_and_decoded_name() {
        let mut state = SnifferDecoderState::default();
        let (frame, _shed) = sniffer_batch_frame(
            &mut state,
            &[forward16([0x00, 0x78], FrameDirection::Tx)],
            0,
            0,
        );
        let f = first_frame(&frame);
        assert_eq!(f["dir"], "tx");
        assert_eq!(f["width"], "forward16");
        assert_eq!(f["hex"], "00 78");
        assert_eq!(f["name"], "DAPC");
        assert_eq!(f["detail"], "level 120");
        assert_eq!(f["target"]["kind"], "short");
        assert_eq!(f["target"]["id"], 0);
        assert_eq!(f["is_query"], false);
    }

    #[test]
    fn hex_shows_only_the_bytes_the_width_actually_used() {
        let mut state = SnifferDecoderState::default();
        let reply = SnifferRecord {
            at_ms: 0,
            direction: FrameDirection::Reply,
            kind: ObservedRawFrameKind::Backward8,
            bytes: [0xFE, 0, 0],
            adapter_id: 0,
            attempt: 0,
        };
        let f = first_frame(&sniffer_batch_frame(&mut state, &[reply], 0, 0).0);
        assert_eq!(f["hex"], "FE");
        assert_eq!(f["dir"], "reply");
        assert_eq!(f["name"], "REPLY");
    }

    #[test]
    fn an_enable_device_type_prelude_labels_the_next_extended_frame() {
        let mut state = SnifferDecoderState::default();
        let (frame, _shed) = sniffer_batch_frame(
            &mut state,
            &[
                forward16([0xC1, 0x08], FrameDirection::Tx),
                forward16([0x01, 0xF8], FrameDirection::Tx),
            ],
            0,
            0,
        );
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["payload"]["frames"][0]["name"], "ENABLE DEVICE TYPE");
        assert_eq!(
            v["payload"]["frames"][1]["name"],
            "DT8 QUERY COLOUR STATUS"
        );
    }

    #[test]
    fn the_prelude_is_one_shot_exactly_as_the_gear_treats_it() {
        let mut state = SnifferDecoderState::default();
        let (frame, _shed) = sniffer_batch_frame(
            &mut state,
            &[
                forward16([0xC1, 0x08], FrameDirection::Tx),
                forward16([0x01, 0xF8], FrameDirection::Tx),
                forward16([0x01, 0xF8], FrameDirection::Tx),
            ],
            0,
            0,
        );
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(
            v["payload"]["frames"][1]["name"],
            "DT8 QUERY COLOUR STATUS"
        );
        assert_eq!(v["payload"]["frames"][2]["name"], "EXTENDED 0xF8");
    }

    #[test]
    fn a_foreign_prelude_does_not_arm_our_own_next_frame() {
        let mut state = SnifferDecoderState::default();
        let (frame, _shed) = sniffer_batch_frame(
            &mut state,
            &[
                forward16([0xC1, 0x08], FrameDirection::ForeignRx),
                forward16([0x01, 0xF8], FrameDirection::Tx),
            ],
            0,
            0,
        );
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["payload"]["frames"][1]["name"], "EXTENDED 0xF8");
    }

    #[test]
    fn a_reply_between_a_foreign_prelude_and_its_extended_frame_does_not_disarm_it() {
        let mut state = SnifferDecoderState::default();
        let reply = SnifferRecord {
            at_ms: 0,
            direction: FrameDirection::Reply,
            kind: ObservedRawFrameKind::Backward8,
            bytes: [0xFE, 0, 0],
            adapter_id: 0,
            attempt: 0,
        };
        let (frame, _shed) = sniffer_batch_frame(
            &mut state,
            &[
                forward16([0xC1, 0x08], FrameDirection::ForeignRx),
                reply,
                forward16([0x01, 0xF8], FrameDirection::ForeignRx),
            ],
            0,
            0,
        );
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["payload"]["frames"][1]["name"], "REPLY");
        assert_eq!(
            v["payload"]["frames"][2]["name"],
            "DT8 QUERY COLOUR STATUS"
        );
    }

    #[test]
    fn a_query_frame_is_flagged_so_a_missing_reply_is_visible() {
        let mut state = SnifferDecoderState::default();
        let f = first_frame(
            &sniffer_batch_frame(
                &mut state,
                &[forward16([0x01, 0x90], FrameDirection::Tx)],
                0,
                0,
            )
            .0,
        );
        assert_eq!(f["name"], "QUERY STATUS");
        assert_eq!(f["is_query"], true);
    }

    #[test]
    fn retries_are_visible_as_retries() {
        let mut state = SnifferDecoderState::default();
        let mut record = forward16([0x01, 0x90], FrameDirection::Tx);
        record.attempt = 2;
        let f = first_frame(&sniffer_batch_frame(&mut state, &[record], 0, 0).0);
        assert_eq!(f["attempt"], 2);
    }
}
