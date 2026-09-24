use dali2rust_platform::logs::LogLine;
use serde_json::{json, Value};

use crate::ws::batch::batch_frame;
use crate::ws::protocol::Channel;

pub fn log_batch_frame(lines: &[LogLine], dropped_since: u32, ts_ms: u64) -> (String, u32) {
    let items: Vec<Value> = lines.iter().map(line_value).collect();
    batch_frame("LogBatch", Channel::Logs, "lines", items, dropped_since, ts_ms)
}

pub fn log_batch_prefix(lines: &[LogLine]) -> usize {
    let items: Vec<Value> = lines.iter().map(line_value).collect();
    crate::ws::batch::fitting_prefix(&items)
}

fn line_value(line: &LogLine) -> Value {
    json!({
        "ts_ms": line.at_ms,
        "seq": line.seq,
        "level": line.level.as_str(),
        "target": line.target_str(),
        "text": line.text_str(),
    })
}

#[cfg(test)]
mod tests {
    use dali2rust_platform::logs::{LogLevel, LogLine};

    use super::log_batch_frame;

    fn line(seq: u32, level: LogLevel, text: &str) -> LogLine {
        LogLine::new(1_700_000_000_000, seq, level, b"dali2rust::x", text.as_bytes())
    }

    #[test]
    fn a_batch_names_its_channel_and_carries_every_field_the_screen_reads() {
        let lines = [line(7, LogLevel::Warn, "bus acquire timeout")];
        let (frame, shed) = log_batch_frame(&lines, 3, 42);
        assert_eq!(shed, 0);

        let parsed: serde_json::Value = serde_json::from_str(&frame).expect("frame is JSON");
        assert_eq!(parsed["channel"], "logs");
        assert_eq!(parsed["type"], "LogBatch");
        assert_eq!(parsed["ts_ms"], 42);
        assert_eq!(parsed["payload"]["dropped_since"], 3);

        let line = &parsed["payload"]["lines"][0];
        assert_eq!(line["seq"], 7);
        assert_eq!(line["level"], "warn");
        assert_eq!(line["target"], "dali2rust::x");
        assert_eq!(line["text"], "bus acquire timeout");
    }

    #[test]
    fn an_oversized_run_is_shed_oldest_first_and_added_to_the_drop_count() {
        let lines: Vec<_> = (0..200)
            .map(|seq| line(seq, LogLevel::Info, &"x".repeat(100)))
            .collect();
        let (frame, shed) = log_batch_frame(&lines, 1, 0);
        assert!(shed > 0, "200 long lines do not fit one frame");

        let parsed: serde_json::Value = serde_json::from_str(&frame).expect("frame is JSON");
        assert_eq!(parsed["payload"]["dropped_since"], 1 + shed);
        let kept = parsed["payload"]["lines"].as_array().expect("lines");
        assert_eq!(kept.last().expect("newest kept")["seq"], 199);
    }
}
