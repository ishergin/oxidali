use serde_json::{Map, Value};

use crate::ws::protocol::{event_frame, Channel};

pub(crate) const MAX_BATCH_PAYLOAD_BYTES: usize = 2048;

pub(crate) fn batch_frame(
    event_type: &'static str,
    channel: Channel,
    items_field: &'static str,
    mut items: Vec<Value>,
    dropped_since: u32,
    ts_ms: u64,
) -> (String, u32) {
    let shed = shed_boundary(&items);
    items.drain(..shed);
    let mut payload = Map::new();
    payload.insert(
        "dropped_since".to_owned(),
        Value::from(dropped_since.saturating_add(shed as u32)),
    );
    payload.insert(items_field.to_owned(), Value::Array(items));
    let frame = event_frame(event_type, channel, ts_ms, Value::Object(payload));
    (frame, shed as u32)
}

fn shed_boundary(items: &[Value]) -> usize {
    let mut kept_bytes = 0usize;
    for (index, item) in items.iter().enumerate().rev() {
        kept_bytes += serialized_len(item) + 1;
        if kept_bytes > MAX_BATCH_PAYLOAD_BYTES {
            return index + 1;
        }
    }
    0
}

pub(crate) fn fitting_prefix(items: &[Value]) -> usize {
    let mut kept_bytes = 0usize;
    for (index, item) in items.iter().enumerate() {
        kept_bytes += serialized_len(item) + 1;
        if kept_bytes > MAX_BATCH_PAYLOAD_BYTES {
            return index;
        }
    }
    items.len()
}

fn serialized_len(value: &Value) -> usize {
    struct CountingSink(usize);
    impl std::io::Write for CountingSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(buf.len());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut sink = CountingSink(0);
    match serde_json::to_writer(&mut sink, value) {
        Ok(()) => sink.0,
        Err(_) => usize::MAX,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{batch_frame, MAX_BATCH_PAYLOAD_BYTES};
    use crate::ws::protocol::Channel;

    #[test]
    fn a_batch_holds_the_budget_whatever_the_channel_calls_its_items() {
        let items: Vec<_> = (0..400)
            .map(|i| json!({ "seq": i, "text": "x".repeat(40) }))
            .collect();
        let (frame, shed) = batch_frame("LogBatch", Channel::Diagnostics, "lines", items, 7, 12);

        assert!(shed > 0, "400 fat items must not fit {MAX_BATCH_PAYLOAD_BYTES} B");
        let parsed: serde_json::Value = serde_json::from_str(&frame).expect("frame is JSON");
        let payload = &parsed["payload"];
        assert_eq!(payload["dropped_since"], json!(7 + shed));
        let kept = payload["lines"].as_array().expect("lines array");
        assert_eq!(kept.last().expect("newest kept")["seq"], json!(399));
        assert!(
            serde_json::to_string(payload).expect("payload").len() <= MAX_BATCH_PAYLOAD_BYTES + 64,
            "payload must stay inside the budget plus its own envelope keys"
        );
    }
}
